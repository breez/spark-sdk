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

/// USDB on mainnet, the token whose holders earn the rewards.
pub const USDB_MAINNET_TOKEN_IDENTIFIER: &str =
    "btkn1xgrvjwey5ngcagvap2dzzvsy4uk8ua9x69k82dwvt5e7ef9drm9qztux87";

/// The first line of every endorsement payload signed with the identity key.
pub const ENDORSEMENT_PAYLOAD_PREFIX: &str = "kind: rewards_endorsement";
