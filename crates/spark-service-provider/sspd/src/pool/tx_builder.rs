use std::collections::HashMap;
use std::hash::BuildHasher;

use bitcoin::{
    Address, Amount, FeeRate, Sequence, Transaction, TxIn, TxOut, Weight, Witness,
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

pub fn key_for<S: BuildHasher>(
    keys: &HashMap<Address, SecretKey, S>,
    address: &Address,
) -> Result<SecretKey, TxBuildError> {
    keys.get(address)
        .copied()
        .ok_or_else(|| TxBuildError::Signing(format!("no key for {address}")))
}

pub struct DepositOutput {
    pub address: Address,
    pub amount: u64,
}

/// Always includes dust for a change output: a child spends the change to raise the
/// funding transaction's fee rate.
pub fn funding_need(destinations: &[DepositOutput], inputs: usize, fee_rate: FeeRate) -> u64 {
    let total: u64 = destinations.iter().map(|d| d.amount).sum();
    total
        .saturating_add(fees::fee_sats(
            fee_rate,
            fees::p2tr_tx_weight(inputs, destinations.len().saturating_add(1)),
        ))
        .saturating_add(fees::P2TR_DUST_SATS)
}

pub fn bump_tx_weight(inputs: usize) -> Weight {
    fees::p2tr_tx_weight(inputs, 1)
}

/// With default settings, Bitcoin Core before version 29.1 accepts a replacement
/// only if its fee exceeds the replaced transaction's fee by at least 1 sat per
/// virtual byte of its size. So for a small rise of the fee rate, the SSP would pay
/// more than the rate it wants.
pub const MIN_BUMP: FeeRate = FeeRate::from_sat_per_vb_u32(1);

pub fn bump_due(paid_rate: FeeRate, wanted_rate: FeeRate) -> bool {
    let least = paid_rate
        .to_sat_per_kwu()
        .saturating_add(MIN_BUMP.to_sat_per_kwu());
    wanted_rate.to_sat_per_kwu() >= least
}

#[derive(Debug, Clone, Copy)]
pub struct ReplacedChild {
    pub fee_sats: u64,
    pub weight: Weight,
}

/// The fee for a CPFP child of `weight` at which it and its parent together have a fee rate
/// of at least `fee_rate`. The replacement for `replaces` has a fee of at least that child's
/// fee plus the relay fee for its own weight, at a higher fee rate, as a node requires.
pub fn bump_fee(
    fee_rate: FeeRate,
    parent_weight: Weight,
    parent_fee_sats: u64,
    weight: Weight,
    replaces: Option<ReplacedChild>,
) -> u64 {
    let relay = fees::fee_sats(fees::MIN_FEE_RATE, weight);
    let package = fees::fee_sats(
        fee_rate,
        parent_weight.checked_add(weight).unwrap_or(Weight::MAX),
    )
    .saturating_sub(parent_fee_sats)
    .max(relay);
    replaces.map_or(package, |earlier| {
        package
            .max(earlier.fee_sats.saturating_add(relay))
            .max(fee_above_rate_of(earlier, weight))
    })
}

/// The least fee for `weight` at a fee rate higher than that of `earlier`, in whole
/// sats per 1,000 virtual bytes. Older versions of Bitcoin Core compare fee rates in
/// those units.
fn fee_above_rate_of(earlier: ReplacedChild, weight: Weight) -> u64 {
    let earlier_rate = earlier
        .fee_sats
        .saturating_mul(1000)
        .checked_div(earlier.weight.to_vbytes_ceil())
        .unwrap_or(u64::MAX);
    earlier_rate
        .saturating_add(1)
        .saturating_mul(weight.to_vbytes_ceil())
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
    fee_rate: FeeRate,
    signing_key_fn: &dyn Fn(&Address) -> Result<SecretKey, TxBuildError>,
) -> Result<Transaction, TxBuildError> {
    let input_total: u64 = inputs.iter().map(|u| u.value).sum();
    let need = funding_need(destinations, inputs.len(), fee_rate);
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

    const ONE_SAT_PER_VB: FeeRate = FeeRate::from_sat_per_vb_u32(1);

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
            fees::fee_sats(ONE_SAT_PER_VB, fees::p2tr_tx_weight(1, 2))
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
        let rate = FeeRate::from_sat_per_vb_u32(10);
        let parent_weight = fees::p2tr_tx_weight(1, 2);
        let parent_fee = fees::fee_sats(ONE_SAT_PER_VB, parent_weight);

        let fee = bump_fee(rate, parent_weight, parent_fee, bump_tx_weight(1), None);

        assert_eq!(
            parent_fee + fee,
            fees::fee_sats(rate, parent_weight.checked_add(bump_tx_weight(1)).unwrap())
        );
    }

    #[test]
    fn a_bump_is_due_once_the_rate_wanted_is_a_sat_per_vbyte_higher() {
        let paid = FeeRate::from_sat_per_vb_u32(10);
        let a_sat_per_vbyte_more = FeeRate::from_sat_per_vb_u32(11);
        let just_short = FeeRate::from_sat_per_kwu(a_sat_per_vbyte_more.to_sat_per_kwu() - 1);

        assert!(!bump_due(paid, just_short));
        assert!(bump_due(paid, a_sat_per_vbyte_more));
    }

    #[test]
    fn a_bump_replacing_another_pays_more_than_it() {
        let parent_weight = fees::p2tr_tx_weight(1, 2);
        let parent_fee = fees::fee_sats(ONE_SAT_PER_VB, parent_weight);
        let earlier = ReplacedChild {
            fee_sats: bump_fee(
                FeeRate::from_sat_per_vb_u32(10),
                parent_weight,
                parent_fee,
                bump_tx_weight(1),
                None,
            ),
            weight: bump_tx_weight(1),
        };

        let fee = bump_fee(
            FeeRate::from_sat_per_vb_u32(10),
            parent_weight,
            parent_fee,
            bump_tx_weight(1),
            Some(earlier),
        );

        assert!(fee >= earlier.fee_sats + fees::fee_sats(fees::MIN_FEE_RATE, bump_tx_weight(1)));
    }

    #[test]
    fn a_heavier_bump_replacing_another_pays_a_higher_rate_than_it() {
        let parent_weight = fees::p2tr_tx_weight(1, 2);
        let parent_fee = fees::fee_sats(ONE_SAT_PER_VB, parent_weight);
        let earlier = ReplacedChild {
            fee_sats: bump_fee(
                FeeRate::from_sat_per_vb_u32(200),
                parent_weight,
                parent_fee,
                bump_tx_weight(1),
                None,
            ),
            weight: bump_tx_weight(1),
        };
        let weight = bump_tx_weight(5);

        let fee = bump_fee(
            FeeRate::from_sat_per_vb_u32(20),
            parent_weight,
            parent_fee,
            weight,
            Some(earlier),
        );

        let rate = |fee: u64, weight: Weight| fee * 1000 / weight.to_vbytes_ceil();
        assert!(rate(fee, weight) > rate(earlier.fee_sats, earlier.weight));
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
