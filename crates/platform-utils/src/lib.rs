/// Platform-specific tokio re-export.
///
/// This module provides a unified way to import tokio across different platforms:
/// - On native platforms: uses the standard tokio runtime
/// - On WASM platforms: uses `tokio_with_wasm` for compatibility
///
/// Usage:
/// ```rust
/// use platform_utils::tokio;
/// ```
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub use tokio;

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
pub use tokio_with_wasm::alias as tokio;
