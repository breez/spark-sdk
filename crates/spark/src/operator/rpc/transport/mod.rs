#[cfg_attr(
    all(target_family = "wasm", target_os = "unknown"),
    path = "grpc_client_wasm.rs"
)]
pub mod grpc_client;

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub(crate) mod retry_channel;
