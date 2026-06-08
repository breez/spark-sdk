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
}

impl Default for TurnkeyRetryConfig {
    fn default() -> Self {
        // Matches turnkey_client::RetryConfig::default().
        Self {
            initial_delay_ms: 500,
            multiplier: 2.0,
            max_delay_ms: 5_000,
            max_retries: 5,
        }
    }
}

impl TurnkeyRetryConfig {
    /// Exponential backoff delay to wait after `attempt` attempts (1-based),
    /// capped at `max_delay_ms`.
    pub(crate) fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let factor = self.multiplier.powi(attempt.saturating_sub(1) as i32);
        let millis = (self.initial_delay_ms as f64 * factor).min(self.max_delay_ms as f64);
        Duration::from_millis(millis as u64)
    }
}

/// Configuration for the Turnkey-backed signer.
///
/// Assumes the Spark wallet already exists in Turnkey (identity, static-deposit,
/// and encryption accounts provisioned); provisioning is out of scope here. The
/// API keypair authenticates every request and must be a secp256k1 key (Turnkey
/// supports both curves; we use secp256k1 so the SDK needs no extra crypto
/// dependency).
#[derive(Clone, Debug)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TurnkeyConfig {
    /// Turnkey API base URL (e.g. `https://api.turnkey.com`).
    pub base_url: String,
    /// Organization (or sub-organization) id that owns the wallet.
    pub organization_id: String,
    /// secp256k1 API public key (compressed, hex), registered with the organization.
    pub api_public_key: String,
    /// secp256k1 API private key (hex) used to stamp requests.
    pub api_private_key: String,
    /// Id of the Spark wallet to sign with.
    pub wallet_id: String,
    /// Retry policy for Turnkey requests.
    pub retry: TurnkeyRetryConfig,
}
