//! SDK-wide SOCKS5 proxy: traffic reaches the network through a real proxy,
//! and never around it.

use anyhow::Result;
use breez_sdk_itest::fixtures::socks5::Socks5Proxy;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rand::RngCore;
use rstest::*;
use tempfile::Builder;
use tracing::info;

fn random_seed() -> [u8; 32] {
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);
    seed
}

async fn build_proxied_sdk(prefix: &str, proxy: Option<ProxyConfig>) -> Result<SdkInstance> {
    let dir = Builder::new().prefix(prefix).tempdir()?;
    let mut config = default_config(Network::Regtest);
    config.proxy = proxy;
    build_sdk_with_custom_config(
        dir.path().to_string_lossy().to_string(),
        random_seed(),
        config,
        Some(dir),
        true,
    )
    .await
}

/// Issues a Lightning invoice, which cannot be answered from local state: it
/// needs the operators over gRPC and the SSP over HTTP.
async fn request_invoice(sdk: &SdkInstance) -> Result<String> {
    Ok(sdk
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::Bolt11Invoice {
                description: "proxy itest".to_string(),
                amount_sats: Some(1_000),
                expiry_secs: None,
                payment_hash: None,
                receiver_identity_public_key: None,
            },
        })
        .await?
        .payment_request)
}

/// Issuing a Lightning invoice needs both the operator gRPC channels and the
/// SSP over HTTP, so a success here means the SOCKS5 handshake works on both
/// transports.
#[rstest]
#[test_log::test(tokio::test)]
async fn proxied_sdk_reaches_the_network() -> Result<()> {
    let proxy = Socks5Proxy::start().await?;
    info!("SOCKS5 proxy listening on {}", proxy.config().port);

    let sdk = build_proxied_sdk("breez-sdk-proxy", Some(proxy.config())).await?;

    let invoice = request_invoice(&sdk).await?;
    info!("issued an invoice through the proxy: {invoice}");

    // The status API is a plain HTTPS call made without an SDK instance, so it
    // carries the proxy on its own request.
    get_spark_status(GetSparkStatusRequest {
        proxy: Some(proxy.config()),
    })
    .await?;

    sdk.sdk.disconnect().await?;
    Ok(())
}

/// The same, through a proxy that demands credentials, so the RFC 1929
/// exchange is covered too.
#[rstest]
#[test_log::test(tokio::test)]
async fn proxied_sdk_authenticates_to_the_proxy() -> Result<()> {
    let proxy = Socks5Proxy::start_with_credentials("breez", "hunter2").await?;

    let sdk = build_proxied_sdk("breez-sdk-proxy-auth", Some(proxy.config())).await?;

    request_invoice(&sdk).await?;

    sdk.sdk.disconnect().await?;
    Ok(())
}

/// The test that actually pins "fails closed": with the proxy pointed at a port
/// nothing listens on, every network operation must fail. A success would mean
/// the SDK reached the network directly, which is the leak this exists to
/// prevent.
///
/// Deliberately probes operations that surface transport errors. `get_info`
/// and `connect` would not do: both fall back to locally stored state by
/// design, so they succeed whether or not anything reached the network.
///
/// Needs no proxy container, so it runs anywhere the rest of the suite does.
#[rstest]
#[test_log::test(tokio::test)]
async fn unreachable_proxy_never_falls_back_to_a_direct_connection() -> Result<()> {
    let dead_proxy = ProxyConfig {
        host: "127.0.0.1".to_string(),
        // Port 1 is reserved and never bound by the test environment.
        port: 1,
        username: None,
        password: None,
    };

    // HTTP path, with no SDK involved.
    let status = get_spark_status(GetSparkStatusRequest {
        proxy: Some(dead_proxy.clone()),
    })
    .await;
    assert!(
        status.is_err(),
        "the status API answered through an unreachable proxy, so it went direct"
    );

    // Connecting succeeds: startup sync failures are logged, not fatal, and the
    // SDK serves stored state instead. So the guarantee has to be pinned on an
    // operation that genuinely needs the network, below.
    let sdk = build_proxied_sdk("breez-sdk-proxy-dead", Some(dead_proxy)).await?;

    // Operator gRPC plus SSP HTTP, both of which propagate their errors.
    let invoice = request_invoice(&sdk).await;
    assert!(
        invoice.is_err(),
        "issued an invoice through an unreachable proxy, so traffic went direct"
    );

    sdk.sdk.disconnect().await?;
    Ok(())
}
