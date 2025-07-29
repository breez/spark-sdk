use std::str::FromStr;

use bitcoin::bip32::{ChildNumber, Xpub};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use tracing::{error, warn};

use crate::rest::{RestClient, parse_json};

use super::{
    LnurlCallbackStatus,
    error::{LnurlError, LnurlResult},
};

/// Wrapped in a [`LnurlAuth`], this is the result of [`parse`] when given a LNURL-auth endpoint.
///
/// It represents the endpoint's parameters for the LNURL workflow.
///
/// See <https://github.com/lnurl/luds/blob/luds/04.md>
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LnurlAuthRequestData {
    /// Hex encoded 32 bytes of challenge
    pub k1: String,

    /// When available, one of: register, login, link, auth
    pub action: Option<String>,

    /// Indicates the domain of the LNURL-auth service, to be shown to the user when asking for
    /// auth confirmation, as per LUD-04 spec.
    #[serde(skip_serializing, skip_deserializing)]
    pub domain: String,

    /// Indicates the URL of the LNURL-auth service, including the query arguments. This will be
    /// extended with the signed challenge and the linking key, then called in the second step of the workflow.
    #[serde(skip_serializing, skip_deserializing)]
    pub url: String,
}

#[breez_sdk_macros::async_trait]
pub trait LnurlAuthSigner {
    async fn derive_bip32_pub_key(&self, derivation_path: &[ChildNumber]) -> LnurlResult<Vec<u8>>;
    async fn sign_ecdsa(&self, msg: &[u8], derivation_path: &[ChildNumber])
    -> LnurlResult<Vec<u8>>;
    async fn hmac_sha256(
        &self,
        key_derivation_path: &[ChildNumber],
        input: &[u8],
    ) -> LnurlResult<Vec<u8>>;
}

/// Performs the third and last step of LNURL-auth, as per
/// <https://github.com/lnurl/luds/blob/luds/04.md>
///
/// Linking key is derived as per LUD-05
/// https://github.com/lnurl/luds/blob/luds/05.md
///
/// See the [`parse`] docs for more detail on the full workflow.
pub async fn perform_lnurl_auth<C: RestClient + ?Sized, S: LnurlAuthSigner>(
    rest_client: &C,
    req_data: &LnurlAuthRequestData,
    signer: &S,
) -> LnurlResult<LnurlCallbackStatus> {
    let url = Url::from_str(&req_data.url).map_err(|e| {
        warn!("Lnurl auth URL is invalid: {:?}", e);
        LnurlError::InvalidUri
    })?;
    let derivation_path = get_derivation_path(signer, url).await?;
    let sig = signer
        .sign_ecdsa(
            &hex::decode(&req_data.k1).map_err(|_| LnurlError::InvalidK1)?,
            &derivation_path,
        )
        .await?;
    let xpub_bytes = signer.derive_bip32_pub_key(&derivation_path).await?;
    let xpub = Xpub::decode(xpub_bytes.as_slice()).map_err(|e| {
        error!("Failed to decode xpub: {:?}", e);
        LnurlError::General("failed to decode xpub".to_string())
    })?;

    // <LNURL_hostname_and_path>?<LNURL_existing_query_parameters>&sig=<hex(sign(utf8ToBytes(k1), linkingPrivKey))>&key=<hex(linkingKey)>
    let mut callback_url = Url::from_str(&req_data.url).map_err(|e| {
        warn!("Lnurl auth callback URL is invalid: {:?}", e);
        LnurlError::InvalidUri
    })?;
    callback_url
        .query_pairs_mut()
        .append_pair("sig", &hex::encode(&sig));
    callback_url
        .query_pairs_mut()
        .append_pair("key", &xpub.public_key.to_string());
    let (response, _) = rest_client.get(callback_url.as_ref()).await?;
    Ok(parse_json(&response)?)
}

pub fn validate_request(url: &reqwest::Url) -> Result<LnurlAuthRequestData, LnurlError> {
    let query_pairs = url.query_pairs();

    let k1 = query_pairs
        .into_iter()
        .find(|(key, _)| key == "k1")
        .map(|(_, v)| v.to_string())
        .ok_or(LnurlError::MissingK1)?;

    let maybe_action = query_pairs
        .into_iter()
        .find(|(key, _)| key == "action")
        .map(|(_, v)| v.to_string());

    let k1_bytes = hex::decode(&k1).map_err(|_| LnurlError::InvalidK1)?;
    if k1_bytes.len() != 32 {
        return Err(LnurlError::InvalidK1);
    }

    if let Some(action) = &maybe_action {
        if !["register", "login", "link", "auth"].contains(&action.as_str()) {
            return Err(LnurlError::UnsupportedAction);
        }
    }

    Ok(LnurlAuthRequestData {
        k1,
        action: maybe_action,
        domain: url.domain().ok_or(LnurlError::MissingDomain)?.to_string(),
        url: url.to_string(),
    })
}

pub async fn get_derivation_path<S: LnurlAuthSigner>(
    signer: &S,
    url: Url,
) -> LnurlResult<Vec<ChildNumber>> {
    let domain = url.domain().ok_or(LnurlError::InvalidUri)?;

    let c138 = ChildNumber::from_hardened_idx(138)
        .map_err(|_| LnurlError::General("failed to derive child auth key".to_string()))?;
    let hmac = signer
        .hmac_sha256(&[c138, ChildNumber::from(0)], domain.as_bytes())
        .await?;

    // m/138'/<long1>/<long2>/<long3>/<long4>
    Ok(vec![
        c138,
        ChildNumber::from(build_path_element_u32(hmac[0..4].try_into()?)),
        ChildNumber::from(build_path_element_u32(hmac[4..8].try_into()?)),
        ChildNumber::from(build_path_element_u32(hmac[8..12].try_into()?)),
        ChildNumber::from(build_path_element_u32(hmac[12..16].try_into()?)),
    ])
}

fn build_path_element_u32(hmac_bytes: [u8; 4]) -> u32 {
    let mut buf = [0u8; 4];
    buf[..4].copy_from_slice(&hmac_bytes);
    u32::from_be_bytes(buf)
}
