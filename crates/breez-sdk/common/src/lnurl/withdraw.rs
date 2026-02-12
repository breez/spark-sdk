use serde::{Deserialize, Serialize};

use crate::lnurl::{
    LnurlErrorDetails,
    error::{LnurlError, LnurlResult},
};

use platform_utils::HttpClient;

/// Performs the second and last step of LNURL-withdraw,
/// as per <https://github.com/lnurl/luds/blob/luds/03.md>
pub async fn execute_lnurl_withdraw<C: HttpClient + ?Sized>(
    http_client: &C,
    withdraw_request: &LnurlWithdrawRequestDetails,
    invoice: &str,
) -> LnurlResult<ValidatedCallbackResponse> {
    // Send invoice to the LNURL-w endpoint via the callback
    let callback_url = build_withdraw_callback_url(withdraw_request, invoice)?;
    let response = http_client.get(callback_url, None).await?;
    if let Ok(err) = response.json::<LnurlErrorDetails>() {
        return Ok(ValidatedCallbackResponse::EndpointError { data: err });
    }
    Ok(ValidatedCallbackResponse::EndpointSuccess)
}

pub fn build_withdraw_callback_url(
    withdraw_request: &LnurlWithdrawRequestDetails,
    invoice: &str,
) -> LnurlResult<String> {
    let mut url = url::Url::parse(&withdraw_request.callback)
        .map_err(|e| LnurlError::InvalidUri(e.to_string()))?;

    url.query_pairs_mut()
        .append_pair("k1", &withdraw_request.k1);
    url.query_pairs_mut().append_pair("pr", invoice);

    Ok(url.to_string())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LnurlWithdrawRequestDetails {
    pub callback: String,
    pub k1: String,
    pub default_description: String,
    /// The minimum amount, in millisats, that this LNURL-withdraw endpoint accepts
    pub min_withdrawable: u64,
    /// The maximum amount, in millisats, that this LNURL-withdraw endpoint accepts
    pub max_withdrawable: u64,
}

impl LnurlWithdrawRequestDetails {
    pub fn is_amount_valid(&self, amount_sats: u64) -> bool {
        let amount_msat = amount_sats.saturating_mul(1000);
        amount_msat >= self.min_withdrawable && amount_msat <= self.max_withdrawable
    }
}

pub enum ValidatedCallbackResponse {
    EndpointSuccess,
    EndpointError { data: LnurlErrorDetails },
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use anyhow::Result;
    use platform_utils::HttpClient;
    use serde_json::json;

    use crate::lnurl::tests::rand_string;
    use crate::lnurl::withdraw::{
        LnurlWithdrawRequestDetails, ValidatedCallbackResponse, execute_lnurl_withdraw,
    };
    use crate::test_utils::mock_rest_client::{MockResponse, MockRestClient};

    #[cfg(all(target_family = "wasm", target_os = "unknown"))]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    #[macros::async_test_all]
    async fn test_lnurl_withdraw_validate_amount_failure() -> Result<()> {
        let mock_http_client = MockRestClient::new();
        let http_client: Arc<dyn HttpClient> = Arc::new(mock_http_client);

        let invoice = "lnbc110n1p38q3gtpp5ypz09jrd8p993snjwnm68cph4ftwp22le34xd4r8ftspwshxhmnsdqqxqyjw5qcqpxsp5htlg8ydpywvsa7h3u4hdn77ehs4z4e844em0apjyvmqfkzqhhd2q9qgsqqqyssqszpxzxt9uuqzymr7zxcdccj5g69s8q7zzjs7sgxn9ejhnvdh6gqjcy22mss2yexunagm5r2gqczh8k24cwrqml3njskm548aruhpwssq9nvrvz";
        let withdraw_req = get_test_withdraw_req_data(0, 1);

        // Fail validation before even calling the endpoint (no mock needed)
        assert!(
            execute_lnurl_withdraw(http_client.as_ref(), &withdraw_req, invoice)
                .await
                .is_err()
        );

        Ok(())
    }

    /// Mock an LNURL-withdraw endpoint that responds with an OK to a withdraw attempt
    fn mock_lnurl_withdraw_callback(mock_http_client: &MockRestClient, error: Option<String>) {
        let response_body = match error {
            None => json!({"status": "OK"}).to_string(),
            Some(err_reason) => json!({
                "status": "ERROR",
                "reason": err_reason
            })
            .to_string(),
        };

        mock_http_client.add_response(MockResponse::new(200, response_body));
    }

    fn get_test_withdraw_req_data(min_sat: u64, max_sat: u64) -> LnurlWithdrawRequestDetails {
        LnurlWithdrawRequestDetails {
            min_withdrawable: min_sat.saturating_mul(1000),
            max_withdrawable: max_sat.saturating_mul(1000),
            k1: rand_string(10),
            default_description: "test description".into(),
            callback: "http://127.0.0.1:8080/callback".into(),
        }
    }

    #[macros::async_test_all]
    async fn test_lnurl_withdraw_success() -> Result<()> {
        let mock_http_client = MockRestClient::new();
        let invoice = "lnbc110n1p38q3gtpp5ypz09jrd8p993snjwnm68cph4ftwp22le34xd4r8ftspwshxhmnsdqqxqyjw5qcqpxsp5htlg8ydpywvsa7h3u4hdn77ehs4z4e844em0apjyvmqfkzqhhd2q9qgsqqqyssqszpxzxt9uuqzymr7zxcdccj5g69s8q7zzjs7sgxn9ejhnvdh6gqjcy22mss2yexunagm5r2gqczh8k24cwrqml3njskm548aruhpwssq9nvrvz";
        let withdraw_req = get_test_withdraw_req_data(0, 100);

        mock_lnurl_withdraw_callback(&mock_http_client, None);
        let http_client: Arc<dyn HttpClient> = Arc::new(mock_http_client);

        assert!(matches!(
            execute_lnurl_withdraw(http_client.as_ref(), &withdraw_req, invoice).await?,
            ValidatedCallbackResponse::EndpointSuccess
        ));

        Ok(())
    }

    #[macros::async_test_all]
    async fn test_lnurl_withdraw_endpoint_failure() -> Result<()> {
        let mock_http_client = MockRestClient::new();
        let invoice = "lnbc110n1p38q3gtpp5ypz09jrd8p993snjwnm68cph4ftwp22le34xd4r8ftspwshxhmnsdqqxqyjw5qcqpxsp5htlg8ydpywvsa7h3u4hdn77ehs4z4e844em0apjyvmqfkzqhhd2q9qgsqqqyssqszpxzxt9uuqzymr7zxcdccj5g69s8q7zzjs7sgxn9ejhnvdh6gqjcy22mss2yexunagm5r2gqczh8k24cwrqml3njskm548aruhpwssq9nvrvz";
        let withdraw_req = get_test_withdraw_req_data(0, 100);

        // Generic error reported by endpoint
        mock_lnurl_withdraw_callback(&mock_http_client, Some("error".to_string()));
        let http_client: Arc<dyn HttpClient> = Arc::new(mock_http_client);

        assert!(matches!(
            execute_lnurl_withdraw(http_client.as_ref(), &withdraw_req, invoice).await?,
            ValidatedCallbackResponse::EndpointError { data: _ }
        ));

        Ok(())
    }
}
