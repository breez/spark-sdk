// HTTP client abstraction for flashnet
// Uses bitreq on native, reqwest on WASM

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
mod native;

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
mod wasm;

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub use native::*;

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
pub use wasm::*;
