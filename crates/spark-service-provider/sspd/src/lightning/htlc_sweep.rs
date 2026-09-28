//! A user can take a leaf of a paid send on chain with a refund signed before the
//! send, which pays an HTLC: the SSP's with the preimage, and the user's once a
//! timeout passes. The SSP sweeps such an output with the preimage as it confirms.

use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use bitcoin::hashes::Hash;
use bitcoin::key::Secp256k1;
use bitcoin::secp256k1::{PublicKey, XOnlyPublicKey};
use bitcoin::sighash::{Prevouts, SighashCache, TapSighashType};
use bitcoin::taproot::{ControlBlock, LeafVersion, TapLeafHash, TaprootBuilder};
use bitcoin::{
    Address, Amount, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness, absolute::LockTime,
    transaction::Version,
};
use spark::signer::Signer;
use spark::utils::htlc_transactions::{create_hash_lock_script, create_htlc_taproot_address};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::chain::{BroadcastError, ChainClient, ChainRepository};
use crate::fees::{self, FeeRateSource};
use crate::wakeup::Wakeup;
use crate::wallet::OnchainWallet;

use super::repository::{HtlcOutput, LightningStore};
use super::send::EXPECTED_HTLC_TIMELOCK_BLOCKS;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Blocks wake the sweeper.
const SWEEP_BACKUP_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// BIP-341's point with no known discrete logarithm, which spark uses as the HTLC
/// output's internal key.
const UNSPENDABLE_INTERNAL_KEY: &str =
    "50929b74c1a04954b78b4b6035e97a5e078a5a0f28ec96d547bfee9ace803ac0";

/// Outpoint, empty scriptSig and sequence at 4 WU per byte, plus a witness of the
/// element count, signature, preimage, hash lock script and control block.
const HASH_LOCK_INPUT_WU: u64 = 41 * 4 + 1 + 65 + 33 + 70 + 66;

pub struct HtlcSweepDeps<R: ChainRepository> {
    pub store: Arc<dyn LightningStore>,
    pub chain_client: Arc<dyn ChainClient + Send + Sync>,
    pub fee_rates: Arc<dyn FeeRateSource>,
    pub signer: Arc<dyn Signer>,
    pub onchain: Arc<OnchainWallet<R>>,
    pub network: spark::Network,
    pub blocks: Wakeup,
}

pub async fn run_htlc_sweep_loop<R>(deps: HtlcSweepDeps<R>, token: CancellationToken)
where
    R: ChainRepository + Send + Sync + 'static,
{
    loop {
        if let Err(e) = sweep_htlc_outputs(&deps).await {
            error!("HTLC sweep failed: {e}");
        }
        tokio::select! {
            () = token.cancelled() => return,
            () = deps.blocks.waited() => {}
            () = tokio::time::sleep(SWEEP_BACKUP_INTERVAL) => {}
        }
    }
}

/// Rebuilds each sweep on every pass at the current fee rate until its outputs are
/// spent on chain, so a dropped sweep is broadcast again and a sweep a rising fee
/// rate left behind is replaced.
async fn sweep_htlc_outputs<R>(deps: &HtlcSweepDeps<R>) -> Result<(), BoxError>
where
    R: ChainRepository + Send + Sync + 'static,
{
    let mut by_send: BTreeMap<String, Vec<HtlcOutput>> = BTreeMap::new();
    for output in deps.store.unswept_htlc_outputs().await? {
        by_send
            .entry(output.send_id.clone())
            .or_default()
            .push(output);
    }
    for (send_id, outputs) in by_send {
        if let Err(e) = sweep(deps, &outputs).await {
            error!(%send_id, "could not sweep the HTLC outputs of a paid send: {e}");
        }
    }
    Ok(())
}

async fn sweep<R>(deps: &HtlcSweepDeps<R>, outputs: &[HtlcOutput]) -> Result<(), BoxError>
where
    R: ChainRepository + Send + Sync + 'static,
{
    let first = outputs.first().ok_or("no outputs to sweep")?;
    let ssp = spark::signer::derive_identity_public_key(deps.signer.as_ref()).await?;
    let (script_pubkey, hash_lock_script, control_block) =
        hash_lock_spend(first, &ssp, deps.network)?;

    let rate = deps.fee_rates.sat_per_kw().await?;
    let weight = fees::TX_OVERHEAD_WU
        .saturating_add(HASH_LOCK_INPUT_WU.saturating_mul(outputs.len() as u64))
        .saturating_add(fees::P2TR_OUTPUT_WU);
    let total: u64 = outputs.iter().map(|output| output.value_sats).sum();
    let Some(value) = total
        .checked_sub(fees::fee_sats(rate, weight))
        .filter(|value| *value >= fees::P2TR_DUST_SATS)
    else {
        warn!(send_id = %first.send_id, total, rate, "the HTLC outputs of a paid send are not worth sweeping at the current fee rate");
        return Ok(());
    };
    let destination = if let Some(address) = &first.sweep_address {
        Address::from_str(address)?.require_network(deps.network.into())?
    } else {
        let (address, _) = deps.onchain.next_address().await?;
        deps.store
            .set_htlc_sweep_address(&first.send_id, &address.to_string())
            .await?;
        address
    };

    let mut tx = Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: outputs
            .iter()
            .map(|output| TxIn {
                previous_output: output.outpoint,
                script_sig: ScriptBuf::new(),
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness: Witness::new(),
            })
            .collect(),
        output: vec![TxOut {
            value: Amount::from_sat(value),
            script_pubkey: destination.script_pubkey(),
        }],
    };
    let prevouts: Vec<TxOut> = outputs
        .iter()
        .map(|output| TxOut {
            value: Amount::from_sat(output.value_sats),
            script_pubkey: script_pubkey.clone(),
        })
        .collect();
    let leaf_hash = TapLeafHash::from_script(&hash_lock_script, LeafVersion::TapScript);
    let mut witnesses = Vec::with_capacity(outputs.len());
    let mut sighashes = SighashCache::new(&tx);
    for index in 0..outputs.len() {
        let sighash = sighashes.taproot_script_spend_signature_hash(
            index,
            &Prevouts::All(&prevouts),
            leaf_hash,
            TapSighashType::Default,
        )?;
        let signature = deps
            .signer
            .sign_hash_schnorr(&identity_path(), sighash.as_byte_array())
            .await?;
        let mut witness = Witness::new();
        witness.push(signature.serialize());
        witness.push(first.preimage.to_vec());
        witness.push(hash_lock_script.as_bytes());
        witness.push(control_block.serialize());
        witnesses.push(witness);
    }
    for (input, witness) in tx.input.iter_mut().zip(witnesses) {
        input.witness = witness;
    }

    let txid = tx.compute_txid();
    match deps.chain_client.broadcast_tx(tx).await {
        Ok(()) => {
            info!(send_id = %first.send_id, %txid, count = outputs.len(), value, "swept the HTLC outputs of a paid send");
            Ok(())
        }
        Err(BroadcastError::AlreadyKnown) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Refuses to sweep unless the rebuilt output matches the one spark builds.
fn hash_lock_spend(
    output: &HtlcOutput,
    ssp: &PublicKey,
    network: spark::Network,
) -> Result<(ScriptBuf, ScriptBuf, ControlBlock), BoxError> {
    let secp = Secp256k1::verification_only();
    let hash = output.preimage.compute_hash();
    let timeout = Sequence::from_height(EXPECTED_HTLC_TIMELOCK_BLOCKS);
    let script_pubkey = create_htlc_taproot_address(
        &hash,
        ssp,
        timeout,
        &output.user_identity_public_key,
        network,
    )?;
    let hash_lock_script = create_hash_lock_script(&hash, ssp);
    let sequence_lock_script = spark::utils::htlc_transactions::create_sequence_lock_script(
        timeout,
        &output.user_identity_public_key,
    );
    let internal_key = XOnlyPublicKey::from_str(UNSPENDABLE_INTERNAL_KEY)?;
    let spend_info = TaprootBuilder::new()
        .add_leaf(1, hash_lock_script.clone())?
        .add_leaf(1, sequence_lock_script)?
        .finalize(&secp, internal_key)
        .map_err(|_| "could not finalize the HTLC taproot tree")?;
    if ScriptBuf::new_p2tr_tweaked(spend_info.output_key()) != script_pubkey {
        return Err("the rebuilt HTLC output does not match spark's".into());
    }
    let control_block = spend_info
        .control_block(&(hash_lock_script.clone(), LeafVersion::TapScript))
        .ok_or("the hash lock script is not in the HTLC taproot tree")?;
    Ok((script_pubkey, hash_lock_script, control_block))
}

fn identity_path() -> bitcoin::bip32::DerivationPath {
    bitcoin::bip32::DerivationPath::from(vec![bitcoin::bip32::ChildNumber::Hardened { index: 0 }])
}

#[cfg(test)]
mod tests {
    use bitcoin::OutPoint;
    use bitcoin::secp256k1::{Secp256k1, SecretKey};
    use spark::services::Preimage;

    use super::*;

    fn pubkey(byte: u8) -> PublicKey {
        PublicKey::from_secret_key(
            &Secp256k1::new(),
            &SecretKey::from_slice(&[byte; 32]).expect("secret"),
        )
    }

    #[test]
    fn the_rebuilt_htlc_output_matches_spark() {
        let output = HtlcOutput {
            send_id: "s1".to_string(),
            user_identity_public_key: pubkey(2),
            preimage: Preimage::try_from(vec![5u8; 32]).unwrap(),
            sweep_address: None,
            outpoint: OutPoint::null(),
            value_sats: 10_000,
        };
        let (script_pubkey, script, control_block) =
            hash_lock_spend(&output, &pubkey(1), spark::Network::Regtest).unwrap();
        assert!(control_block.verify_taproot_commitment(
            &Secp256k1::verification_only(),
            XOnlyPublicKey::from_slice(&script_pubkey.as_bytes()[2..]).unwrap(),
            &script,
        ));
    }

    #[test]
    fn the_input_weight_matches_its_witness() {
        let script = create_hash_lock_script(
            &Preimage::try_from(vec![5u8; 32]).unwrap().compute_hash(),
            &pubkey(1),
        );
        assert_eq!(script.len(), 69);
        assert_eq!(HASH_LOCK_INPUT_WU, 164 + 1 + 65 + 33 + (1 + 69) + (1 + 65));
    }
}
