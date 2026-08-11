use bitcoin::{
    Sequence,
    relative::{Height, LockTime},
};
use tracing::{trace, warn};

const SPARK_SEQUENCE_FLAG: u32 = 1 << 30;
const TIMELOCK_MASK: u32 = 0x0000_FFFF;
const INITIAL_TIME_LOCK: u16 = 2000;
const TIME_LOCK_INTERVAL: u16 = 100;
const DIRECT_TIME_LOCK_OFFSET: u16 = 50;
const MAX_TIME_LOCK: u16 = INITIAL_TIME_LOCK + DIRECT_TIME_LOCK_OFFSET;
const DIRECT_HTLC_TIME_LOCK_OFFSET: u16 = 85;
const HTLC_TIME_LOCK_OFFSET: u16 = 70;

pub fn initial_timelock_sequence() -> (Sequence, Sequence) {
    (
        to_sequence(INITIAL_TIME_LOCK, 0),
        to_sequence(INITIAL_TIME_LOCK + DIRECT_TIME_LOCK_OFFSET, 0),
    )
}

pub fn initial_root_timelock_sequence() -> (Sequence, Sequence) {
    (to_sequence(0, 0), to_sequence(DIRECT_TIME_LOCK_OFFSET, 0))
}

pub fn initial_zero_timelock_sequence() -> (Sequence, Sequence) {
    (to_sequence(0, 0), to_sequence(DIRECT_TIME_LOCK_OFFSET, 0))
}

/// Reads the block-height timelock out of an untrusted sequence. `None` for a
/// disabled or time-based lock, or a value above `MAX_TIME_LOCK`.
pub(crate) fn checked_timelock(sequence: Sequence) -> Option<u16> {
    if !sequence.is_height_locked() {
        return None;
    }
    let timelock = (sequence.to_consensus_u32() & TIMELOCK_MASK) as u16;
    (timelock <= MAX_TIME_LOCK).then_some(timelock)
}

/// `None` when the direct offset does not fit the 16-bit timelock field. Left
/// unchecked the addition wraps in release builds and panics in debug ones.
pub fn current_sequence(current_sequence: Sequence) -> Option<(Sequence, Sequence)> {
    let timelock = (current_sequence.to_consensus_u32() & TIMELOCK_MASK) as u16;
    let direct_timelock = timelock.checked_add(DIRECT_TIME_LOCK_OFFSET)?;
    let spark_sequence_flag = spark_sequence_flag(current_sequence);
    Some((
        current_sequence,
        to_sequence(direct_timelock, spark_sequence_flag),
    ))
}

/// Enforces timelock alignment to `TIME_LOCK_INTERVAL` (100 blocks) by rounding down.
///
/// This is used during claim operations to ensure timelocks are aligned to 100-block
/// boundaries (X00 or X50 with the direct offset).
///
/// # Examples
/// - 1950 -> 1900
/// - 1900 -> 1900 (already aligned)
/// - 1899 -> 1800
pub fn enforce_timelock(sequence: Sequence) -> Sequence {
    let timelock = (sequence.to_consensus_u32() & TIMELOCK_MASK) as u16;

    // Round down to nearest TIME_LOCK_INTERVAL
    let enforced_timelock = timelock - (timelock % TIME_LOCK_INTERVAL);

    let spark_flag = spark_sequence_flag(sequence);
    to_sequence(enforced_timelock, spark_flag)
}

/// Calculates the next pair of sequence numbers for transaction timelocks.
///
/// This function is used in the Spark protocol to generate decreasing timelocks
/// for refund transactions. Each call decreases the timelock by `TIME_LOCK_INTERVAL` blocks.
/// It returns both a CPFP sequence and a direct sequence, where the direct sequence
/// is offset by `DIRECT_TIME_LOCK_OFFSET` blocks from the CPFP sequence.
///
/// # Arguments
///
/// * `current_sequence` - The current sequence number to decrement
///
/// # Returns
///
/// * `Some((cpfp_sequence, direct_sequence))` - A tuple containing the next CPFP and direct sequence numbers
/// * `None` - If the timelock can't be decreased further (would go below zero)
///
/// # Notes
///
/// - CPFP sequences are used for transactions that include an anchor output for fee bumping
/// - Direct sequences are used for transactions that spend directly without anchor outputs
/// - The direct sequence is always `DIRECT_TIME_LOCK_OFFSET` blocks higher than the CPFP sequence
pub fn next_sequence(current_sequence: Sequence) -> Option<(Sequence, Sequence)> {
    let next_timelock = check_next_timelock(current_sequence)?;
    let spark_sequence_flag = spark_sequence_flag(current_sequence);
    Some((
        to_sequence(next_timelock, spark_sequence_flag),
        to_sequence(next_timelock + DIRECT_TIME_LOCK_OFFSET, spark_sequence_flag),
    ))
}

/// Calculates the next pair of sequence numbers for HTLC timelocks in a Lightning transaction.
///
/// This function is used in the Spark protocol to generate decreasing timelocks
/// for HTLC refund transactions. Each call decreases the timelock by `TIME_LOCK_INTERVAL` blocks.
/// It returns both a CPFP sequence and a direct sequence, where each sequence
/// is offset by `HTLC_TIME_LOCK_OFFSET` and `DIRECT_HTLC_TIME_LOCK_OFFSET` respectively.
///
/// # Arguments
///
/// * `current_sequence` - The current sequence number to decrement
///
/// # Returns
///
/// * `Some((cpfp_sequence, direct_sequence))` - A tuple containing the next CPFP and direct sequence numbers
/// * `None` - If the timelock can't be decreased further (would go below zero)
///
/// # Notes
///
/// - CPFP sequences are used for transactions that include an anchor output for fee bumping
/// - Direct sequences are used for transactions that spend directly without anchor outputs
/// - CPFP sequences are offset by `HTLC_TIME_LOCK_OFFSET` blocks
/// - Direct sequences are offset by `DIRECT_HTLC_TIME_LOCK_OFFSET` blocks
/// - Both offsets are applied to the base timelock calculated from `current_sequence`
pub fn next_lightning_htlc_sequence(current_sequence: Sequence) -> Option<(Sequence, Sequence)> {
    let next_timelock = check_next_timelock(current_sequence)?;
    let spark_sequence_flag = spark_sequence_flag(current_sequence);
    Some((
        to_sequence(next_timelock + HTLC_TIME_LOCK_OFFSET, spark_sequence_flag),
        to_sequence(
            next_timelock + DIRECT_HTLC_TIME_LOCK_OFFSET,
            spark_sequence_flag,
        ),
    ))
}

/// Extracts the 30th bit flag from the given sequence number.
fn spark_sequence_flag(current_sequence: Sequence) -> u32 {
    current_sequence.to_consensus_u32() & SPARK_SEQUENCE_FLAG
}

fn to_sequence(blocks: u16, spark_sequence_flag: u32) -> Sequence {
    let new_locktime = LockTime::Blocks(Height::from_height(blocks));
    let sequence = Sequence::from_consensus(new_locktime.to_consensus_u32() | spark_sequence_flag);
    trace!("To sequence: {sequence:?}");
    sequence
}

fn check_next_timelock(current_sequence: Sequence) -> Option<u16> {
    trace!("Current sequence: {current_sequence:?}");   
    let timelock = checked_timelock(current_sequence).unwrap_or_else(|| {
        warn!(
            "Deriving the next sequence from an unexpected timelock {:#x}",
            current_sequence.to_consensus_u32()
        );
        (current_sequence.to_consensus_u32() & TIMELOCK_MASK) as u16
    });

    timelock.checked_sub(TIME_LOCK_INTERVAL).or_else(|| {
        trace!(
            "Current sequence locktime {} is too low to calculate next sequence",
            current_sequence
        );
        None
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use macros::test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    #[test_all]
    fn test_initial_timelock_sequence() {
        let (cpfp_sequence, direct_sequence) = initial_timelock_sequence();

        assert!(cpfp_sequence.is_height_locked());
        assert!(direct_sequence.is_height_locked());

        assert!(cpfp_sequence.is_relative_lock_time());
        assert!(direct_sequence.is_relative_lock_time());

        assert!(!cpfp_sequence.is_time_locked());
        assert!(!direct_sequence.is_time_locked());

        let cpfp_locktime = cpfp_sequence.to_relative_lock_time().unwrap();
        let direct_locktime = direct_sequence.to_relative_lock_time().unwrap();

        assert!(cpfp_locktime.is_block_height());
        let LockTime::Blocks(cpfp_height) = cpfp_locktime else {
            panic!("Expected a cpfp block height locktime");
        };

        assert!(direct_locktime.is_block_height());
        let LockTime::Blocks(direct_height) = direct_locktime else {
            panic!("Expected a direct block height locktime");
        };

        assert_eq!(cpfp_height.value(), INITIAL_TIME_LOCK);
        assert_eq!(
            direct_height.value(),
            INITIAL_TIME_LOCK + DIRECT_TIME_LOCK_OFFSET
        );

        assert_eq!(cpfp_sequence.to_consensus_u32() & SPARK_SEQUENCE_FLAG, 0);
        assert_eq!(direct_sequence.to_consensus_u32() & SPARK_SEQUENCE_FLAG, 0);
    }

    #[test_all]
    fn test_initial_zero_timelock_sequence() {
        let (cpfp_sequence, direct_sequence) = initial_zero_timelock_sequence();

        assert_eq!(cpfp_sequence.to_consensus_u32() & SPARK_SEQUENCE_FLAG, 0);
        assert_eq!(direct_sequence.to_consensus_u32() & SPARK_SEQUENCE_FLAG, 0);

        let LockTime::Blocks(cpfp_height) = cpfp_sequence.to_relative_lock_time().unwrap() else {
            panic!("Expected a cpfp block height locktime");
        };
        assert_eq!(cpfp_height.value(), 0);

        let LockTime::Blocks(direct_height) = direct_sequence.to_relative_lock_time().unwrap()
        else {
            panic!("Expected a direct block height locktime");
        };
        assert_eq!(direct_height.value(), DIRECT_TIME_LOCK_OFFSET);
    }

    #[test_all]
    fn test_initial_root_timelock_sequence() {
        let (cpfp_sequence, direct_sequence) = initial_root_timelock_sequence();

        assert_eq!(cpfp_sequence.to_consensus_u32() & SPARK_SEQUENCE_FLAG, 0);
        assert_eq!(direct_sequence.to_consensus_u32() & SPARK_SEQUENCE_FLAG, 0);

        let LockTime::Blocks(cpfp_height) = cpfp_sequence.to_relative_lock_time().unwrap() else {
            panic!("Expected a cpfp block height locktime");
        };
        assert_eq!(cpfp_height.value(), 0);

        let LockTime::Blocks(direct_height) = direct_sequence.to_relative_lock_time().unwrap()
        else {
            panic!("Expected a direct block height locktime");
        };
        assert_eq!(direct_height.value(), DIRECT_TIME_LOCK_OFFSET);
    }

    #[test_all]
    fn test_with_spark_sequence_flag() {
        let sequence = Sequence::from_consensus(INITIAL_TIME_LOCK as u32 | SPARK_SEQUENCE_FLAG);
        let (next_cpfp, next_direct) = next_sequence(sequence).unwrap();

        assert_eq!(
            next_cpfp.to_consensus_u32() & SPARK_SEQUENCE_FLAG,
            SPARK_SEQUENCE_FLAG
        );
        assert_eq!(
            next_direct.to_consensus_u32() & SPARK_SEQUENCE_FLAG,
            SPARK_SEQUENCE_FLAG
        );
    }

    #[test_all]
    fn test_without_spark_sequence_flag() {
        let sequence = Sequence::from_consensus(INITIAL_TIME_LOCK as u32);
        let (next_cpfp, next_direct) = next_sequence(sequence).unwrap();

        assert_eq!(next_cpfp.to_consensus_u32() & SPARK_SEQUENCE_FLAG, 0);
        assert_eq!(next_direct.to_consensus_u32() & SPARK_SEQUENCE_FLAG, 0);
    }

    #[test_all]
    fn test_next_sequence() {
        let (mut cpfp_sequence, mut direct_sequence) = initial_timelock_sequence();

        for i in 1u16..21 {
            let next_sequences = next_sequence(cpfp_sequence);
            let (cpfp, direct) = next_sequences.unwrap();
            assert!(cpfp.is_height_locked());
            assert!(cpfp.is_relative_lock_time());

            let LockTime::Blocks(cpfp_height) = cpfp.to_relative_lock_time().unwrap() else {
                panic!("Expected a block height locktime for cpfp sequence");
            };
            assert_eq!(
                cpfp_height.value(),
                INITIAL_TIME_LOCK - i * TIME_LOCK_INTERVAL
            );

            let LockTime::Blocks(direct_height) = direct.to_relative_lock_time().unwrap() else {
                panic!("Expected a block height locktime for direct sequence");
            };
            assert_eq!(
                direct_height.value(),
                cpfp_height.value() + DIRECT_TIME_LOCK_OFFSET
            );
            cpfp_sequence = cpfp;
            direct_sequence = direct;
        }

        let LockTime::Blocks(height) = cpfp_sequence.to_relative_lock_time().unwrap() else {
            panic!("Expected a block height locktime for cpfp sequence");
        };
        assert_eq!(height.value(), 0);

        let LockTime::Blocks(direct_height) = direct_sequence.to_relative_lock_time().unwrap()
        else {
            panic!("Expected a block height locktime for direct sequence");
        };
        assert_eq!(direct_height.value(), DIRECT_TIME_LOCK_OFFSET);

        let next = next_sequence(cpfp_sequence);
        assert!(next.is_none());
    }

    /// The claim path applies `enforce_timelock` before `current_sequence`, so a
    /// sequence of 0xFFFF reaches the direct offset as 65500. Left unchecked the
    /// addition wrapped to 14 in release builds and panicked in debug ones.
    #[test_all]
    fn rejects_a_timelock_whose_direct_offset_would_wrap() {
        let sequence = Sequence::from_consensus(0xFFFF);
        assert!(current_sequence(enforce_timelock(sequence)).is_none());
    }

    #[test_all]
    fn accepts_a_timelock_at_the_representable_boundary() {
        let sequence = Sequence::from_consensus(u32::from(u16::MAX - DIRECT_TIME_LOCK_OFFSET));
        assert!(current_sequence(sequence).is_some());
    }

    #[test_all]
    fn rejects_a_timelock_one_past_the_representable_boundary() {
        let sequence = Sequence::from_consensus(u32::from(u16::MAX - DIRECT_TIME_LOCK_OFFSET) + 1);
        assert!(current_sequence(sequence).is_none());
    }

    /// A timelock above the protocol maximum is reported by the sanity check, not
    /// rejected here: the arithmetic still has to produce a usable pair so the
    /// claim can proceed.
    #[test_all]
    fn accepts_a_timelock_above_the_protocol_max() {
        let sequence = Sequence::from_consensus(u32::from(MAX_TIME_LOCK) + 1);
        assert!(current_sequence(sequence).is_some());
        assert!(checked_timelock(sequence).is_none());
    }

    #[test_all]
    fn accepts_a_spark_flagged_sequence() {
        let sequence = Sequence::from_consensus(u32::from(INITIAL_TIME_LOCK) | SPARK_SEQUENCE_FLAG);
        assert!(current_sequence(sequence).is_some());
        assert!(checked_timelock(sequence).is_some());
    }

    #[test_all]
    fn accepts_a_zero_timelock() {
        assert_eq!(checked_timelock(Sequence::from_consensus(0)), Some(0));
    }

    /// `check_next_timelock` warns on anything `checked_timelock` rejects, so
    /// every sequence the protocol itself produces has to pass. Otherwise a
    /// healthy wallet logs a warning on each renewal.
    #[test_all]
    fn every_protocol_sequence_passes_the_timelock_check() {
        let (mut cpfp, direct) = initial_timelock_sequence();
        assert!(checked_timelock(cpfp).is_some());
        assert!(checked_timelock(direct).is_some());

        while let Some((next_cpfp, next_direct)) = next_sequence(cpfp) {
            assert!(checked_timelock(next_cpfp).is_some(), "cpfp {next_cpfp}");
            assert!(
                checked_timelock(next_direct).is_some(),
                "direct {next_direct}"
            );
            // The HTLC ladder branches off the same sequence at wider offsets.
            let (htlc_cpfp, htlc_direct) = next_lightning_htlc_sequence(cpfp).unwrap();
            assert!(checked_timelock(htlc_cpfp).is_some(), "htlc {htlc_cpfp}");
            assert!(
                checked_timelock(htlc_direct).is_some(),
                "htlc direct {htlc_direct}"
            );
            cpfp = next_cpfp;
        }
    }

    #[test_all]
    fn rejects_a_time_based_timelock() {
        let sequence = Sequence::from_consensus(u32::from(INITIAL_TIME_LOCK) | (1 << 22));
        assert!(checked_timelock(sequence).is_none());
    }

    /// `is_zero_timelock` truncates with `as u16` and so cannot see the disable
    /// bit: a sequence with no relative locktime at all reads as a zero timelock
    /// and silently drops the direct refund tx from the claim.
    #[test_all]
    fn rejects_a_disabled_sequence_that_reads_as_a_zero_timelock() {
        assert!(checked_timelock(Sequence::from_consensus(1 << 31)).is_none());
    }

    #[test_all]
    fn test_enforce_timelock_rounds_down() {
        // 1950 should round down to 1900
        let sequence = Sequence::from_consensus(1950 | SPARK_SEQUENCE_FLAG);
        let enforced = enforce_timelock(sequence);

        let LockTime::Blocks(height) = enforced.to_relative_lock_time().unwrap() else {
            panic!("Expected block height locktime");
        };
        assert_eq!(height.value(), 1900);

        // Spark flag should be preserved
        assert_eq!(
            enforced.to_consensus_u32() & SPARK_SEQUENCE_FLAG,
            SPARK_SEQUENCE_FLAG
        );
    }

    #[test_all]
    fn test_enforce_timelock_already_aligned() {
        // 1900 should stay 1900
        let sequence = Sequence::from_consensus(1900 | SPARK_SEQUENCE_FLAG);
        let enforced = enforce_timelock(sequence);

        let LockTime::Blocks(height) = enforced.to_relative_lock_time().unwrap() else {
            panic!("Expected block height locktime");
        };
        assert_eq!(height.value(), 1900);
    }

    #[test_all]
    fn test_enforce_timelock_edge_case() {
        // 99 should round down to 0
        let sequence = Sequence::from_consensus(99 | SPARK_SEQUENCE_FLAG);
        let enforced = enforce_timelock(sequence);

        let LockTime::Blocks(height) = enforced.to_relative_lock_time().unwrap() else {
            panic!("Expected block height locktime");
        };
        assert_eq!(height.value(), 0);
    }
}
