pub mod address;
pub mod bitcoin;
pub mod core;
pub mod events;
pub mod operator;
pub mod services;
pub mod session_manager;
pub mod signer;
pub mod ssp;
pub mod tree;
pub mod utils;

pub use core::Network;
pub use frost_secp256k1_tr::Identifier;

#[allow(clippy::doc_markdown)]
pub(crate) mod built_info {
    // The file has been placed there by the build script.
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}

pub(crate) fn default_user_agent() -> String {
    format!(
        "{}/{}",
        crate::built_info::PKG_NAME,
        crate::built_info::GIT_VERSION.unwrap_or(crate::built_info::PKG_VERSION),
    )
}
