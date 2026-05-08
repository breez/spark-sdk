use std::collections::HashMap;

use platform_utils::tokio::sync::RwLock;
use spark_wallet::{HeaderProvider, HeaderProviderError};

const PARTNER_ID_HEADER: &str = "x-partner-jwt";

/// Header provider that injects the Breez partner JWT (`x-partner-jwt`) into
/// outgoing SSP and SO requests. The underlying token is fetched and refreshed
/// by the SDK's sync loop; this provider is just a typed handle around the
/// shared cache.
pub struct BreezPartnerHeaderProvider {
    token: RwLock<Option<String>>,
}

impl BreezPartnerHeaderProvider {
    pub(crate) fn new() -> Self {
        Self {
            token: RwLock::new(None),
        }
    }

    pub(crate) async fn get_token(&self) -> Option<String> {
        self.token.read().await.clone()
    }

    pub(crate) async fn set_token(&self, new_token: String) {
        *self.token.write().await = Some(new_token);
    }
}

impl Default for BreezPartnerHeaderProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[macros::async_trait]
impl HeaderProvider for BreezPartnerHeaderProvider {
    async fn headers(&self) -> Result<HashMap<String, String>, HeaderProviderError> {
        match self.token.read().await.as_ref() {
            Some(token) => Ok(HashMap::from([(
                PARTNER_ID_HEADER.to_string(),
                token.clone(),
            )])),
            None => Ok(HashMap::new()),
        }
    }
}
