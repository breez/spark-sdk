use std::{collections::HashMap, sync::Arc};

use crate::{
    services::{ServiceError, TokenMetadata, TokenOutput},
    signer::Signer,
};

pub struct TokenOutputs {
    pub metadata: TokenMetadata,
    pub outputs: Vec<TokenOutput>,
}

pub struct TokenService<S> {
    tokens_outputs: HashMap<String, TokenOutputs>,
    signer: Arc<S>,
}

impl<S: Signer> TokenService<S> {
    pub fn new(signer: Arc<S>) -> Self {
        Self {
            tokens_outputs: HashMap::new(),
            signer,
        }
    }

    /// Fetches all owned token outputs from the SE and updates the local cache.
    pub async fn refresh_tokens(&self) -> Result<(), ServiceError> {
        // TODO: Implement
        Ok(())
    }

    /// Returns owned token outputs from the local cache.
    pub fn get_tokens_outputs(&self) -> &HashMap<String, TokenOutputs> {
        &self.tokens_outputs
    }
}
