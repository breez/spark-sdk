//! Adapter exposing the SDK [`Storage`] as a [`boltz_client::BoltzStorage`].
//!
//! Boltz rows live in the provider-agnostic `cross_chain_swaps` table under
//! `provider = "boltz"`.
//!
//! boltz-client runs in seedless mode: each swap carries its own random preimage
//! and gas/claim key under `key_source` ([`boltz_client::models::SwapKeySource::Stored`]).
//! Those secrets are money-critical, so before a swap row reaches local storage
//! this adapter lifts `key_source` out of the swap JSON, ECIES-encrypts it via
//! the SDK signer, and persists only the ciphertext in
//! [`StoredCrossChainSwap::secrets`]. The rest of the swap is stored as
//! plaintext JSON in [`StoredCrossChainSwap::data`]. On read the ciphertext is
//! decrypted and `key_source` spliced back, reconstructing the full
//! [`BoltzSwap`].
//!
//! The encryption key is derived from the wallet identity at a fixed path (see
//! [`BOLTZ_SECRETS_ENCRYPTION_PATH`]), so it is deterministic across every
//! instance restored from the same mnemonic. That is what lets a second instance
//! decrypt a swap synced to it and drive it to terminal (see the realtime-sync
//! `CrossChainSwap` record). Correctness and cross-instance convergence are
//! boltz-client's job: the SDK just persists and syncs rows with plain
//! last-writer-wins.
//!
//! Terminal swap rows are retained (only excluded from `list_active_swaps`),
//! keeping a completed swap queryable by id for the wallet's lifetime.

use std::sync::Arc;

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use bitcoin::bip32::DerivationPath;
use boltz_client::{BoltzError, BoltzStorage, models::BoltzSwap};
use tracing::warn;

use crate::{Storage, persist::StoredCrossChainSwap, signer::EciesSigner};

/// Provider tag this adapter writes into `StoredCrossChainSwap::provider`.
pub(crate) const PROVIDER_TAG_BOLTZ: &str = "boltz";

/// JSON key under which a swap's secrets live before they are lifted out.
const KEY_SOURCE_FIELD: &str = "key_source";

/// Hardened derivation path for the at-rest encryption of Boltz swap secrets.
/// `1112493140` == ASCII "BOLT", distinct from the other SDK subsystems deriving
/// from the same identity master key. The derived key is deterministic per
/// mnemonic+network (the signer's master is derived under a network-specific
/// account), so every instance of the same wallet encrypts and decrypts swap
/// secrets under the same key: a swap synced to a second instance is decryptable
/// there. Never change it: altering the path makes every existing encrypted
/// `secrets` blob undecryptable.
const BOLTZ_SECRETS_ENCRYPTION_PATH: &str = "m/1112493140'/0'/0'/0/0";

/// Bridge between breez-sdk [`Storage`] and [`boltz_client::BoltzStorage`].
pub(crate) struct BoltzStorageAdapter {
    storage: Arc<dyn Storage>,
    ecies: Arc<dyn EciesSigner>,
    encryption_path: DerivationPath,
}

impl BoltzStorageAdapter {
    pub(crate) fn new(
        storage: Arc<dyn Storage>,
        ecies: Arc<dyn EciesSigner>,
    ) -> Result<Self, BoltzError> {
        let encryption_path: DerivationPath = BOLTZ_SECRETS_ENCRYPTION_PATH
            .parse()
            .map_err(|e| BoltzError::Store(format!("Invalid Boltz secrets path: {e}")))?;
        Ok(Self {
            storage,
            ecies,
            encryption_path,
        })
    }

    /// Lifts `key_source` out of the swap JSON, encrypts it, and assembles the
    /// persisted row.
    async fn to_stored(&self, swap: &BoltzSwap) -> Result<StoredCrossChainSwap, BoltzError> {
        let mut value = serde_json::to_value(swap)
            .map_err(|e| BoltzError::Store(format!("Failed to serialize swap: {e}")))?;
        let key_source = value
            .as_object_mut()
            .and_then(|map| map.remove(KEY_SOURCE_FIELD))
            .ok_or_else(|| BoltzError::Store("Swap JSON missing key_source".to_string()))?;
        let plaintext = serde_json::to_vec(&key_source)
            .map_err(|e| BoltzError::Store(format!("Failed to serialize swap secrets: {e}")))?;
        let ciphertext = self
            .ecies
            .encrypt_ecies(&plaintext, &self.encryption_path)
            .await
            .map_err(|e| BoltzError::Store(format!("Failed to encrypt swap secrets: {e}")))?;
        let data = serde_json::to_string(&value)
            .map_err(|e| BoltzError::Store(format!("Failed to serialize swap data: {e}")))?;
        Ok(StoredCrossChainSwap {
            provider: PROVIDER_TAG_BOLTZ.to_string(),
            id: swap.id.clone(),
            is_terminal: swap.status.is_terminal(),
            updated_at: swap.updated_at,
            data,
            secrets: BASE64.encode(ciphertext),
        })
    }

    /// Decrypts the secrets, splices `key_source` back into the data JSON, and
    /// deserializes the full swap.
    async fn swap_from_stored(
        &self,
        stored: StoredCrossChainSwap,
    ) -> Result<BoltzSwap, BoltzError> {
        let ciphertext = BASE64
            .decode(stored.secrets.as_bytes())
            .map_err(|e| BoltzError::Store(format!("Invalid base64 swap secrets: {e}")))?;
        let plaintext = self
            .ecies
            .decrypt_ecies(&ciphertext, &self.encryption_path)
            .await
            .map_err(|e| BoltzError::Store(format!("Failed to decrypt swap secrets: {e}")))?;
        let key_source: serde_json::Value = serde_json::from_slice(&plaintext)
            .map_err(|e| BoltzError::Store(format!("Failed to deserialize swap secrets: {e}")))?;
        let mut value: serde_json::Value = serde_json::from_str(&stored.data)
            .map_err(|e| BoltzError::Store(format!("Failed to deserialize swap data: {e}")))?;
        value
            .as_object_mut()
            .ok_or_else(|| BoltzError::Store("Stored swap data is not a JSON object".to_string()))?
            .insert(KEY_SOURCE_FIELD.to_string(), key_source);
        serde_json::from_value(value)
            .map_err(|e| BoltzError::Store(format!("Failed to deserialize swap: {e}")))
    }
}

#[macros::async_trait]
impl BoltzStorage for BoltzStorageAdapter {
    async fn upsert_swap(&self, swap: &BoltzSwap) -> Result<(), BoltzError> {
        let stored = self.to_stored(swap).await?;
        self.storage
            .set_cross_chain_swap(stored)
            .await
            .map_err(|e| BoltzError::Store(format!("Failed to persist swap row: {e}")))
    }

    async fn get_swap(&self, id: &str) -> Result<Option<BoltzSwap>, BoltzError> {
        let stored = self
            .storage
            .get_cross_chain_swap(PROVIDER_TAG_BOLTZ.to_string(), id.to_string())
            .await
            .map_err(|e| BoltzError::Store(format!("Failed to read swap row: {e}")))?;
        match stored {
            Some(stored) => Ok(Some(self.swap_from_stored(stored).await?)),
            None => Ok(None),
        }
    }

    async fn list_active_swaps(&self) -> Result<Vec<BoltzSwap>, BoltzError> {
        let rows = self
            .storage
            .list_active_cross_chain_swaps(PROVIDER_TAG_BOLTZ.to_string())
            .await
            .map_err(|e| BoltzError::Store(format!("Failed to list active swaps: {e}")))?;
        let mut swaps = Vec::with_capacity(rows.len());
        for stored in rows {
            let id = stored.id.clone();
            // Skip row on error (bad base64/json, decryption).
            match self.swap_from_stored(stored).await {
                Ok(swap) => swaps.push(swap),
                Err(e) => warn!("Skipping Boltz swap '{id}': failed to load from storage: {e}"),
            }
        }
        Ok(swaps)
    }
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use bitcoin::Network;
    use bitcoin::bip32::Xpriv;
    use boltz_client::models::{Asset, BoltzSwap, BoltzSwapStatus, BridgeKind, SwapKeySource};

    use super::*;
    use crate::persist::sqlite::SqliteStorage;
    use crate::signer::breez::BreezSignerImpl;

    fn create_temp_dir(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("breez-test-{}-{}", name, uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    // The seedless `SwapKeySource::Stored` variant wraps `SwapSecrets`, whose
    // fields are crate-private to boltz-client and so not constructible here.
    // `Derived` exercises the same lift/encrypt/splice path (it is key-source
    // agnostic), so the round-trip is covered regardless.
    fn make_swap(id: &str, status: BoltzSwapStatus) -> BoltzSwap {
        BoltzSwap {
            id: id.to_string(),
            status,
            bridge_kind: BridgeKind::Oft,
            key_source: SwapKeySource::Derived { claim_key_index: 7 },
            chain_id: 42161,
            claim_address: "0xclaim".to_string(),
            destination_address: "0xdest".to_string(),
            destination_chain: "Arbitrum One".to_string(),
            asset: Asset::Usdt,
            refund_address: "0xrefund".to_string(),
            erc20swap_address: "0xswap".to_string(),
            router_address: "0xrouter".to_string(),
            invoice: "lnbc1000n".to_string(),
            invoice_amount_sats: 100_000,
            onchain_amount: 99_500,
            expected_output_amount: 71_000_000,
            slippage_bps: 100,
            timeout_block_height: 123_456,
            lockup_tx_id: None,
            claim_tx_hash: None,
            pending_call_id: None,
            delivered_amount: None,
            bridge_ref: None,
            created_at: 1_700_000_000,
            updated_at: 1_700_000_000,
        }
    }

    fn make_adapter() -> (BoltzStorageAdapter, Arc<dyn Storage>) {
        let dir = create_temp_dir("boltz_storage_adapter");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&dir).unwrap());
        let master = Xpriv::new_master(Network::Regtest, &[7u8; 32]).unwrap();
        let ecies: Arc<dyn EciesSigner> = Arc::new(BreezSignerImpl::new(master));
        let adapter = BoltzStorageAdapter::new(Arc::clone(&storage), ecies).unwrap();
        (adapter, storage)
    }

    #[tokio::test]
    async fn upsert_get_list_active_roundtrip() {
        let (adapter, _storage) = make_adapter();
        let swap = make_swap("s1", BoltzSwapStatus::Created);
        adapter.upsert_swap(&swap).await.unwrap();

        let fetched = adapter.get_swap("s1").await.unwrap().unwrap();
        assert_eq!(fetched.id, "s1");
        assert_eq!(fetched.status, BoltzSwapStatus::Created);
        match fetched.key_source {
            SwapKeySource::Derived { claim_key_index } => assert_eq!(claim_key_index, 7),
            SwapKeySource::Stored(_) => panic!("key_source variant must round-trip"),
        }

        let active = adapter.list_active_swaps().await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "s1");
    }

    #[tokio::test]
    async fn secrets_are_encrypted_and_absent_from_plaintext_data() {
        let (adapter, storage) = make_adapter();
        adapter
            .upsert_swap(&make_swap("s1", BoltzSwapStatus::Created))
            .await
            .unwrap();

        let stored = storage
            .get_cross_chain_swap(PROVIDER_TAG_BOLTZ.to_string(), "s1".to_string())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.provider, PROVIDER_TAG_BOLTZ);
        // key_source was lifted out of the plaintext data.
        assert!(
            !stored.data.contains("key_source"),
            "key_source must not remain in plaintext data"
        );
        // The encrypted blob does not reveal the claim_key_index in cleartext.
        assert!(!stored.secrets.is_empty());
        let ciphertext = BASE64.decode(stored.secrets.as_bytes()).unwrap();
        assert!(!ciphertext.is_empty());
    }

    #[tokio::test]
    async fn terminal_swaps_excluded_from_active() {
        let (adapter, _storage) = make_adapter();
        adapter
            .upsert_swap(&make_swap("s1", BoltzSwapStatus::Created))
            .await
            .unwrap();
        adapter
            .upsert_swap(&make_swap("s2", BoltzSwapStatus::Created))
            .await
            .unwrap();

        // Drive s1 terminal; it leaves the active set but stays queryable by id.
        adapter
            .upsert_swap(&make_swap("s1", BoltzSwapStatus::Completed))
            .await
            .unwrap();

        let active = adapter.list_active_swaps().await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "s2");

        let fetched = adapter.get_swap("s1").await.unwrap().unwrap();
        assert_eq!(fetched.status, BoltzSwapStatus::Completed);
    }
}
