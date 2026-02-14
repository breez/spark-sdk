use std::sync::Arc;

use anyhow::Result;
use breez_sdk_common::sync::{
    BreezSyncerClient, SetLockParams, SigningClient, SyncSigner, SyncerClient,
};
use breez_sdk_itest::DataSyncFixture;
use rstest::*;
use tracing::info;
use uuid::Uuid;

const LOCK_NAME: &str = "test_lock";

// ---------------------
// Test SyncSigner
// ---------------------

/// A lightweight `SyncSigner` for testing that signs with a raw secp256k1 secret key.
/// Uses the same double-SHA256 + recoverable ECDSA format as `RTSyncSigner`.
struct TestSyncSigner {
    secret_key: bitcoin::secp256k1::SecretKey,
}

impl TestSyncSigner {
    fn new(secret_bytes: &[u8; 32]) -> Self {
        let secret_key =
            bitcoin::secp256k1::SecretKey::from_slice(secret_bytes).expect("valid secret key");
        Self { secret_key }
    }
}

#[macros::async_trait]
impl SyncSigner for TestSyncSigner {
    async fn sign_ecdsa_recoverable(&self, data: &[u8]) -> anyhow::Result<Vec<u8>> {
        use bitcoin::hashes::{Hash, sha256};
        use bitcoin::secp256k1::{Message, Secp256k1};

        let secp = Secp256k1::new();
        let hash = sha256::Hash::hash(sha256::Hash::hash(data).as_ref());
        let message = Message::from_digest(hash.to_byte_array());
        let sig = secp.sign_ecdsa_recoverable(&message, &self.secret_key);

        let (recovery_id, sig_bytes) = sig.serialize_compact();
        let mut complete_signature = vec![
            31u8.saturating_add(u8::try_from(recovery_id.to_i32()).expect("valid recovery id")),
        ];
        complete_signature.extend_from_slice(&sig_bytes);
        Ok(complete_signature)
    }

    async fn encrypt_ecies(&self, msg: Vec<u8>) -> anyhow::Result<Vec<u8>> {
        // Not needed for lock tests
        Ok(msg)
    }

    async fn decrypt_ecies(&self, msg: Vec<u8>) -> anyhow::Result<Vec<u8>> {
        // Not needed for lock tests
        Ok(msg)
    }
}

// ---------------------
// Fixtures
// ---------------------

#[fixture]
async fn data_sync_fixture() -> DataSyncFixture {
    DataSyncFixture::new()
        .await
        .expect("Failed to start DataSync service")
}

// ---------------------
// Helpers
// ---------------------

/// Creates a `SigningClient` connected to the data-sync service.
/// All clients using the same `signer_key` represent the same user.
fn create_signing_client(
    sync_client: Arc<dyn SyncerClient>,
    signer_key: &[u8; 32],
) -> SigningClient {
    let signer: Arc<dyn SyncSigner> = Arc::new(TestSyncSigner::new(signer_key));
    let client_id = Uuid::now_v7().to_string();
    SigningClient::new(sync_client, signer, client_id)
}

fn lock_params(acquire: bool, exclusive: bool) -> SetLockParams {
    SetLockParams {
        lock_name: LOCK_NAME.to_string(),
        acquire,
        exclusive,
    }
}

// ---------------------
// Tests
// ---------------------

/// Test basic lock acquire, check, and release between two instances.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_distributed_lock_acquire_and_release(
    #[future] data_sync_fixture: DataSyncFixture,
) -> Result<()> {
    info!("=== Starting test_distributed_lock_acquire_and_release ===");

    let data_sync = data_sync_fixture.await;
    let grpc_url = data_sync.grpc_url();

    // Same signer key = same user (two instances of the same wallet)
    let signer_key: [u8; 32] = [1u8; 32];

    let sync_client: Arc<dyn SyncerClient> = Arc::new(BreezSyncerClient::new(grpc_url, None)?);

    let instance_a = create_signing_client(Arc::clone(&sync_client), &signer_key);
    let instance_b = create_signing_client(Arc::clone(&sync_client), &signer_key);

    // Initially, lock should not be held
    let locked = instance_b.get_lock(LOCK_NAME).await?;
    assert!(!locked, "Lock should not be held initially");

    // Instance A acquires the lock
    instance_a.set_lock(lock_params(true, false)).await?;
    info!("Instance A acquired lock");

    // Instance B should see the lock is held
    let locked = instance_b.get_lock(LOCK_NAME).await?;
    assert!(locked, "Lock should be held after Instance A acquired it");

    // Instance A releases the lock
    instance_a.set_lock(lock_params(false, false)).await?;
    info!("Instance A released lock");

    // Instance B should see the lock is no longer held
    let locked = instance_b.get_lock(LOCK_NAME).await?;
    assert!(
        !locked,
        "Lock should not be held after Instance A released it"
    );

    info!("=== Test test_distributed_lock_acquire_and_release PASSED ===");
    Ok(())
}

/// Test that multiple instances can hold locks and the lock remains
/// active until all instances release.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_distributed_lock_multiple_instances(
    #[future] data_sync_fixture: DataSyncFixture,
) -> Result<()> {
    info!("=== Starting test_distributed_lock_multiple_instances ===");

    let data_sync = data_sync_fixture.await;
    let grpc_url = data_sync.grpc_url();

    let signer_key: [u8; 32] = [2u8; 32];

    let sync_client: Arc<dyn SyncerClient> = Arc::new(BreezSyncerClient::new(grpc_url, None)?);

    let instance_a = create_signing_client(Arc::clone(&sync_client), &signer_key);
    let instance_b = create_signing_client(Arc::clone(&sync_client), &signer_key);

    // Both instances acquire the lock
    instance_a.set_lock(lock_params(true, false)).await?;
    instance_b.set_lock(lock_params(true, false)).await?;
    info!("Both instances acquired lock");

    // Lock should be held
    let locked = instance_a.get_lock(LOCK_NAME).await?;
    assert!(locked, "Lock should be held when both instances hold it");

    // Instance A releases
    instance_a.set_lock(lock_params(false, false)).await?;
    info!("Instance A released lock");

    // Lock should still be held (Instance B still has it)
    let locked = instance_a.get_lock(LOCK_NAME).await?;
    assert!(
        locked,
        "Lock should still be held after only Instance A released"
    );

    // Instance B releases
    instance_b.set_lock(lock_params(false, false)).await?;
    info!("Instance B released lock");

    // Lock should no longer be held
    let locked = instance_a.get_lock(LOCK_NAME).await?;
    assert!(
        !locked,
        "Lock should not be held after all instances released"
    );

    info!("=== Test test_distributed_lock_multiple_instances PASSED ===");
    Ok(())
}

/// Test that locks expire after TTL.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_distributed_lock_expiration(
    #[future] data_sync_fixture: DataSyncFixture,
) -> Result<()> {
    info!("=== Starting test_distributed_lock_expiration ===");

    let data_sync = data_sync_fixture.await;
    let grpc_url = data_sync.grpc_url();

    let signer_key: [u8; 32] = [3u8; 32];

    let sync_client: Arc<dyn SyncerClient> = Arc::new(BreezSyncerClient::new(grpc_url, None)?);

    let instance_a = create_signing_client(Arc::clone(&sync_client), &signer_key);
    let instance_b = create_signing_client(Arc::clone(&sync_client), &signer_key);

    // Instance A acquires with default TTL (30s).
    instance_a.set_lock(lock_params(true, false)).await?;

    let locked = instance_b.get_lock(LOCK_NAME).await?;
    assert!(locked, "Lock should be held immediately after acquire");

    // Wait for the lock to expire (default TTL is 30s)
    info!("Waiting 31s for lock to expire...");
    tokio::time::sleep(std::time::Duration::from_secs(31)).await;

    // Lock should have expired without an explicit release
    let locked = instance_b.get_lock(LOCK_NAME).await?;
    assert!(!locked, "Lock should have expired after TTL");

    info!("=== Test test_distributed_lock_expiration PASSED ===");
    Ok(())
}

/// Test that releasing a non-existent lock is a no-op (idempotent).
#[rstest]
#[test_log::test(tokio::test)]
async fn test_distributed_lock_release_idempotent(
    #[future] data_sync_fixture: DataSyncFixture,
) -> Result<()> {
    info!("=== Starting test_distributed_lock_release_idempotent ===");

    let data_sync = data_sync_fixture.await;
    let grpc_url = data_sync.grpc_url();

    let signer_key: [u8; 32] = [4u8; 32];

    let sync_client: Arc<dyn SyncerClient> = Arc::new(BreezSyncerClient::new(grpc_url, None)?);

    let instance_a = create_signing_client(Arc::clone(&sync_client), &signer_key);

    // Release a lock that was never acquired — should succeed
    instance_a.set_lock(lock_params(false, false)).await?;
    info!("Released non-existent lock successfully (idempotent)");

    // Verify it's not locked
    let locked = instance_a.get_lock(LOCK_NAME).await?;
    assert!(!locked, "Lock should not be held");

    info!("=== Test test_distributed_lock_release_idempotent PASSED ===");
    Ok(())
}

/// Test that different users have independent locks.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_distributed_lock_different_users(
    #[future] data_sync_fixture: DataSyncFixture,
) -> Result<()> {
    info!("=== Starting test_distributed_lock_different_users ===");

    let data_sync = data_sync_fixture.await;
    let grpc_url = data_sync.grpc_url();

    // Different signer keys = different users
    let user_a_key: [u8; 32] = [5u8; 32];
    let user_b_key: [u8; 32] = [6u8; 32];

    let sync_client: Arc<dyn SyncerClient> = Arc::new(BreezSyncerClient::new(grpc_url, None)?);

    let user_a = create_signing_client(Arc::clone(&sync_client), &user_a_key);
    let user_b = create_signing_client(Arc::clone(&sync_client), &user_b_key);

    // User A acquires the lock
    user_a.set_lock(lock_params(true, false)).await?;
    info!("User A acquired lock");

    // User A should see it locked
    let locked = user_a.get_lock(LOCK_NAME).await?;
    assert!(locked, "User A should see the lock as held");

    // User B should NOT see User A's lock (different user)
    let locked = user_b.get_lock(LOCK_NAME).await?;
    assert!(
        !locked,
        "User B should not see User A's lock (different user)"
    );

    // Clean up
    user_a.set_lock(lock_params(false, false)).await?;

    info!("=== Test test_distributed_lock_different_users PASSED ===");
    Ok(())
}

/// Test that exclusive lock fails when another instance holds the lock.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_distributed_lock_exclusive(
    #[future] data_sync_fixture: DataSyncFixture,
) -> Result<()> {
    info!("=== Starting test_distributed_lock_exclusive ===");

    let data_sync = data_sync_fixture.await;
    let grpc_url = data_sync.grpc_url();

    let signer_key: [u8; 32] = [7u8; 32];

    let sync_client: Arc<dyn SyncerClient> = Arc::new(BreezSyncerClient::new(grpc_url, None)?);

    let instance_a = create_signing_client(Arc::clone(&sync_client), &signer_key);
    let instance_b = create_signing_client(Arc::clone(&sync_client), &signer_key);

    // Instance A acquires non-exclusive lock
    instance_a.set_lock(lock_params(true, false)).await?;
    info!("Instance A acquired non-exclusive lock");

    // Instance B tries exclusive acquire — should fail
    let result = instance_b.set_lock(lock_params(true, true)).await;
    assert!(
        result.is_err(),
        "Exclusive lock should fail when another instance holds the lock"
    );
    info!("Instance B exclusive acquire correctly failed");

    // Release Instance A
    instance_a.set_lock(lock_params(false, false)).await?;

    // Instance B tries exclusive acquire again — should succeed
    instance_b.set_lock(lock_params(true, true)).await?;
    info!("Instance B exclusive acquire succeeded after A released");

    // Instance A tries non-exclusive acquire while B holds exclusive — should fail
    let result = instance_a.set_lock(lock_params(true, false)).await;
    assert!(
        result.is_err(),
        "Non-exclusive lock should fail when another instance holds an exclusive lock"
    );
    info!("Instance A non-exclusive acquire correctly failed (B holds exclusive)");

    // Release Instance B
    instance_b.set_lock(lock_params(false, true)).await?;

    // Instance A can now acquire non-exclusive
    instance_a.set_lock(lock_params(true, false)).await?;
    info!("Instance A non-exclusive acquire succeeded after B released exclusive");

    // Clean up
    instance_a.set_lock(lock_params(false, false)).await?;

    info!("=== Test test_distributed_lock_exclusive PASSED ===");
    Ok(())
}
