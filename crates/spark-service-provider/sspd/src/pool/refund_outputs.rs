//! The refund output of a leaf that the SSP exited unilaterally is for the leaf's
//! signing key, so the on-chain wallet does not have it until the SSP sweeps it
//! there.

use std::collections::HashMap;
use std::sync::Arc;

use bitcoin::hashes::Hash;
use bitcoin::key::Secp256k1;
use bitcoin::sighash::{Prevouts, SighashCache, TapSighashType};
use bitcoin::{
    Address, Amount, FeeRate, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid,
    Witness, absolute::LockTime, transaction::Version,
};
use spark::signer::SparkSigner;
use spark::tree::TreeNodeId;
use tracing::{info, warn};

use crate::chain::spend_check::MAX_OUTPUTS_PER_QUERY;
use crate::chain::{BroadcastError, ChainClient};
use crate::fees::{self, FeeRateSource};
use crate::leaves::LeafSigningKeys;
use crate::postgresql::{ChainRepository, PostgresLeafStore, RefundOutput};
use crate::wallet::OnchainWallet;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// A sweep with this many inputs is well under the largest transaction weight that
/// Bitcoin Core relays.
const INPUTS_PER_SWEEP: usize = 500;

#[derive(Debug, Clone)]
pub struct UnsweptOutput {
    pub output: RefundOutput,
    /// The unconfirmed transaction that has the output as an input.
    pub spender: Option<Txid>,
}

#[derive(Debug, Default)]
pub struct Sweep {
    pub transactions: Vec<Txid>,
    pub outputs: usize,
    /// The value of the transactions' outputs, which are the wallet's.
    pub value_sats: u64,
    pub fee_sats: u64,
}

pub struct RefundOutputs {
    pub leaf_store: Arc<PostgresLeafStore>,
    pub chain_repository: Arc<ChainRepository>,
    pub chain_client: Arc<dyn ChainClient + Send + Sync>,
    pub fee_rates: Arc<dyn FeeRateSource>,
    pub onchain: Arc<OnchainWallet<ChainRepository>>,
    pub signer: Arc<dyn SparkSigner>,
    pub signing_keys: Arc<dyn LeafSigningKeys>,
}

impl RefundOutputs {
    pub async fn unswept(&self) -> Result<Vec<UnsweptOutput>, BoxError> {
        let unspent = self.chain_repository.confirmed_unspent_outputs().await?;
        let outputs = self.leaf_store.refund_outputs(&unspent).await?;
        let outpoints: Vec<OutPoint> = outputs.iter().map(|output| output.outpoint).collect();
        let mut spends = HashMap::new();
        for chunk in outpoints.chunks(MAX_OUTPUTS_PER_QUERY as usize) {
            for spend in self.chain_client.get_spends(chunk).await? {
                spends.insert(spend.outpoint, spend);
            }
        }
        Ok(outputs
            .into_iter()
            .filter_map(|output| match spends.get(&output.outpoint) {
                Some(spend) if spend.block.is_some() => None,
                spend => Some(UnsweptOutput {
                    spender: spend.map(|spend| spend.spender),
                    output,
                }),
            })
            .collect())
    }

    /// Sweeps the unswept outputs into the on-chain wallet. With `replace`, it also
    /// sweeps an output that is an input of an unconfirmed transaction.
    /// Bitcoin Core accepts such a sweep only at a higher fee rate than that
    /// transaction's.
    pub async fn sweep(&self, rate: Option<FeeRate>, replace: bool) -> Result<Sweep, BoxError> {
        let rate = match rate {
            Some(rate) => rate.max(fees::MIN_FEE_RATE),
            None => self.fee_rates.fee_rate().await?,
        };
        let unswept: Vec<UnsweptOutput> = self
            .unswept()
            .await?
            .into_iter()
            .filter(|unswept| {
                (unswept.spender.is_none() || replace) && pays_for_itself(&unswept.output, rate)
            })
            .collect();
        let node_ids: Vec<String> = unswept
            .iter()
            .map(|unswept| unswept.output.node_id.to_string())
            .collect();
        let signing_ids = self.signing_keys.get_signing_leaf_ids(&node_ids).await?;
        let mut inputs = Vec::new();
        for unswept in unswept {
            let output = unswept.output;
            match signing_id(self.signer.as_ref(), &signing_ids, &output).await? {
                Some(id) => inputs.push((output, id)),
                None => {
                    warn!(leaf_id = %output.node_id, outpoint = %output.outpoint, "a refund output is not for the leaf's signing key, so the SSP does not sweep it");
                }
            }
        }

        let mut sweep = Sweep::default();
        for inputs in inputs.chunks(INPUTS_PER_SWEEP) {
            let Some((value_sats, fee_sats)) = swept_value(inputs, rate) else {
                continue;
            };
            let (destination, _) = self.onchain.next_address().await?;
            let mut transaction = sweep_transaction(inputs, &destination, value_sats);
            sign(self.signer.as_ref(), &mut transaction, inputs).await?;
            let txid = transaction.compute_txid();
            match self.chain_client.broadcast_tx(transaction).await {
                Ok(()) | Err(BroadcastError::AlreadyKnown) => {}
                Err(e) => return Err(e.into()),
            }
            info!(%txid, outputs = inputs.len(), value_sats, fee_sats, "broadcast a sweep of refund outputs into the on-chain wallet");
            sweep.transactions.push(txid);
            sweep.outputs = sweep.outputs.saturating_add(inputs.len());
            sweep.value_sats = sweep.value_sats.saturating_add(value_sats);
            sweep.fee_sats = sweep.fee_sats.saturating_add(fee_sats);
        }
        Ok(sweep)
    }
}

async fn signing_id(
    signer: &dyn SparkSigner,
    signing_ids: &HashMap<String, String>,
    output: &RefundOutput,
) -> Result<Option<TreeNodeId>, BoxError> {
    let id = match signing_ids.get(&output.node_id.to_string()) {
        Some(id) => id.parse()?,
        None => output.node_id.clone(),
    };
    let key = signer.get_public_key_for_leaf(&id).await?;
    let script = ScriptBuf::new_p2tr(
        &Secp256k1::verification_only(),
        key.x_only_public_key().0,
        None,
    );
    Ok((script == output.output.script_pubkey).then_some(id))
}

async fn sign(
    signer: &dyn SparkSigner,
    transaction: &mut Transaction,
    inputs: &[(RefundOutput, TreeNodeId)],
) -> Result<(), BoxError> {
    let prevouts: Vec<TxOut> = inputs
        .iter()
        .map(|(output, _)| output.output.clone())
        .collect();
    let mut witnesses = Vec::with_capacity(inputs.len());
    {
        let mut sighashes = SighashCache::new(&*transaction);
        for (index, (_, id)) in inputs.iter().enumerate() {
            let sighash = sighashes.taproot_key_spend_signature_hash(
                index,
                &Prevouts::All(&prevouts),
                TapSighashType::Default,
            )?;
            let signature = signer
                .sign_leaf_refund_spend(id, sighash.as_byte_array())
                .await?;
            witnesses.push(Witness::from_slice(&[signature.as_ref()]));
        }
    }
    for (input, witness) in transaction.input.iter_mut().zip(witnesses) {
        input.witness = witness;
    }
    Ok(())
}

fn pays_for_itself(output: &RefundOutput, rate: FeeRate) -> bool {
    output.output.value.to_sat() > fees::fee_sats(rate, fees::P2TR_INPUT)
}

fn swept_value(inputs: &[(RefundOutput, TreeNodeId)], rate: FeeRate) -> Option<(u64, u64)> {
    let total: u64 = inputs
        .iter()
        .map(|(output, _)| output.output.value.to_sat())
        .sum();
    let fee = fees::fee_sats(rate, fees::p2tr_tx_weight(inputs.len(), 1));
    let value = total
        .checked_sub(fee)
        .filter(|value| *value >= fees::P2TR_DUST_SATS)?;
    Some((value, fee))
}

fn sweep_transaction(
    inputs: &[(RefundOutput, TreeNodeId)],
    destination: &Address,
    value: u64,
) -> Transaction {
    Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: inputs
            .iter()
            .map(|(output, _)| TxIn {
                previous_output: output.outpoint,
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                ..Default::default()
            })
            .collect(),
        output: vec![TxOut {
            value: Amount::from_sat(value),
            script_pubkey: destination.script_pubkey(),
        }],
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::secp256k1::{Message, XOnlyPublicKey, schnorr::Signature};
    use bitcoin::{Network, Txid};
    use spark::signer::{DefaultSigner, SparkSignerAdapter};

    use super::*;

    fn signer() -> SparkSignerAdapter {
        let signer = DefaultSigner::new(&[7; 32], spark::Network::Regtest).unwrap();
        SparkSignerAdapter::new(Arc::new(signer))
    }

    async fn refund_output(
        signer: &dyn SparkSigner,
        leaf: &str,
        paid: &str,
        value: u64,
    ) -> RefundOutput {
        let key = signer
            .get_public_key_for_leaf(&paid.parse().unwrap())
            .await
            .unwrap();
        let script_pubkey = ScriptBuf::new_p2tr(&Secp256k1::new(), key.x_only_public_key().0, None);
        RefundOutput {
            node_id: leaf.parse().unwrap(),
            outpoint: OutPoint::new(Txid::hash(leaf.as_bytes()), 0),
            output: TxOut {
                value: Amount::from_sat(value),
                script_pubkey,
            },
            height: 100,
        }
    }

    fn destination() -> Address {
        let key = XOnlyPublicKey::from_slice(&[2; 32]).unwrap();
        Address::p2tr(&Secp256k1::new(), key, None, Network::Regtest)
    }

    #[tokio::test]
    async fn a_sweep_signs_for_each_output_with_its_leafs_signing_key() {
        let signer = signer();
        let claimed = refund_output(&signer, "claimed", "claimed", 10_000).await;
        let pooled = refund_output(&signer, "pooled", "deposit", 20_000).await;
        let elsewhere = refund_output(&signer, "stray", "another", 30_000).await;

        // The SSP signs for the leaf `pooled` with the key derived from `deposit`,
        // and for every other leaf with the key derived from its own id.
        let ids = HashMap::from([("pooled".to_string(), "deposit".to_string())]);
        let mut inputs = Vec::new();
        for output in [&claimed, &pooled] {
            let id = signing_id(&signer, &ids, output).await.unwrap().unwrap();
            inputs.push((output.clone(), id));
        }
        assert_eq!(inputs[0].1, "claimed".parse().unwrap());
        assert_eq!(inputs[1].1, "deposit".parse().unwrap());
        assert_eq!(signing_id(&signer, &ids, &elsewhere).await.unwrap(), None);

        let rate = fees::MIN_FEE_RATE;
        let (value, fee) = swept_value(&inputs, rate).unwrap();
        let mut transaction = sweep_transaction(&inputs, &destination(), value);
        sign(&signer, &mut transaction, &inputs).await.unwrap();

        assert_eq!(fee, fees::fee_sats(rate, transaction.weight()));
        assert_eq!(value, 30_000 - fee);
        let prevouts: Vec<TxOut> = inputs.iter().map(|(o, _)| o.output.clone()).collect();
        let mut sighashes = SighashCache::new(&transaction);
        for (index, prevout) in prevouts.iter().enumerate() {
            let sighash = sighashes
                .taproot_key_spend_signature_hash(
                    index,
                    &Prevouts::All(&prevouts),
                    TapSighashType::Default,
                )
                .unwrap();
            let key = XOnlyPublicKey::from_slice(&prevout.script_pubkey.as_bytes()[2..]).unwrap();
            let signature = Signature::from_slice(&transaction.input[index].witness[0]).unwrap();
            Secp256k1::verification_only()
                .verify_schnorr(
                    &signature,
                    &Message::from_digest(*sighash.as_byte_array()),
                    &key,
                )
                .unwrap();
        }
    }

    #[tokio::test]
    async fn an_output_is_swept_only_when_it_is_worth_the_fee() {
        let signer = signer();
        let rate = fees::MIN_FEE_RATE;
        let small = refund_output(&signer, "small", "small", 50).await;
        let large = refund_output(&signer, "large", "large", 10_000).await;
        assert!(!pays_for_itself(&small, rate));
        assert!(pays_for_itself(&large, rate));

        // The output is worth more than the fee for its input, but a sweep of it
        // alone would have an output below dust.
        let alone = refund_output(&signer, "alone", "alone", 200).await;
        assert!(pays_for_itself(&alone, rate));
        let inputs = [(alone, "alone".parse().unwrap())];
        assert!(swept_value(&inputs, rate).is_none());
    }
}
