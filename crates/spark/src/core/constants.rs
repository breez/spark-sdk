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
    if !current_sequence.is_height_locked() {
        trace!(
            "Current sequence {} is not height locked, cannot calculate next sequence",
            current_sequence
        );
        return None;
    }

    let Some(current_locktime) = current_sequence.to_relative_lock_time() else {
        trace!(
            "Current sequence {} is not a relative lock time, cannot calculate next sequence",
            current_sequence
        );
        return None;
    };

    let LockTime::Blocks(blocks) = current_locktime else {
        trace!(
            "Current sequence locktime {} is not expressed in blocks, cannot calculate next sequence",
            current_sequence
        );
        return None;
    };

    let Some(new_blocks) = blocks.value().checked_sub(TIME_LOCK_INTERVAL) else {
        trace!(
            "Current sequence locktime {} is too low to calculate next sequence",
            current_sequence
        );
        return None;
    };

    if new_blocks < TIME_LOCK_INTERVAL {
        trace!(
            "new_blocks {} is less than TIME_LOCK_INTERVAL {}, cannot calculate next sequence for {}",
            new_blocks, TIME_LOCK_INTERVAL, current_sequence
        );
        return None;
    }

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

        for i in 1u16..20 {
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

        assert_eq!(height.value(), TIME_LOCK_INTERVAL);
        let next = next_sequence(sequence);
        assert!(next.is_none());
    }
}
