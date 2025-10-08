use bitcoin::{
    Sequence,
    relative::{Height, LockTime},
};
use tracing::trace;

const SPARK_SEQUENCE_FLAG: u32 = 1 << 30;
const INITIAL_TIME_LOCK: u16 = 2000;
const TIME_LOCK_INTERVAL: u16 = 100;
const DIRECT_TIME_LOCK_OFFSET: u16 = 50;
const DIRECT_HTLC_TIME_LOCK_OFFSET: u16 = 85;
const HTLC_TIME_LOCK_OFFSET: u16 = 70;

pub fn initial_cpfp_sequence() -> Sequence {
    to_sequence(INITIAL_TIME_LOCK)
}

pub fn initial_direct_sequence() -> Sequence {
    to_sequence(INITIAL_TIME_LOCK + DIRECT_TIME_LOCK_OFFSET)
}

pub fn current_sequence(current_sequence: Sequence) -> (Sequence, Sequence) {
    let timelock = current_sequence.to_consensus_u32() as u16;
    (
        current_sequence,
        to_sequence(timelock + DIRECT_TIME_LOCK_OFFSET),
    )
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
    Some((
        to_sequence(next_timelock),
        to_sequence(next_timelock + DIRECT_TIME_LOCK_OFFSET),
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
    Some((
        to_sequence(next_timelock + HTLC_TIME_LOCK_OFFSET),
        to_sequence(next_timelock + DIRECT_HTLC_TIME_LOCK_OFFSET),
    ))
}

fn to_sequence(blocks: u16) -> Sequence {
    let new_locktime = LockTime::Blocks(Height::from_height(blocks));
    let sequence = Sequence::from_consensus(new_locktime.to_consensus_u32() | SPARK_SEQUENCE_FLAG);
    trace!("To sequence: {sequence:?}");
    sequence
}

fn check_next_timelock(current_sequence: Sequence) -> Option<u16> {
    trace!("Current sequence: {current_sequence:?}");
    let current_sequence_num = current_sequence.to_consensus_u32();
    let timelock = current_sequence_num as u16;
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
    fn test_initial_sequence() {
        let sequence = initial_cpfp_sequence();
        assert!(sequence.is_height_locked());
        assert!(sequence.is_relative_lock_time());
        assert!(!sequence.is_time_locked());
        let locktime = sequence.to_relative_lock_time().unwrap();
        assert!(locktime.is_block_height());
        let LockTime::Blocks(height) = locktime else {
            panic!("Expected a block height locktime");
        };

        assert_eq!(height.value(), INITIAL_TIME_LOCK);
    }

    #[test_all]
    fn test_initial_direct_sequence() {
        let sequence = initial_direct_sequence();
        assert!(sequence.is_height_locked());
        assert!(sequence.is_relative_lock_time());
        assert!(!sequence.is_time_locked());
        let locktime = sequence.to_relative_lock_time().unwrap();
        assert!(locktime.is_block_height());
        let LockTime::Blocks(height) = locktime else {
            panic!("Expected a block height locktime");
        };

        assert_eq!(height.value(), INITIAL_TIME_LOCK + DIRECT_TIME_LOCK_OFFSET);
    }

    #[test_all]
    fn test_next_sequence() {
        let mut cpfp_sequence = initial_cpfp_sequence();
        let mut direct_sequence = initial_direct_sequence();

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
}
