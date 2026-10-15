//! Fee ladders: the same exit transaction pre-signed at increasing fees
//! ("rungs"), so the watchtower can pick the fee the mempool needs at broadcast
//! time. The operators validate every ladder against the bounds below.

use bitcoin::{
    Amount, FeeRate, OutPoint, Script, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
    absolute::LockTime, secp256k1::constants::SCHNORR_SIGNATURE_SIZE, transaction::Version,
};

/// The rung table the operators validate ladders against.
pub const LADDER_RUNG_TABLE_VERSION: u32 = 1;

pub const LADDER_MAX_RUNGS: usize = 24;

/// Each fee must be at least 1.25 times the previous one, compared as
/// `fee[i] * 100 >= fee[i - 1] * 125`.
const MIN_STEP_NUMERATOR: u64 = 125;
const MIN_STEP_DENOMINATOR: u64 = 100;

/// The fees of a ladder spending `value` sats into `script_pubkey`, cheapest
/// first. `vsize` is what each rung pays for: the refund plus the rung for a
/// refund ladder, the rung alone for a direct node ladder. The operators state
/// their bounds in vsize, so the fees are computed on it rather than on weight,
/// which rounds a refund and its rung together.
///
/// The lowest fee is the minimum relay fee. The highest spends the output down
/// to dust, so outbidding the ladder costs more than the output is worth. The
/// step is 1.25x, raised when needed to reach the top within the rung limit.
/// Empty when `value` cannot fund even the lowest fee.
#[must_use]
pub fn ladder_fees(value: u64, vsize: u64, script_pubkey: &Script) -> Vec<u64> {
    let Some(floor) = FeeRate::BROADCAST_MIN.fee_vb(vsize).map(Amount::to_sat) else {
        return Vec::new();
    };
    let Some(ceiling) = value.checked_sub(script_pubkey.minimal_non_dust().to_sat()) else {
        return Vec::new();
    };
    if vsize == 0 || ceiling < floor {
        return Vec::new();
    }

    // The largest fee the ceiling is still a valid step above.
    let max_below_ceiling = ceiling.saturating_mul(MIN_STEP_DENOMINATOR) / MIN_STEP_NUMERATOR;
    if floor > max_below_ceiling {
        return vec![ceiling];
    }

    #[allow(clippy::cast_precision_loss)]
    let auto_step = (ceiling as f64 / floor as f64).powf(1.0 / (LADDER_MAX_RUNGS - 1) as f64);
    let step = auto_step.max(MIN_STEP_NUMERATOR as f64 / MIN_STEP_DENOMINATOR as f64);

    let mut fees = vec![floor];
    let mut prev = floor;
    while fees.len() < LADDER_MAX_RUNGS - 1 {
        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        #[allow(clippy::cast_sign_loss)]
        let scaled = (prev as f64 * step).ceil() as u64;
        let next = scaled.max(min_next_fee(prev));
        if next > max_below_ceiling {
            break;
        }
        fees.push(next);
        prev = next;
    }
    fees.push(ceiling);
    fees
}

/// The smallest fee that is a valid step above `prev`.
fn min_next_fee(prev: u64) -> u64 {
    prev.saturating_mul(MIN_STEP_NUMERATOR)
        .div_ceil(MIN_STEP_DENOMINATOR)
        .max(prev.saturating_add(1))
}

/// A refund ladder rung: spends the refund's output and pays `output_value` back
/// to the same script. The rest of the refund's value is the fee for both.
#[must_use]
pub fn refund_rung_tx(refund_tx: &Transaction, output_value: u64) -> Transaction {
    let script_pubkey = refund_tx
        .output
        .first()
        .map(|out| out.script_pubkey.clone())
        .unwrap_or_default();
    Transaction {
        version: Version::non_standard(3),
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint {
                txid: refund_tx.compute_txid(),
                vout: 0,
            },
            // Leaves the relative timelock off, so the rung can go out as soon
            // as its refund does.
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            ..Default::default()
        }],
        output: vec![TxOut {
            value: Amount::from_sat(output_value),
            script_pubkey,
        }],
    }
}

/// A direct node ladder rung: the direct node tx's input and sequence, paying
/// `output_value` to `script_pubkey` (the leaf's verifying key). Operators
/// rebuild it from the direct node tx and compare it byte for byte, so only the
/// value may differ between rungs.
#[must_use]
pub fn direct_node_ladder_rung_tx(
    direct_node_tx: &Transaction,
    script_pubkey: ScriptBuf,
    output_value: u64,
) -> Transaction {
    Transaction {
        version: direct_node_tx.version,
        lock_time: LockTime::ZERO,
        input: direct_node_tx
            .input
            .iter()
            .take(1)
            .map(|input| TxIn {
                previous_output: input.previous_output,
                sequence: input.sequence,
                ..Default::default()
            })
            .collect(),
        output: vec![TxOut {
            value: Amount::from_sat(output_value),
            script_pubkey,
        }],
    }
}

/// The vsize of `tx` once its single input carries a key-path signature, which
/// is the size the operators price the relay floor against.
#[must_use]
pub fn signed_key_path_vsize(tx: &Transaction) -> u64 {
    let mut signed = tx.clone();
    for input in &mut signed.input {
        input.witness = Witness::from_slice(&[[0u8; SCHNORR_SIGNATURE_SIZE]]);
    }
    signed.vsize() as u64
}

/// The fees of a refund ladder for `refund_tx`: each rung pays for the refund
/// and itself, which are the same size.
#[must_use]
pub fn refund_ladder_fees(refund_tx: &Transaction) -> Vec<u64> {
    let Some(output) = refund_tx.output.first() else {
        return Vec::new();
    };
    let value = output.value.to_sat();
    let rung_vsize = signed_key_path_vsize(&refund_rung_tx(refund_tx, value));
    ladder_fees(value, rung_vsize.saturating_mul(2), &output.script_pubkey)
}

/// The rungs of a direct node ladder for `direct_node_tx`, cheapest first, spending
/// a parent output of `parent_value` sats. Empty when the output is too small
/// to fund one.
#[must_use]
pub fn direct_node_ladder_txs(
    direct_node_tx: &Transaction,
    script_pubkey: &ScriptBuf,
    parent_value: u64,
) -> Vec<Transaction> {
    let probe = direct_node_ladder_rung_tx(direct_node_tx, script_pubkey.clone(), parent_value);
    ladder_fees(parent_value, signed_key_path_vsize(&probe), script_pubkey)
        .into_iter()
        .map(|fee| {
            direct_node_ladder_rung_tx(direct_node_tx, script_pubkey.clone(), parent_value - fee)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use bitcoin::{Txid, hashes::Hash};

    use super::*;

    fn p2tr_script() -> ScriptBuf {
        let mut bytes = vec![0x51, 0x20];
        bytes.extend_from_slice(&[7u8; 32]);
        ScriptBuf::from_bytes(bytes)
    }

    fn dust() -> u64 {
        p2tr_script().minimal_non_dust().to_sat()
    }

    fn fees(value: u64, vsize: u64) -> Vec<u64> {
        ladder_fees(value, vsize, &p2tr_script())
    }

    /// A refund without an anchor: one output paying `value` to the receiver.
    fn anchorless_refund(value: u64) -> Transaction {
        Transaction {
            version: Version::non_standard(3),
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint {
                    txid: Txid::from_byte_array([1; 32]),
                    vout: 0,
                },
                sequence: Sequence(1 << 30 | 1900),
                ..Default::default()
            }],
            output: vec![TxOut {
                value: Amount::from_sat(value),
                script_pubkey: p2tr_script(),
            }],
        }
    }

    fn assert_valid_ladder(fees: &[u64], value: u64, vsize: u64) {
        assert!(!fees.is_empty() && fees.len() <= LADDER_MAX_RUNGS);
        assert!(fees[0] >= vsize);
        assert_eq!(*fees.last().unwrap(), value - dust());
        for pair in fees.windows(2) {
            assert!(
                pair[1] * 100 >= pair[0] * 125,
                "{} after {}",
                pair[1],
                pair[0]
            );
        }
    }

    #[test]
    fn a_ladder_runs_from_the_relay_floor_to_dust() {
        for value in [1_000, 5_000, 38_000, 100_000, 1_000_000, 100_000_000] {
            assert_valid_ladder(&fees(value, 222), value, 222);
        }
    }

    #[test]
    fn a_large_output_still_reaches_dust_within_the_rung_limit() {
        let value = 21_000_000 * 100_000_000;
        let ladder = fees(value, 222);
        assert_eq!(ladder.len(), LADDER_MAX_RUNGS);
        assert_valid_ladder(&ladder, value, 222);
    }

    #[test]
    fn a_small_output_gets_fewer_rungs() {
        let ladder = fees(5_000, 222);
        assert!(ladder.len() < LADDER_MAX_RUNGS);
        assert_valid_ladder(&ladder, 5_000, 222);
    }

    #[test]
    fn an_output_just_above_the_floor_gets_only_the_top_rung() {
        assert_eq!(fees(600, 222), vec![270]);
    }

    #[test]
    fn an_output_that_cannot_fund_the_floor_gets_no_ladder() {
        assert!(fees(551, 222).is_empty());
        assert!(fees(300, 222).is_empty());
        assert!(fees(10_000, 0).is_empty());
        assert_eq!(fees(552, 222), vec![222]);
    }

    #[test]
    fn a_refund_rung_spends_the_refund_and_pays_the_same_script() {
        let refund = anchorless_refund(10_000);
        let rung = refund_rung_tx(&refund, 9_000);
        assert_eq!(rung.version, Version::non_standard(3));
        assert_eq!(rung.lock_time, LockTime::ZERO);
        assert_eq!(rung.input.len(), 1);
        assert_eq!(rung.input[0].previous_output.txid, refund.compute_txid());
        assert_eq!(rung.input[0].previous_output.vout, 0);
        assert_eq!(rung.input[0].sequence, Sequence(0xffff_fffd));
        assert_eq!(rung.output.len(), 1);
        assert_eq!(rung.output[0].script_pubkey, refund.output[0].script_pubkey);
        assert_eq!(rung.output[0].value, Amount::from_sat(9_000));
    }

    #[test]
    fn a_refund_ladder_prices_the_refund_and_the_rung() {
        let refund = anchorless_refund(100_000);
        let rung_vsize = signed_key_path_vsize(&refund_rung_tx(&refund, 100_000));
        assert_eq!(rung_vsize, signed_key_path_vsize(&refund));
        let ladder = refund_ladder_fees(&refund);
        assert_valid_ladder(&ladder, 100_000, rung_vsize * 2);
        assert_eq!(ladder[0], rung_vsize * 2);
    }

    #[test]
    fn direct_node_ladder_rungs_copy_the_direct_tx_and_vary_only_the_value() {
        let direct = anchorless_refund(50_000);
        let script = p2tr_script();
        let rungs = direct_node_ladder_txs(&direct, &script, 50_000);
        assert!(!rungs.is_empty());
        for rung in &rungs {
            assert_eq!(rung.version, direct.version);
            assert_eq!(rung.lock_time, LockTime::ZERO);
            assert_eq!(rung.input.len(), 1);
            assert_eq!(
                rung.input[0].previous_output,
                direct.input[0].previous_output
            );
            assert_eq!(rung.input[0].sequence, direct.input[0].sequence);
            assert_eq!(rung.output.len(), 1);
            assert_eq!(rung.output[0].script_pubkey, script);
        }
        let fees: Vec<u64> = rungs
            .iter()
            .map(|rung| 50_000 - rung.output[0].value.to_sat())
            .collect();
        assert_valid_ladder(&fees, 50_000, signed_key_path_vsize(&rungs[0]));
    }
}
