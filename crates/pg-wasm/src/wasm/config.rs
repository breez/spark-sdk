//! Connection setup.
//!
//! Mirrors `tokio_postgres::connect` — but doesn't return a separate
//! `connection` future, because I/O is driven on the JS side by
//! node-postgres's event loop.

use super::client::Client;
use super::error::Error;
use super::js::connect_client;

/// Connection parameters. v0 only supports a libpq-style connection string.
#[derive(Debug, Clone)]
pub struct Config {
    connection_string: String,
}

impl Config {
    #[must_use]
    pub fn new(connection_string: impl Into<String>) -> Self {
        Self {
            connection_string: connection_string.into(),
        }
    }

    /// Open the connection. Returns a [`Client`] tied to a single
    /// `pg.Client` on the JS side.
    pub async fn connect(self) -> Result<Client, Error> {
        let js = connect_client(&self.connection_string)
            .await
            .map_err(Error::from_js)?;
        Ok(Client::new(js))
    }
}

/// Convenience wrapper matching `tokio_postgres::connect`'s signature
/// (minus the `tls` arg and the separate `connection` future, neither of
/// which apply on wasm).
pub async fn connect(connection_string: &str) -> Result<Client, Error> {
    Config::new(connection_string).connect().await
}
