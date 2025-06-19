use bitcoin::Sequence;

pub const INITIAL_TIME_LOCK: u32 = 2000;
pub const TIME_LOCK_INTERVAL: u32 = 100;

pub fn initial_sequence() -> Sequence {
    Sequence((1 << 30) | INITIAL_TIME_LOCK)
}
