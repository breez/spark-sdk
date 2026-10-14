//! Flashnet Rewards API client.
//!
//! USDB holders earn daily BTC rewards, and an endorsement directs a share of
//! them to a pubkey. Requests carry no auth header: each change is signed with
//! the endorser's identity key over a canonical YAML payload.

pub mod client;
pub mod models;

pub use client::RewardsClient;
pub use models::*;

/// The Rewards API serves mainnet only.
pub const REWARDS_MAINNET_BASE_URL: &str = "https://rewards.flashnet.xyz";
