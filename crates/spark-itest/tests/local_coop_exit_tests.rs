use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use spark::session_store::InMemorySessionStore;
use spark::signer::{Signer, SparkSigner, SparkSignerAdapter};
use spark::ssp::ServiceProvider;
use spark_itest::fixtures::setup::{TestFixtures, create_test_signer_alice};
use spark_wallet::{RetryConfig, ServiceProviderConfig};
use sspd_lib::coop_exit::coop_exit_fees;
use sspd_lib::fees::MIN_FEE_RATE_SAT_PER_KW;
use tracing::info;

const WITHDRAWAL_ADDRESS: &str = "bcrt1qqxhsts0qkmhkgwvl5j03zw7gry4z56mm993s0z";

struct QuoteFixture {
    _fixtures: TestFixtures,
    ssp: ServiceProvider,
}

async fn setup_quote_fixture() -> Result<QuoteFixture> {
    let fixtures = TestFixtures::new().await?;
    let sspd = fixtures.sspd().await?;
    sspd.wait_for_pool(&fixtures.bitcoind, 1, Duration::from_secs(600))
        .await
        .context("waiting for the daemon to stock its pool")?;

    let ssp_config = ServiceProviderConfig {
        base_url: sspd.base_url.clone(),
        schema_endpoint: Some("graphql/spark/rc".to_string()),
        identity_public_key: sspd.identity_public_key,
        user_agent: Some("spark-itest/0.1.0".to_string()),
        retry_config: RetryConfig::default(),
    };
    let signer: Arc<dyn Signer> = Arc::new(create_test_signer_alice());
    let spark_signer: Arc<dyn SparkSigner> = Arc::new(SparkSignerAdapter::new(signer));
    let ssp = ServiceProvider::new(
        ssp_config,
        spark_signer,
        Arc::new(InMemorySessionStore::default()),
        None,
    )?;

    Ok(QuoteFixture {
        _fixtures: fixtures,
        ssp,
    })
}

/// The daemon sizes a quote from the number of leaf ids alone, so these name no
/// real leaves.
fn leaf_ids(count: usize) -> Vec<String> {
    (0..count)
        .map(|i| format!("00000000-0000-0000-0000-{i:012}"))
        .collect()
}

/// Regtest fee rates are at or below the daemon's floor, so the daemon quotes at
/// the floor rate.
#[tokio::test]
#[test_log::test]
async fn test_coop_exit_quote_matches_the_fee_model() -> Result<()> {
    let fixture = setup_quote_fixture().await?;

    for leaves in [1usize, 2, 5] {
        let quote = fixture
            .ssp
            .get_coop_exit_fee_quote(leaf_ids(leaves), WITHDRAWAL_ADDRESS)
            .await?;
        let expected = coop_exit_fees(leaves as u64, MIN_FEE_RATE_SAT_PER_KW);
        let expected_total = expected
            .l1_broadcast_fee_sats
            .saturating_add(expected.user_fee_sats);

        info!(
            "quote for {leaves} leaves: total {} sats (model says {expected_total})",
            quote.total_amount.original_value
        );
        assert_eq!(
            quote.total_amount.original_value, expected_total,
            "the quote for {leaves} leaves must be the fee the request path charges"
        );
        assert_eq!(
            quote.l1_broadcast_fee_medium.original_value, expected.l1_broadcast_fee_sats,
            "the broadcast component must be the model's"
        );
        assert_eq!(
            quote.user_fee_medium.original_value, expected.user_fee_sats,
            "the service component must be the model's"
        );
    }

    Ok(())
}

#[tokio::test]
#[test_log::test]
async fn test_coop_exit_quote_total_is_its_parts() -> Result<()> {
    let fixture = setup_quote_fixture().await?;
    let quote = fixture
        .ssp
        .get_coop_exit_fee_quote(leaf_ids(3), WITHDRAWAL_ADDRESS)
        .await?;

    assert_eq!(
        quote.total_amount.original_value,
        quote
            .user_fee_medium
            .original_value
            .saturating_add(quote.l1_broadcast_fee_medium.original_value),
        "the total must be the user fee plus the broadcast fee"
    );

    let user_fees = [
        quote.user_fee_fast.original_value,
        quote.user_fee_medium.original_value,
        quote.user_fee_slow.original_value,
    ];
    assert!(
        user_fees.iter().all(|fee| *fee == user_fees[0]),
        "the model is speed-independent, so every speed carries one user fee: {user_fees:?}"
    );
    let broadcast_fees = [
        quote.l1_broadcast_fee_fast.original_value,
        quote.l1_broadcast_fee_medium.original_value,
        quote.l1_broadcast_fee_slow.original_value,
    ];
    assert!(
        broadcast_fees.iter().all(|fee| *fee == broadcast_fees[0]),
        "the model is speed-independent, so every speed carries one broadcast fee: {broadcast_fees:?}"
    );

    assert!(
        quote.expires_at > quote.created_at,
        "a quote that expires when it is issued is one no client can act on"
    );

    Ok(())
}

#[tokio::test]
#[test_log::test]
async fn test_coop_exit_quote_grows_with_the_leaf_count() -> Result<()> {
    let fixture = setup_quote_fixture().await?;

    let mut previous = 0u64;
    for leaves in [1usize, 2, 4, 8] {
        let quote = fixture
            .ssp
            .get_coop_exit_fee_quote(leaf_ids(leaves), WITHDRAWAL_ADDRESS)
            .await?;
        let total = quote.total_amount.original_value;
        info!("quote for {leaves} leaves: {total} sats");
        assert!(
            total > previous,
            "quoting {leaves} leaves at {total} sats is no more than the {previous} sats quoted for fewer"
        );
        previous = total;
    }

    Ok(())
}
