use rand::RngCore;
use spark::{Network, bech32m_decode_token_id};

use crate::FlashnetError;

pub(crate) fn generate_nonce() -> [u8; 16] {
    let mut entropy_bytes = [0u8; 16];
    let mut rng = rand::thread_rng();
    rng.fill_bytes(&mut entropy_bytes);
    entropy_bytes
}

pub(crate) fn decode_token_identifier(
    token_id: &str,
    network: Network,
) -> Result<String, FlashnetError> {
    if token_id.starts_with("btkn") {
        return Ok(hex::encode(
            bech32m_decode_token_id(token_id, Some(network))
                .map_err(|e| FlashnetError::Generic(format!("Invalid token identifier: {e}")))?,
        ));
    }

    Ok(token_id.to_string())
}
