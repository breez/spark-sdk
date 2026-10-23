use std::{str::FromStr, sync::Arc};

use bitcoin::{
    bip32::DerivationPath,
    hashes::{Hash, HashEngine, Hmac, HmacEngine, sha256},
};
use platform_utils::tokio::sync::OnceCell;

use crate::{SdkError, signer::HmacSigner};

/// Used for nothing else, so the key can't be produced through another feature.
const QUOTE_KEY_PATH: &str = "m/402'/0'";
const QUOTE_KEY_LABEL: &[u8] = b"x402:exact:lnbtc:quote-key:v1";
const MIN_SECRET_LEN: usize = 32;

/// The key x402 quotes are signed with. It is derived from the SDK's signer,
/// so every instance of the same SDK shares it without any configuration,
/// unless a secret is configured instead.
pub(crate) struct X402QuoteKey {
    hmac: Option<Arc<dyn HmacSigner>>,
    secret: Option<Vec<u8>>,
    key: OnceCell<[u8; 32]>,
}

impl X402QuoteKey {
    pub(crate) fn new(hmac: Option<Arc<dyn HmacSigner>>, secret: Option<Vec<u8>>) -> Self {
        Self {
            hmac,
            secret,
            key: OnceCell::new(),
        }
    }

    /// The key, derived on first use: one signer call per SDK instance.
    pub(crate) async fn get(&self) -> Result<[u8; 32], SdkError> {
        self.key
            .get_or_try_init(|| async {
                if let Some(secret) = &self.secret {
                    if secret.len() < MIN_SECRET_LEN {
                        return Err(SdkError::InvalidInput(format!(
                            "the x402 quote secret must be at least {MIN_SECRET_LEN} bytes"
                        )));
                    }
                    let mut engine = HmacEngine::<sha256::Hash>::new(secret);
                    engine.input(QUOTE_KEY_LABEL);
                    return Ok(Hmac::<sha256::Hash>::from_engine(engine).to_byte_array());
                }
                let hmac = self.hmac.as_ref().ok_or_else(|| {
                    SdkError::Generic(
                        "x402 charging needs a signer that supports HMAC, or a quote secret \
                         set with SdkBuilder::with_x402_quote_secret"
                            .to_string(),
                    )
                })?;
                let path = DerivationPath::from_str(QUOTE_KEY_PATH)
                    .map_err(|e| SdkError::Generic(e.to_string()))?;
                Ok(hmac
                    .hmac_sha256(&path, QUOTE_KEY_LABEL)
                    .await?
                    .to_byte_array())
            })
            .await
            .copied()
    }
}
