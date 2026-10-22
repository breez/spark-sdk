//! x402 payments over Lightning: the `exact` scheme on `lnbtc`
//! (x402-foundation/x402, `specs/schemes/exact/scheme_exact_lnbtc.md`).

pub mod binding;
pub mod client;
pub mod error;
mod invoice;
mod jcs;
pub mod network;
pub mod types;

#[cfg(test)]
mod tests;
