//! Flashnet Orchestra (cross-chain) API client.
//!
//! Unlike the AMM client in [`crate::amm`], Orchestra uses a static bearer
//! API key (no challenge/verify) and requires an `X-Idempotency-Key` header
//! on mutating requests. The flow is `quote` → deposit → `submit` → `status`.

pub mod client;
pub mod models;

pub use client::OrchestraClient;
pub use models::*;
