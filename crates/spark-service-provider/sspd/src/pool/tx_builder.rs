use bitcoin::{
    Address, Amount, Sequence, Transaction, TxIn, TxOut, Witness,
    absolute::LockTime,
    hashes::Hash,
    key::TapTweak,
    secp256k1::{Keypair, Message, Secp256k1, SecretKey},
    sighash::{Prevouts, SighashCache},
    transaction::Version,
};
use thiserror::Error;

use crate::fees;
use crate::wallet::onchain::Utxo;

#[derive(Debug, Error)]
pub enum TxBuildError {
    #[error("insufficient funds: need {need}, have {have}")]
    InsufficientFunds { need: u64, have: u64 },

    #[error("signing error: {0}")]
    Signing(String),

    #[error("sighash error: {0}")]
    Sighash(String),
}

pub struct DepositOutput {
    pub address: Address,
    pub amount: u64,
}

/// Assumes P2TR key-path inputs and P2TR outputs.
pub fn funding_tx_weight_wu(inputs: usize, outputs: usize) -> u64 {
    fees::TX_OVERHEAD_WU
        .saturating_add((inputs as u64).saturating_mul(fees::P2TR_INPUT_WU))
        .saturating_add((outputs as u64).saturating_mul(fees::P2TR_OUTPUT_WU))
}

/// Always includes dust for a change output: a child spends the change to raise the
/// funding transaction's fee rate.
pub fn funding_need(
    destinations: &[DepositOutput],
    inputs: usize,
    fee_rate_sat_per_kw: u64,
) -> u64 {
    let total: u64 = destinations.iter().map(|d| d.amount).sum();
    total
        .saturating_add(fees::fee_sats(
            fee_rate_sat_per_kw,
            funding_tx_weight_wu(inputs, destinations.len().saturating_add(1)),
        ))
        .saturating_add(fees::P2TR_DUST_SATS)
}

pub fn bump_tx_weight_wu(inputs: usize) -> u64 {
    funding_tx_weight_wu(inputs, 1)
}

#[derive(Debug, Clone, Copy)]
pub struct ReplacedChild {
    pub fee_sats: u64,
    pub weight_wu: u64,
}

/// The fee a child with `inputs` inputs pays so it and its parent together pay at least
/// `fee_rate_sat_per_kw`. A child replacing `replaces` also pays at least that child's
/// fee plus the relay fee for its own weight, at a higher fee rate, as a node requires.
pub fn bump_fee(
    fee_rate_sat_per_kw: u64,
    parent_weight_wu: u64,
    parent_fee_sats: u64,
    inputs: usize,
    replaces: Option<ReplacedChild>,
) -> u64 {
    let weight = bump_tx_weight_wu(inputs);
    let relay = fees::fee_sats(fees::MIN_FEE_RATE_SAT_PER_KW, weight);
    let package = fees::fee_sats(fee_rate_sat_per_kw, parent_weight_wu.saturating_add(weight))
        .saturating_sub(parent_fee_sats)
        .max(relay);
    replaces.map_or(package, |earlier| {
        package
            .max(earlier.fee_sats.saturating_add(relay))
            .max(fee_above_rate_of(earlier, weight))
    })
}

/// The least fee for `weight_wu` whose rate a node counts as higher than
/// `earlier`'s. A node compares whole sats per 1000 virtual bytes.
fn fee_above_rate_of(earlier: ReplacedChild, weight_wu: u64) -> u64 {
    let vbytes = |weight_wu: u64| weight_wu.div_ceil(4);
    let earlier_rate = earlier
        .fee_sats
        .saturating_mul(1000)
        .checked_div(vbytes(earlier.weight_wu))
        .unwrap_or(u64::MAX);
    earlier_rate
        .saturating_add(1)
        .saturating_mul(vbytes(weight_wu))
        .div_ceil(1000)
}

pub fn build_bump_tx(
    change: &Utxo,
    extra: &[Utxo],
    destination: &Address,
    fee_sats: u64,
    signing_key_fn: &dyn Fn(&Address) -> Result<SecretKey, TxBuildError>,
) -> Result<Transaction, TxBuildError> {
    let inputs: Vec<Utxo> = std::iter::once(change.clone())
        .chain(extra.iter().cloned())
        .collect();
    let input_total: u64 = inputs.iter().map(|u| u.value).sum();
    let need = fee_sats.saturating_add(fees::P2TR_DUST_SATS);
    if input_total < need {
        return Err(TxBuildError::InsufficientFunds {
            need,
            have: input_total,
        });
    }
    let mut tx = Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: inputs
            .iter()
            .map(|utxo| TxIn {
                previous_output: utxo.outpoint,
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                ..Default::default()
            })
            .collect(),
        output: vec![TxOut {
            value: Amount::from_sat(input_total.saturating_sub(fee_sats)),
            script_pubkey: destination.script_pubkey(),
        }],
    };
    sign_taproot_inputs(&mut tx, &inputs, signing_key_fn)?;
    Ok(tx)
}

pub fn build_funding_tx(
    inputs: &[Utxo],
    destinations: &[DepositOutput],
    change_address: &Address,
    fee_rate_sat_per_kw: u64,
    signing_key_fn: &dyn Fn(&Address) -> Result<SecretKey, TxBuildError>,
) -> Result<Transaction, TxBuildError> {
    let input_total: u64 = inputs.iter().map(|u| u.value).sum();
    let need = funding_need(destinations, inputs.len(), fee_rate_sat_per_kw);
    if input_total < need {
        return Err(TxBuildError::InsufficientFunds {
            need,
            have: input_total,
        });
    }
    // `need` includes the change output's dust.
    let change = input_total
        .saturating_sub(need)
        .saturating_add(fees::P2TR_DUST_SATS);

    let mut outputs: Vec<TxOut> = destinations
        .iter()
        .map(|dest| TxOut {
            value: Amount::from_sat(dest.amount),
            script_pubkey: dest.address.script_pubkey(),
        })
        .collect();
    outputs.push(TxOut {
        value: Amount::from_sat(change),
        script_pubkey: change_address.script_pubkey(),
    });

    let mut tx = Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: inputs
            .iter()
            .map(|utxo| TxIn {
                previous_output: utxo.outpoint,
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                ..Default::default()
            })
            .collect(),
        output: outputs,
    };

    sign_taproot_inputs(&mut tx, inputs, signing_key_fn)?;
    Ok(tx)
}

fn sign_taproot_inputs(
    tx: &mut Transaction,
    selected: &[Utxo],
    signing_key_fn: &dyn Fn(&Address) -> Result<SecretKey, TxBuildError>,
) -> Result<(), TxBuildError> {
    let secp = Secp256k1::new();
    let prevouts: Vec<TxOut> = selected
        .iter()
        .map(|utxo| TxOut {
            value: Amount::from_sat(utxo.value),
            script_pubkey: utxo.address.script_pubkey(),
        })
        .collect();

    let mut signatures = Vec::with_capacity(selected.len());
    {
        let mut sighash_cache = SighashCache::new(&*tx);
        for (i, utxo) in selected.iter().enumerate() {
            let secret_key = signing_key_fn(&utxo.address)?;
            let keypair = Keypair::from_secret_key(&secp, &secret_key);
            let tweaked = keypair.tap_tweak(&secp, None);

            let sighash = sighash_cache
                .taproot_key_spend_signature_hash(
                    i,
                    &Prevouts::All(&prevouts),
                    bitcoin::sighash::TapSighashType::Default,
                )
                .map_err(|e| TxBuildError::Sighash(e.to_string()))?;

            let msg = Message::from_digest(*sighash.as_byte_array());
            let sig = secp.sign_schnorr_no_aux_rand(&msg, &tweaked.to_keypair());
            signatures.push(sig);
        }
    }

    for (input, sig) in tx.input.iter_mut().zip(&signatures) {
        input.witness = Witness::from_slice(&[sig.as_ref()]);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::{Network, OutPoint, Txid, hashes::Hash, key::Secp256k1};

    const ONE_SAT_PER_VB: u64 = 250;

    fn test_utxo(value: u64, index: u8) -> Utxo {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[index.saturating_add(1); 32]).unwrap();
        let pk = sk.public_key(&secp);
        let addr = Address::p2tr(&secp, pk.x_only_public_key().0, None, Network::Regtest);
        Utxo {
            outpoint: OutPoint {
                txid: Txid::from_byte_array([index; 32]),
                vout: 0,
            },
            value,
            block_height: 100,
            address: addr,
        }
    }

    fn key_fn(
        secp: &Secp256k1<bitcoin::secp256k1::All>,
    ) -> impl Fn(&Address) -> Result<SecretKey, TxBuildError> + '_ {
        move |addr: &Address| {
            for i in 1..=10u8 {
                let sk = SecretKey::from_slice(&[i; 32]).unwrap();
                let a = Address::p2tr(
                    secp,
                    sk.public_key(secp).x_only_public_key().0,
                    None,
                    Network::Regtest,
                );
                if a == *addr {
                    return Ok(sk);
                }
            }
            Err(TxBuildError::Signing("unknown address".into()))
        }
    }

    fn make_addr(secp: &Secp256k1<bitcoin::secp256k1::All>, byte: u8) -> Address {
        let sk = SecretKey::from_slice(&[byte; 32]).unwrap();
        Address::p2tr(
            secp,
            sk.public_key(secp).x_only_public_key().0,
            None,
            Network::Regtest,
        )
    }

    #[test]
    fn a_funding_tx_pays_each_destination_and_the_change() {
        let secp = Secp256k1::new();
        let inputs = vec![test_utxo(10_000, 1)];
        let destinations = vec![DepositOutput {
            address: make_addr(&secp, 0xAA),
            amount: 5000,
        }];
        let change_addr = make_addr(&secp, 0xCC);

        let tx = build_funding_tx(
            &inputs,
            &destinations,
            &change_addr,
            ONE_SAT_PER_VB,
            &key_fn(&secp),
        )
        .unwrap();

        assert_eq!(tx.output[0].value.to_sat(), 5000);
        let fee = 10_000 - tx.output.iter().map(|o| o.value.to_sat()).sum::<u64>();
        assert_eq!(
            fee,
            fees::fee_sats(ONE_SAT_PER_VB, funding_tx_weight_wu(1, 2))
        );
    }

    #[test]
    fn a_funding_tx_keeps_a_change_output_when_the_coins_only_just_cover_it() {
        let secp = Secp256k1::new();
        let destinations = vec![DepositOutput {
            address: make_addr(&secp, 0xAA),
            amount: 5000,
        }];
        let need = funding_need(&destinations, 1, ONE_SAT_PER_VB);
        let tx = build_funding_tx(
            &[test_utxo(need, 1)],
            &destinations,
            &make_addr(&secp, 0xCC),
            ONE_SAT_PER_VB,
            &key_fn(&secp),
        )
        .unwrap();

        assert_eq!(tx.output.len(), 2);
        assert_eq!(tx.output[1].value.to_sat(), fees::P2TR_DUST_SATS);
    }

    #[test]
    fn a_bump_pays_for_its_parent_to_reach_the_rate() {
        let rate = 10 * ONE_SAT_PER_VB;
        let parent_weight = funding_tx_weight_wu(1, 2);
        let parent_fee = fees::fee_sats(ONE_SAT_PER_VB, parent_weight);

        let fee = bump_fee(rate, parent_weight, parent_fee, 1, None);

        assert_eq!(
            parent_fee + fee,
            fees::fee_sats(rate, parent_weight + bump_tx_weight_wu(1))
        );
    }

    #[test]
    fn a_bump_replacing_another_pays_more_than_it() {
        let parent_weight = funding_tx_weight_wu(1, 2);
        let parent_fee = fees::fee_sats(ONE_SAT_PER_VB, parent_weight);
        let earlier = ReplacedChild {
            fee_sats: bump_fee(10 * ONE_SAT_PER_VB, parent_weight, parent_fee, 1, None),
            weight_wu: bump_tx_weight_wu(1),
        };

        let fee = bump_fee(
            10 * ONE_SAT_PER_VB,
            parent_weight,
            parent_fee,
            1,
            Some(earlier),
        );

        assert!(
            fee >= earlier.fee_sats
                + fees::fee_sats(fees::MIN_FEE_RATE_SAT_PER_KW, bump_tx_weight_wu(1))
        );
    }

    #[test]
    fn a_heavier_bump_replacing_another_pays_a_higher_rate_than_it() {
        let parent_weight = funding_tx_weight_wu(1, 2);
        let parent_fee = fees::fee_sats(ONE_SAT_PER_VB, parent_weight);
        let earlier = ReplacedChild {
            fee_sats: bump_fee(200 * ONE_SAT_PER_VB, parent_weight, parent_fee, 1, None),
            weight_wu: bump_tx_weight_wu(1),
        };
        let weight = bump_tx_weight_wu(5);

        let fee = bump_fee(
            20 * ONE_SAT_PER_VB,
            parent_weight,
            parent_fee,
            5,
            Some(earlier),
        );

        let rate = |fee: u64, weight: u64| fee * 1000 / weight.div_ceil(4);
        assert!(rate(fee, weight) > rate(earlier.fee_sats, earlier.weight_wu));
    }

    #[test]
    fn a_bump_spends_the_change_and_extra_coins_to_one_output() {
        let secp = Secp256k1::new();
        let change = test_utxo(2_000, 1);
        let extra = [test_utxo(3_000, 2)];

        let tx = build_bump_tx(
            &change,
            &extra,
            &make_addr(&secp, 0xDD),
            700,
            &key_fn(&secp),
        )
        .unwrap();

        assert_eq!(tx.input.len(), 2);
        assert_eq!(tx.input[0].previous_output, change.outpoint);
        assert_eq!(tx.output.len(), 1);
        assert_eq!(tx.output[0].value.to_sat(), 4_300);
    }

    #[test]
    fn test_insufficient_funds() {
        let secp = Secp256k1::new();
        let inputs = vec![test_utxo(1000, 1)];
        let destinations = vec![DepositOutput {
            address: make_addr(&secp, 0xAA),
            amount: 5000,
        }];
        let change_addr = make_addr(&secp, 0xCC);
        let result = build_funding_tx(
            &inputs,
            &destinations,
            &change_addr,
            ONE_SAT_PER_VB,
            &key_fn(&secp),
        );
        assert!(matches!(
            result,
            Err(TxBuildError::InsufficientFunds { .. })
        ));
    }
}
