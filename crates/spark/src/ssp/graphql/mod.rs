mod auth_provider;
mod client;
mod error;
mod fragments;
mod mutations;
mod queries;
pub(crate) mod types;

use std::num::ParseIntError;

pub(crate) use client::GraphQLClient;
pub(crate) use error::GraphQLError;
pub use types::*;

// TODO: handle the case where the currency is not sats
impl CurrencyAmount {
    pub fn as_sats(&self) -> Result<u64, ParseIntError> {
        self.original_value.parse()
    }
}
