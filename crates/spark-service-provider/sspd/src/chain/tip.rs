use std::sync::{Arc, Mutex, PoisonError};

use super::SyncState;
use super::{BlockHeader, ChainRepository, ChainRepositoryError};

#[derive(Default)]
struct Tips {
    next: Option<BlockHeader>,
    stored: Option<BlockHeader>,
    bitcoind: Option<SyncState>,
}

/// The chain monitor keeps the chain record's tip and bitcoind's sync state in
/// memory, so the rest of sspd reads them without a database query or a call
/// to bitcoind.
///
/// The monitor cannot change the database and memory in one instant, so it
/// keeps two copies of the record's tip. Before each write to the record, it
/// sets the next tip to the tip the record is about to have. Once the database
/// has the change, it copies the next tip to the stored tip. While the two tips
/// differ, the monitor is writing or has failed to write, and the record's tip
/// in the database is either of them.
#[derive(Clone, Default)]
pub struct ChainTip(Arc<Mutex<Tips>>);

impl ChainTip {
    pub async fn load<R: ChainRepository + Sync>(
        chain_repository: &R,
    ) -> Result<Self, ChainRepositoryError> {
        let chain_tip = Self::default();
        chain_tip.stored_is(chain_repository.get_tip().await?);
        Ok(chain_tip)
    }

    #[cfg(test)]
    pub fn at(tip: BlockHeader) -> Self {
        let chain_tip = Self::default();
        chain_tip.stored_is(Some(tip));
        chain_tip
    }

    fn tips(&self) -> std::sync::MutexGuard<'_, Tips> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The lower of the two tips: a block that the record has even while the
    /// monitor is writing. What sspd reads from the record is complete up to
    /// this block, so a count of confirmations against it is never too high.
    /// `None` while the record has no block.
    pub fn record(&self) -> Option<BlockHeader> {
        let tips = self.tips();
        let (next, stored) = (tips.next.as_ref()?, tips.stored.as_ref()?);
        let lower = if next.height < stored.height {
            next
        } else {
            stored
        };
        Some(lower.clone())
    }

    /// The chain record's tip, while the two tips are the same block. `None`
    /// while the chain monitor writes, from a failed write until the monitor's
    /// next sync, and while the record has no block.
    pub fn stored(&self) -> Option<BlockHeader> {
        let tips = self.tips();
        if tips.next == tips.stored {
            tips.stored.clone()
        } else {
            None
        }
    }

    /// bitcoind's sync state at the start of the chain monitor's last sync.
    /// `None` until the monitor has fetched it.
    pub fn bitcoind_sync_state(&self) -> Option<SyncState> {
        self.tips().bitcoind
    }

    pub(super) fn stored_is(&self, tip: Option<BlockHeader>) {
        let mut tips = self.tips();
        tips.next.clone_from(&tip);
        tips.stored = tip;
    }

    pub(super) fn writing(&self, next: Option<BlockHeader>) {
        self.tips().next = next;
    }

    pub(super) fn written(&self) {
        let tips = &mut *self.tips();
        tips.stored.clone_from(&tips.next);
    }

    pub(super) fn set_bitcoind_sync_state(&self, bitcoind: SyncState) {
        self.tips().bitcoind = Some(bitcoind);
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::BlockHash;
    use bitcoin::hashes::Hash;

    use super::*;

    fn block(height: u64) -> BlockHeader {
        let mut hash = [0; 32];
        hash[..8].copy_from_slice(&height.to_be_bytes());
        BlockHeader {
            hash: BlockHash::from_byte_array(hash),
            height,
        }
    }

    #[test]
    fn while_a_block_is_stored_the_tip_below_counts_and_the_stored_tip_is_not_known() {
        let chain_tip = ChainTip::at(block(100));
        assert_eq!(chain_tip.record(), Some(block(100)));
        assert_eq!(chain_tip.stored(), Some(block(100)));

        chain_tip.writing(Some(block(101)));
        assert_eq!(chain_tip.record(), Some(block(100)));
        assert_eq!(chain_tip.stored(), None);

        chain_tip.written();
        assert_eq!(chain_tip.record(), Some(block(101)));
        assert_eq!(chain_tip.stored(), Some(block(101)));
    }

    #[test]
    fn while_a_block_is_undone_the_tip_below_counts_and_the_stored_tip_is_not_known() {
        let chain_tip = ChainTip::at(block(101));

        chain_tip.writing(Some(block(100)));
        assert_eq!(chain_tip.record(), Some(block(100)));
        assert_eq!(chain_tip.stored(), None);

        chain_tip.written();
        assert_eq!(chain_tip.record(), Some(block(100)));
        assert_eq!(chain_tip.stored(), Some(block(100)));
    }

    #[test]
    fn after_a_write_that_did_not_land_the_stored_tip_is_known_once_it_is_read_again() {
        let chain_tip = ChainTip::at(block(101));
        chain_tip.writing(Some(block(100)));

        // The monitor failed to undo block 101, and reads it from the database
        // at its next sync.
        chain_tip.stored_is(Some(block(101)));

        assert_eq!(chain_tip.record(), Some(block(101)));
        assert_eq!(chain_tip.stored(), Some(block(101)));
    }

    #[test]
    fn a_record_without_a_block_has_no_tip_until_its_first_block_has_landed() {
        let chain_tip = ChainTip::default();
        assert_eq!(chain_tip.record(), None);
        assert_eq!(chain_tip.stored(), None);

        chain_tip.writing(Some(block(100)));
        assert_eq!(chain_tip.record(), None);
        assert_eq!(chain_tip.stored(), None);

        chain_tip.written();
        assert_eq!(chain_tip.record(), Some(block(100)));
        assert_eq!(chain_tip.stored(), Some(block(100)));
    }
}
