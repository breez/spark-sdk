use breez_sdk_common::rest::ReqwestRestClient;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::{
    Network, SdkError,
    chain::rest_client::{
        BasicAuth, SPARK_MEMPOOL_SPACE_PASSWORD, SPARK_MEMPOOL_SPACE_URL,
        SPARK_MEMPOOL_SPACE_USERNAME,
    },
    utils::rest::get_with_retry,
};

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RecommendedFeesResponse {
    pub fastest_fee: u64,
    pub half_hour_fee: u64,
    pub hour_fee: u64,
    pub economy_fee: u64,
    pub minimum_fee: u64,
}

/// Get the recommended BTC fees based on the configured chain service.
#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
pub async fn recommended_fees(network: Network) -> Result<RecommendedFeesResponse, SdkError> {
    match network {
        Network::Mainnet => recommended_fees_mempool_space("https://mempool.space/api", None).await,
        Network::Regtest => {
            recommended_fees_mempool_space(
                SPARK_MEMPOOL_SPACE_URL,
                Some(BasicAuth::new(
                    SPARK_MEMPOOL_SPACE_USERNAME.to_string(),
                    SPARK_MEMPOOL_SPACE_PASSWORD.to_string(),
                )),
            )
            .await
        }
    }
}

async fn recommended_fees_mempool_space(
    base_url: &str,
    basic_auth: Option<BasicAuth>,
) -> Result<RecommendedFeesResponse, SdkError> {
    let url = format!("{base_url}/v1/fees/recommended");
    info!("Fetching response json from {}", url);

    let client = ReqwestRestClient::new()?;
    let (response, _) = get_with_retry(basic_auth.as_ref(), 5, &url, &client).await?;

    let response: MempoolSpaceRecommendedFeesResponse =
        serde_json::from_str(&response).map_err(|e| SdkError::Generic(e.to_string()))?;

    Ok(response.into())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MempoolSpaceRecommendedFeesResponse {
    fastest_fee: f64,
    half_hour_fee: f64,
    hour_fee: f64,
    economy_fee: f64,
    minimum_fee: f64,
}

impl From<MempoolSpaceRecommendedFeesResponse> for RecommendedFeesResponse {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn from(response: MempoolSpaceRecommendedFeesResponse) -> Self {
        Self {
            fastest_fee: response.fastest_fee.ceil() as u64,
            half_hour_fee: response.half_hour_fee.ceil() as u64,
            hour_fee: response.hour_fee.ceil() as u64,
            economy_fee: response.economy_fee.ceil() as u64,
            minimum_fee: response.minimum_fee.ceil() as u64,
        }
    }
}
