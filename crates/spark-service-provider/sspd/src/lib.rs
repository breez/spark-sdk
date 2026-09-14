#![cfg_attr(
    not(test),
    warn(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        clippy::string_slice,
        clippy::todo,
        clippy::unimplemented,
        clippy::unreachable,
        clippy::unwrap_used
    )
)]

pub mod auth;
pub mod bitcoind;
pub mod chain;
pub mod coop_exit;
pub mod fees;
pub mod graphql;
pub mod handover;
pub mod leaves;
pub mod lightning;
pub mod operator_rpc;
pub mod pool;
pub mod postgresql;
pub mod shutdown;
pub mod swap;
pub mod tree;
pub mod wakeup;
pub mod wallet;
