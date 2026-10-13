// Operators and the SSP supply most of the data this crate handles, and a
// release build aborts on panic, so an out-of-range index is not allowed.
#![warn(clippy::indexing_slicing)]

pub mod address;
pub mod bitcoin;
pub mod core;
pub mod events;
pub mod header_provider;
pub mod operator;
pub mod services;
pub mod session_store;
pub mod signer;
pub mod ssp;
pub mod token;
pub mod tree;
pub mod utils;

pub use core::Network;
pub use frost_secp256k1_tr::Identifier;
pub use token::{bech32m_decode_token_id, bech32m_encode_token_id};

/// The `git describe` output for the commit that cargo built this crate from.
/// Unset when cargo built it outside a git checkout or without git installed.
pub const GIT_VERSION: Option<&str> = option_env!("SPARK_GIT_VERSION");

pub(crate) fn default_user_agent() -> String {
    format!(
        "{}/{}",
        env!("CARGO_PKG_NAME"),
        GIT_VERSION.unwrap_or(env!("CARGO_PKG_VERSION")),
    )
}
