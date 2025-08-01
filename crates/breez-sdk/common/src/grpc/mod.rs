#[allow(clippy::doc_markdown)]
mod breez {
    tonic::include_proto!("breez");
}

#[cfg_attr(
    all(target_family = "wasm", target_os = "unknown"),
    path = "transport_wasm.rs"
)]
pub mod transport;
pub use breez::*;
