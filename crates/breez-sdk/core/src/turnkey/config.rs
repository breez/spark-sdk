use std::time::Duration;

/// Retry policy for Turnkey API requests (used while polling a pending
/// activity). Mirrors `turnkey_client`'s `RetryConfig` with FFI-friendly
/// millisecond fields.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TurnkeyRetryConfig {
    /// Delay before the first retry, in milliseconds.
    pub initial_delay_ms: u64,
    /// Multiplier applied to the delay after each attempt.
    pub multiplier: f64,
    /// Upper bound on the delay between retries, in milliseconds.
    pub max_delay_ms: u64,
    /// Maximum number of retries (0 disables retrying).
    pub max_retries: u32,
    /// Total time budget for one API request including its retries and waits,
    /// in milliseconds. No retry begins past this deadline: when the next wait
    /// (server-requested or backoff) would end after it, the request fails with
    /// the last error instead of stalling.
    pub request_timeout_ms: u64,
}

impl Default for TurnkeyRetryConfig {
    fn default() -> Self {
        // Backoff matches turnkey_client::RetryConfig::default().
        Self {
            initial_delay_ms: 500,
            multiplier: 2.0,
            max_delay_ms: 5_000,
            max_retries: 5,
            request_timeout_ms: 60_000,
        }
    }
}

impl TurnkeyRetryConfig {
    /// Exponential backoff delay to wait after `attempt` attempts (1-based),
    /// capped at `max_delay_ms`.
    // Backoff over small millisecond delays: the float/int casts are bounded
    // and intentional.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    pub(crate) fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let factor = self.multiplier.powf(f64::from(attempt.saturating_sub(1)));
        let millis = (self.initial_delay_ms as f64 * factor).min(self.max_delay_ms as f64);
        Duration::from_millis(millis as u64)
    }

    pub(crate) fn request_timeout(&self) -> Duration {
        Duration::from_millis(self.request_timeout_ms)
    }
}

/// Configuration for the Turnkey-backed signer.
///
/// Assumes the Spark wallet already exists in Turnkey (identity, static-deposit,
/// and encryption accounts provisioned); provisioning is out of scope here. The
/// API keypair authenticates every request: secp256k1 keys are always supported
/// (reusing the SDK's existing crypto dependency), and P-256 keys (Turnkey's
/// console default) when built with the `turnkey-p256` feature. The curve is
/// detected from the key material.
/// The Turnkey API base URL used when [`TurnkeyConfig::base_url`] is unset.
pub(crate) const DEFAULT_BASE_URL: &str = "https://api.turnkey.com";

#[derive(Clone, Debug)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TurnkeyConfig {
    /// Turnkey API base URL. Unset uses `https://api.turnkey.com`.
    pub base_url: Option<String>,
    /// Organization (or sub-organization) id that owns the wallet.
    pub organization_id: String,
    /// API public key (compressed, hex), registered with the organization.
    pub api_public_key: String,
    /// API private key (hex) used to stamp requests.
    pub api_private_key: String,
    /// Id of the Spark wallet to sign with.
    pub wallet_id: String,
    /// Network the wallet operates on; selects the Spark address format
    /// (mainnet or regtest) used for Spark-protocol and Schnorr signing.
    pub network: crate::Network,
    /// Spark account number: the `{account}` in every derivation path
    /// (`m/8797555'/{account}'/...`). Unset uses the network default, matching
    /// the seed-based signer, so the same wallet seed derives the same keys on
    /// either backend.
    pub account_number: Option<u32>,
    /// Retry policy for Turnkey requests. Unset uses the default policy.
    pub retry: Option<TurnkeyRetryConfig>,
}
