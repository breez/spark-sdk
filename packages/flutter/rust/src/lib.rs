pub mod errors;
pub mod events;
mod frb_generated;
pub mod issuer;
pub mod logger;
pub mod models;
pub mod sdk;
pub mod sdk_builder;
pub mod seedless_restore;

pub use sdk::BreezSdk;
pub use seedless_restore::SeedlessRestore;

// Re-export PasskeyPrfProvider trait for flutter_rust_bridge generated code
pub use breez_sdk_spark::seedless_restore::PasskeyPrfProvider;
