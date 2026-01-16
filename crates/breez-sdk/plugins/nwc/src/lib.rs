use std::collections::HashMap;

use crate::error::NwcResult;

pub(crate) mod context;
pub mod error;
pub mod event;
pub(crate) mod handler;
mod persist;
pub(crate) mod sdk_event;

pub use event::{NwcEvent, NwcEventDetails, NwcEventListener};

pub const DEFAULT_RELAY_URLS: [&str; 1] = ["wss://relay.getalbypro.com/breez"];

#[macros::async_trait]
pub trait NwcService: Send + Sync {
    /// Creates a Nostr Wallet Connect connection string for this service.
    ///
    /// Generates a unique connection URI that external applications can use
    /// to connect to this wallet service. The URI includes the wallet's public key,
    /// relay information, and a randomly generated secret for secure communication.
    ///
    /// # Arguments
    /// * `name` - The unique identifier for the connection string
    async fn add_connection_string(&self, name: String) -> NwcResult<String>;

    /// Lists the active Nostr Wallet Connect connections for this service.
    async fn list_connection_strings(&self) -> NwcResult<HashMap<String, String>>;

    /// Removes a Nostr Wallet Connect connection string
    ///
    /// Removes a previously set connection string. Returns error if unset.
    ///
    /// # Arguments
    /// * `name` - The unique identifier for the connection string
    async fn remove_connection_string(&self, name: String) -> NwcResult<()>;

    /// Adds an event listener to the service, where all [NwcEvent]s will be emitted to.
    /// The event listener can be removed be calling [NwcService::remove_event_listener].
    ///
    /// # Arguments
    ///
    /// * `listener` - The listener which is an implementation of the [NwcEventListener] trait
    async fn add_event_listener(&self, listener: Box<dyn NwcEventListener>) -> String;

    /// Removes an event listener from the service
    ///
    /// # Arguments
    ///
    /// * `id` - the event listener id returned by [NwcService::add_event_listener]
    async fn remove_event_listener(&self, id: &str);
}

pub struct NwcConfig {
    pub relay_urls: Option<Vec<String>>,
    pub secret_key_hex: Option<String>,
}

impl NwcConfig {
    pub fn relays(&self) -> Vec<String> {
        self.relay_urls
            .clone()
            .unwrap_or(DEFAULT_RELAY_URLS.iter().map(|s| s.to_string()).collect())
    }
}
