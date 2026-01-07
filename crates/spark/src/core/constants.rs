use bitcoin::{
    Sequence,
    relative::{Height, LockTime},
};
use tracing::trace;

const SPARK_SEQUENCE_FLAG: u32 = 1 << 30;
const TIMELOCK_MASK: u32 = 0x0000_FFFF;
const INITIAL_TIME_LOCK: u16 = 2000;
const TIME_LOCK_INTERVAL: u16 = 100;
const DIRECT_TIME_LOCK_OFFSET: u16 = 50;
const DIRECT_HTLC_TIME_LOCK_OFFSET: u16 = 85;
const HTLC_TIME_LOCK_OFFSET: u16 = 70;

pub fn initial_timelock_sequence() -> (Sequence, Sequence) {
    (
        to_sequence(INITIAL_TIME_LOCK, SPARK_SEQUENCE_FLAG),
        to_sequence(
            INITIAL_TIME_LOCK + DIRECT_TIME_LOCK_OFFSET,
            SPARK_SEQUENCE_FLAG,
        ),
    )
}

pub fn initial_root_timelock_sequence() -> (Sequence, Sequence) {
    (
        to_sequence(0, SPARK_SEQUENCE_FLAG),
        to_sequence(DIRECT_TIME_LOCK_OFFSET, SPARK_SEQUENCE_FLAG),
    )
}

pub fn initial_zero_timelock_sequence() -> (Sequence, Sequence) {
    (
        to_sequence(0, SPARK_SEQUENCE_FLAG),
        to_sequence(DIRECT_TIME_LOCK_OFFSET, 0),
    )
}

pub fn current_sequence(current_sequence: Sequence) -> (Sequence, Sequence) {
    let timelock = current_sequence.to_consensus_u32() as u16;
    let spark_sequence_flag = spark_sequence_flag(current_sequence);
    (
        current_sequence,
        to_sequence(timelock + DIRECT_TIME_LOCK_OFFSET, spark_sequence_flag),
    )
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
    let current_sequence_num = sequence.to_consensus_u32();

    // Extract lower 16 bits (timelock value)
    let timelock = (current_sequence_num & TIMELOCK_MASK) as u16;

    // Round down to nearest TIME_LOCK_INTERVAL
    let remainder = timelock % TIME_LOCK_INTERVAL;
    let enforced_timelock = if remainder != 0 {
        timelock - remainder
    } else {
        timelock
    };

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
    let current_sequence_num = current_sequence.to_consensus_u32();

    // Extract only the lower 16 bits (timelock value)
    // Upper bits including SPARK_SEQUENCE_FLAG are ignored for timelock calculation
    let timelock = (current_sequence_num & TIMELOCK_MASK) as u16;

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
    }

    #[test_all]
    fn test_with_spark_sequence_flag() {
        let (sequence, _) = initial_timelock_sequence();
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
