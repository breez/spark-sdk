//! Turnkey signer backend (behind the `turnkey` feature).
//!
//! A minimal cross-platform (native + wasm) Turnkey API client on
//! [`platform_utils::HttpClient`], plus signer implementations backing Spark
//! and SDK-layer signing on Turnkey activities. Uses secp256k1 API keys.
//! Private keys stay in Turnkey except where its design requires a local export
//! (static-deposit refund and the SDK-layer encryption/HMAC key).

mod accounts;
mod breez_signer;
mod config;
mod error;
mod factory;
#[cfg(feature = "test-utils")]
mod management;
mod spark_signer;
mod stamp;
mod transport;
mod types;

pub use config::{TurnkeyConfig, TurnkeyRetryConfig};
pub use error::TurnkeyError;
pub use factory::create_turnkey_signer;
#[cfg(feature = "test-utils")]
pub use management::{TurnkeyWalletInfo, TurnkeyWalletManager};
