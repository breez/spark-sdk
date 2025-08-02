use anyhow::Result;
use rstest::*;
use spark_wallet::{DefaultSigner, SparkWallet, WalletEvent};
use tracing::info;

use spark_itest::fixtures::setup::{TestFixtures, create_test_signer};

// Setup test fixtures
#[fixture]
async fn fixtures() -> TestFixtures {
    TestFixtures::new()
        .await
        .expect("Failed to initialize test fixtures")
}

pub struct WalletFixture {
    #[allow(dead_code)]
    fixtures: TestFixtures,
    wallet: SparkWallet<DefaultSigner>,
}

// Create a wallet for testing
#[fixture]
async fn wallet(#[future] fixtures: TestFixtures) -> WalletFixture {
    let fixtures = fixtures.await;
    let config = fixtures
        .create_wallet_config()
        .await
        .expect("failed to create wallet config");
    let signer = create_test_signer();

    let wallet = SparkWallet::connect(config, signer)
        .await
        .expect("Failed to connect wallet");

    WalletFixture { fixtures, wallet }
}
// Test creating a deposit address
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_create_deposit_address(#[future] wallet: WalletFixture) -> Result<()> {
    let fixture = wallet.await;
    let wallet = fixture.wallet;

    let mut listener = wallet.subscribe_events();
    loop {
        let event = listener.recv().await?;
        info!("Wallet event: {:?}", event);
        if event == WalletEvent::Synced {
            break;
        }
    }
    let address = wallet.generate_deposit_address(false).await?;
    info!("Generated deposit address: {}", address);

    assert!(
        !address.to_string().is_empty(),
        "Address should not be empty"
    );

    Ok(())
}
