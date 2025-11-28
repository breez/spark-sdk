use std::{collections::HashMap, time::Duration};

use base64::Engine as _;
use base64::engine::general_purpose;
use breez_sdk_common::{
    error::ServiceConnectivityError,
    rest::{RestClient as CommonRestClient, RestResponse},
};

use crate::chain::rest_client::BasicAuth;

/// Base backoff in milliseconds.
const BASE_BACKOFF_MILLIS: Duration = Duration::from_millis(256);

pub const RETRYABLE_ERROR_CODES: [u16; 3] = [
    429, // TOO_MANY_REQUESTS
    500, // INTERNAL_SERVER_ERROR
    503, // SERVICE_UNAVAILABLE
];

pub(crate) async fn get_with_retry(
    basic_auth: Option<&BasicAuth>,
    max_retries: usize,
    url: &str,
    client: &dyn CommonRestClient,
) -> Result<(String, u16), ServiceConnectivityError> {
    let mut delay = BASE_BACKOFF_MILLIS;
    let mut attempts = 0;

    loop {
        let mut headers: Option<HashMap<String, String>> = None;
        if let Some(basic_auth) = &basic_auth {
            let auth_string = format!("{}:{}", basic_auth.username, basic_auth.password);
            let encoded_auth = general_purpose::STANDARD.encode(auth_string.as_bytes());

            headers = Some(
                vec![("Authorization".to_string(), format!("Basic {encoded_auth}"))]
                    .into_iter()
                    .collect(),
            );
        }

        let RestResponse { body, status } = client.get_request(url.to_string(), headers).await?;
        match status {
            status if attempts < max_retries && is_status_retryable(status) => {
                tokio::time::sleep(delay).await;
                attempts = attempts.saturating_add(1);
                delay = delay.saturating_mul(2);
            }
            _ => {
                if !(200..300).contains(&status) {
                    return Err(ServiceConnectivityError::Status { status, body });
                }
                return Ok((body, status));
            }
        }
    }
}

fn is_status_retryable(status: u16) -> bool {
    RETRYABLE_ERROR_CODES.contains(&status)
}
