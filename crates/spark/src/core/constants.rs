use bitcoin::{
    Sequence,
    relative::{Height, LockTime},
};

pub const INITIAL_TIME_LOCK: u32 = 2000;
pub const TIME_LOCK_INTERVAL: u32 = 100;

pub fn initial_sequence() -> Sequence {
    let height = Height::from_height(INITIAL_TIME_LOCK as u16);
    let locktime = LockTime::Blocks(height);
    locktime.to_sequence()
}

pub fn next_sequence(current_sequence: Sequence) -> Option<Sequence> {
    let Some(current_locktime) = current_sequence.to_relative_lock_time() else {
        return None;
    };

    let LockTime::Blocks(blocks) = current_locktime else {
        return None;
    };

    let Some(new_blocks) = blocks.value().checked_sub(TIME_LOCK_INTERVAL as u16) else {
        return None;
    };

    let new_locktime = LockTime::Blocks(Height::from_height(new_blocks));
    Some(new_locktime.to_sequence())
}
