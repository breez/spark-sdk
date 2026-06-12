//! Turnkey-backed test harness: per-test wallet provisioning, cleanup, and SDK
//! construction. Compiled only with the `turnkey` feature (gated at the module
//! declaration in `lib.rs`), so everything here can assume the backend exists.

use anyhow::Result;
use breez_sdk_spark::turnkey::{TurnkeyConfig, TurnkeyWalletManager};
use breez_sdk_spark::{Config, GetInfoRequest, Network, SdkBuilder};
use tempfile::TempDir;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::SdkInstance;
use crate::helpers::{ChannelEventListener, apply_storage};

/// Reads the Turnkey test-org config from the environment, returning `None`
/// when the org or API-key variables are unset. The wallet is provisioned per
/// test (see [`build_sdk_with_turnkey`]), so `wallet_id` is left empty here.
pub fn turnkey_config_from_env() -> Option<TurnkeyConfig> {
    let var = |key: &str| std::env::var(key).ok().filter(|v| !v.is_empty());
    Some(TurnkeyConfig {
        base_url: var("TURNKEY_BASE_URL"),
        organization_id: var("TURNKEY_ORG_ID")?,
        api_public_key: var("TURNKEY_API_PUBLIC_KEY")?,
        api_private_key: var("TURNKEY_API_PRIVATE_KEY")?,
        wallet_id: String::new(),
        network: Network::Regtest,
        // Defaults to the network default (0 on regtest); settable to verify
        // non-default accounts against the live API.
        account_number: var("TURNKEY_ACCOUNT_NUMBER").and_then(|v| v.parse().ok()),
        retry: None,
    })
}

/// Name prefix for throwaway per-test Turnkey wallets. The age-gated reaper uses
/// it to find abandoned wallets without ever matching a concurrent runner's
/// freshly created ones.
const TURNKEY_TEST_WALLET_PREFIX: &str = "brz-itest-";
/// The reaper only deletes test wallets older than this. It comfortably exceeds
/// a test run, so a concurrent runner's just-created wallets are never in scope.
const TURNKEY_REAP_MIN_AGE_SECS: u64 = 3600;

/// Runs the age-gated reaper at most once per test-binary execution, shared
/// across the concurrent Turnkey cases.
static TURNKEY_REAP_ONCE: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

/// Deletes abandoned per-test wallets left by panicked runs. Safe under
/// concurrent runners: it only removes `TURNKEY_TEST_WALLET_PREFIX` wallets
/// older than [`TURNKEY_REAP_MIN_AGE_SECS`], so another runner's fresh wallets
/// (seconds old) are never deleted. Best-effort: failures are logged, not fatal.
async fn reap_stale_turnkey_wallets(config: &TurnkeyConfig) {
    let manager = match TurnkeyWalletManager::new(config) {
        Ok(manager) => manager,
        Err(e) => {
            warn!("Turnkey reaper: manager init failed: {e}");
            return;
        }
    };
    let wallets = match manager.list_wallets().await {
        Ok(wallets) => wallets,
        Err(e) => {
            warn!("Turnkey reaper: list_wallets failed: {e}");
            return;
        }
    };
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let stale: Vec<String> = wallets
        .into_iter()
        .filter(|w| w.wallet_name.starts_with(TURNKEY_TEST_WALLET_PREFIX))
        .filter(|w| now_secs.saturating_sub(w.created_at_secs) >= TURNKEY_REAP_MIN_AGE_SECS)
        .map(|w| w.wallet_id)
        .collect();
    if stale.is_empty() {
        return;
    }
    info!(
        "Turnkey reaper: deleting {} abandoned test wallet(s)",
        stale.len()
    );
    if let Err(e) = manager.delete_wallets(stale).await {
        warn!("Turnkey reaper: delete_wallets failed: {e}");
    }
}

/// Deletes a per-test Turnkey wallet when the owning [`SdkInstance`] drops.
///
/// Deletes only the wallet id it created, so it never touches another runner's
/// wallets. The async delete runs on a dedicated thread with its own runtime, so
/// it works regardless of the test's tokio flavor and during panic unwinding; a
/// wallet that still slips through (e.g. a hard abort) is reaped age-gated on a
/// later run.
pub struct TurnkeyWalletGuard {
    config: TurnkeyConfig,
}

impl TurnkeyWalletGuard {
    fn new(config: TurnkeyConfig) -> Self {
        Self { config }
    }
}

impl Drop for TurnkeyWalletGuard {
    fn drop(&mut self) {
        let config = self.config.clone();
        let wallet_id = config.wallet_id.clone();
        let handle = std::thread::spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(e) => {
                    warn!("Turnkey cleanup: runtime build failed for wallet {wallet_id}: {e}");
                    return;
                }
            };
            runtime.block_on(async {
                match TurnkeyWalletManager::new(&config) {
                    Ok(manager) => match manager.delete_wallets(vec![wallet_id.clone()]).await {
                        Ok(()) => info!("Deleted per-test Turnkey wallet {wallet_id}"),
                        Err(e) => warn!("Turnkey cleanup: delete wallet {wallet_id} failed: {e}"),
                    },
                    Err(e) => warn!("Turnkey cleanup: manager init failed: {e}"),
                }
            });
        });
        let _ = handle.join();
    }
}

/// Provisions a fresh throwaway Turnkey wallet and returns the config pointing
/// at it, plus the [`TurnkeyWalletGuard`] that deletes it on drop. Requires the
/// `TURNKEY_*` credentials: building this crate with the `turnkey` feature opts
/// into them.
pub async fn provision_turnkey_wallet()
-> Result<(breez_sdk_spark::turnkey::TurnkeyConfig, TurnkeyWalletGuard)> {
    let Some(mut turnkey_config) = turnkey_config_from_env() else {
        anyhow::bail!(
            "the turnkey feature is enabled but TURNKEY_ORG_ID, \
             TURNKEY_API_PUBLIC_KEY or TURNKEY_API_PRIVATE_KEY is unset"
        );
    };

    // Reap abandoned wallets once per run (age-gated, safe under concurrent
    // runners), then provision a fresh wallet for this instance.
    TURNKEY_REAP_ONCE
        .get_or_init(|| reap_stale_turnkey_wallets(&turnkey_config))
        .await;
    let manager = TurnkeyWalletManager::new(&turnkey_config)
        .map_err(|e| anyhow::anyhow!("Turnkey wallet manager init failed: {e}"))?;
    let wallet_name = format!("{TURNKEY_TEST_WALLET_PREFIX}{:016x}", rand::random::<u64>());
    let wallet_id = manager
        .create_wallet(wallet_name.clone())
        .await
        .map_err(|e| anyhow::anyhow!("Turnkey create_wallet failed: {e}"))?;
    info!("Created per-test Turnkey wallet {wallet_id} ({wallet_name})");
    turnkey_config.wallet_id = wallet_id;
    let guard = TurnkeyWalletGuard::new(turnkey_config.clone());
    Ok((turnkey_config, guard))
}

/// Builds a Regtest SDK backed by the Turnkey signers (`create_turnkey_signer`)
/// on a freshly provisioned throwaway wallet (fully isolating each case, like
/// the seed backend), deleted on teardown via [`TurnkeyWalletGuard`].
pub async fn build_sdk_with_turnkey(
    config: Config,
    storage_dir: String,
    temp_dir: Option<TempDir>,
) -> Result<SdkInstance> {
    let (turnkey_config, guard) = provision_turnkey_wallet().await?;
    let turnkey_guard = Some(guard);

    let signers = breez_sdk_spark::turnkey::create_turnkey_signer(turnkey_config)
        .await
        .map_err(|e| anyhow::anyhow!("create_turnkey_signer failed: {e}"))?;

    let builder = SdkBuilder::new_with_signer(config, signers.breez_signer, signers.spark_signer);
    let builder = apply_storage(builder, storage_dir).await?;
    let sdk = builder.build().await?;

    let (tx, rx) = mpsc::channel(100);
    let event_listener = Box::new(ChannelEventListener { tx });
    let _listener_id = sdk.add_event_listener(event_listener).await;

    let _ = sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(true),
        })
        .await?;

    Ok(SdkInstance {
        sdk,
        events: rx,
        span: tracing::Span::current(),
        temp_dir,
        data_sync_fixture: None,
        lnurl_fixture: None,
        turnkey_guard,
    })
}
