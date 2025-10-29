use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rand::RngCore;
use rstest::*;
use tempdir::TempDir;
use tracing::info;

// ---------------------
// Fixtures
// ---------------------

/// Fixture: Alice's SDK with temporary storage
#[fixture]
async fn alice_sdk() -> Result<SdkInstance> {
    let alice_dir = TempDir::new("breez-sdk-alice")?;
    let path = alice_dir.path().to_string_lossy().to_string();

    // Generate random seed for Alice
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);

    info!("Initializing Alice's SDK at: {} with random seed", path);
    build_sdk_with_dir(path, seed, Some(alice_dir)).await
}

/// Fixture: Bob's SDK with temporary storage
#[fixture]
async fn bob_sdk() -> Result<SdkInstance> {
    let bob_dir = TempDir::new("breez-sdk-bob")?;
    let path = bob_dir.path().to_string_lossy().to_string();

    // Generate random seed for Bob
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);

    info!("Initializing Bob's SDK at: {} with random seed", path);
    build_sdk_with_dir(path, seed, Some(bob_dir)).await
}

/// Test 1: Sign and Check Messages with DER encoded signatures
#[rstest]
#[test_log::test(tokio::test)]
async fn test_01_sign_and_check_der(
    #[future] alice_sdk: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_01_sign_and_check_der ===");

    let alice = alice_sdk.await?;
    let bob = bob_sdk.await?;

    let alice_message = "Hello Bob!".to_string();
    let bob_message = "Hello Alice!".to_string();

    let alice_signing_res = alice
        .sdk
        .sign_message(SignMessageRequest {
            message: alice_message.clone(),
            compact: None,
        })
        .await?;
    let bob_signing_res = bob
        .sdk
        .sign_message(SignMessageRequest {
            message: bob_message.clone(),
            compact: None,
        })
        .await?;

    let bob_verify_res = bob
        .sdk
        .check_message(CheckMessageRequest {
            message: alice_message.clone(),
            pubkey: alice_signing_res.pubkey.clone(),
            signature: alice_signing_res.signature.clone(),
        })
        .await?;
    assert!(bob_verify_res.is_valid, "Alice's signature should be valid");

    let bob_verify_res = bob
        .sdk
        .check_message(CheckMessageRequest {
            message: bob_message,
            pubkey: alice_signing_res.pubkey,
            signature: alice_signing_res.signature.clone(),
        })
        .await?;
    assert!(
        !bob_verify_res.is_valid,
        "Alice's signature should be invalid for Bob's message"
    );

    let bob_verify_res = bob
        .sdk
        .check_message(CheckMessageRequest {
            message: alice_message,
            pubkey: bob_signing_res.pubkey,
            signature: alice_signing_res.signature,
        })
        .await?;
    assert!(
        !bob_verify_res.is_valid,
        "Alice's signature should be invalid for Bob's public key"
    );

    info!("=== Test test_01_sign_and_check_der PASSED ===");
    Ok(())
}

/// Test 2: Sign and Check Messages with compact encoded signatures
#[rstest]
#[test_log::test(tokio::test)]
async fn test_02_sign_and_check_compact(
    #[future] alice_sdk: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_02_sign_and_check_compact ===");

    let alice = alice_sdk.await?;
    let bob = bob_sdk.await?;

    let alice_message = "Goodbye Bob!".to_string();
    let bob_message = "Goodbye Alice!".to_string();

    let bob_signing_res = bob
        .sdk
        .sign_message(SignMessageRequest {
            message: bob_message.clone(),
            compact: Some(true),
        })
        .await?;
    let alice_signing_res = alice
        .sdk
        .sign_message(SignMessageRequest {
            message: alice_message.clone(),
            compact: Some(true),
        })
        .await?;

    let alice_verify_res = alice
        .sdk
        .check_message(CheckMessageRequest {
            message: bob_message.clone(),
            pubkey: bob_signing_res.pubkey.clone(),
            signature: bob_signing_res.signature.clone(),
        })
        .await?;
    assert!(alice_verify_res.is_valid, "Bob's signature should be valid");

    let alice_verify_res = alice
        .sdk
        .check_message(CheckMessageRequest {
            message: alice_message,
            pubkey: bob_signing_res.pubkey,
            signature: bob_signing_res.signature.clone(),
        })
        .await?;
    assert!(
        !alice_verify_res.is_valid,
        "Bob's signature should be invalid for Alice's message"
    );

    let alice_verify_res = alice
        .sdk
        .check_message(CheckMessageRequest {
            message: bob_message,
            pubkey: alice_signing_res.pubkey,
            signature: bob_signing_res.signature,
        })
        .await?;
    assert!(
        !alice_verify_res.is_valid,
        "Bob's signature should be invalid for Alice's public key"
    );

    info!("=== Test test_02_sign_and_check_compact PASSED ===");
    Ok(())
}
