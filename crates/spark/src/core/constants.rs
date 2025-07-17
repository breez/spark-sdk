use bitcoin::{
    Sequence,
    relative::{Height, LockTime},
};
use tracing::trace;

pub const INITIAL_TIME_LOCK: u16 = 2000;
pub const TIME_LOCK_INTERVAL: u16 = 100;
pub const SPARK_SEQUENCE_FLAG: u32 = 1 << 30;

pub fn initial_sequence() -> Sequence {
    to_sequence(INITIAL_TIME_LOCK)
}

pub fn next_sequence(current_sequence: Sequence) -> Option<Sequence> {
    let current_sequence_num = current_sequence.to_consensus_u32();
    trace!("Current sequence {}", current_sequence_num);
    let timelock = current_sequence_num as u16;
    let Some(new_blocks) = timelock.checked_sub(TIME_LOCK_INTERVAL) else {
        trace!(
            "Current sequence locktime {} is too low to calculate next sequence",
            current_sequence
        );
        return None;
    };

    Some(to_sequence(new_blocks))
}

fn to_sequence(blocks: u16) -> Sequence {
    let new_locktime = LockTime::Blocks(Height::from_height(blocks));
    Sequence::from_consensus(new_locktime.to_consensus_u32() | SPARK_SEQUENCE_FLAG)
}

pub fn validate_sequence(sequence: Sequence) -> bool {
    if !sequence.is_height_locked() {
        return false;
    }

    let Some(locktime) = sequence.to_relative_lock_time() else {
        return false;
    };

    let LockTime::Blocks(_) = locktime else {
        return false;
    };

    true
}
#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_initial_sequence() {
        let sequence = initial_sequence();
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

    #[test]
    fn test_next_sequence() {
        let mut sequence = initial_sequence();

        for i in 1u16..21 {
            let next = next_sequence(sequence);
            let next = next.unwrap();
            assert!(next.is_height_locked());
            assert!(next.is_relative_lock_time());

            let LockTime::Blocks(height) = next.to_relative_lock_time().unwrap() else {
                panic!("Expected a block height locktime");
            };
            assert_eq!(height.value(), INITIAL_TIME_LOCK - i * TIME_LOCK_INTERVAL);
            sequence = next;
        }

        let LockTime::Blocks(height) = sequence.to_relative_lock_time().unwrap() else {
            panic!("Expected a block height locktime");
        };

        assert_eq!(height.value(), 0);
        let next = next_sequence(sequence);
        assert!(next.is_none());
    }
}
