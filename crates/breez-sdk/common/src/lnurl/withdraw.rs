use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::{
    input::Bolt11InvoiceDetails,
    lnurl::{
        LnurlErrorDetails,
        error::{LnurlError, LnurlResult},
    },
    rest::{RestClient, RestResponse},
};

/// Validates invoice and performs the second and last step of LNURL-withdraw, as per
/// <https://github.com/lnurl/luds/blob/luds/03.md>
///
/// See the [parse] docs for more detail on the full workflow.
///
/// Note that the invoice amount has to respect two separate min/max limits:
/// * those in the [`LnurlWithdrawRequestDetails`] showing the limits of the LNURL endpoint, and
/// * those of the current node, depending on the LSP settings and LN channel conditions
pub async fn validate_lnurl_withdraw<C: RestClient + ?Sized>(
    rest_client: &C,
    withdraw_request: &LnurlWithdrawRequestDetails,
    invoice: &Bolt11InvoiceDetails,
) -> LnurlResult<ValidatedCallbackResponse> {
    let amount_msat = invoice.amount_msat.ok_or(LnurlError::general(
        "Expected invoice amount, but found none",
    ))?;

    if !withdraw_request.is_msat_amount_valid(amount_msat) {
        return Err(LnurlError::InvalidInvoice(
            "Amount must within min/max LNURL withdrawable limits".to_string(),
        ));
    }

    // Send invoice to the LNURL-w endpoint via the callback
    let callback_url = build_withdraw_callback_url(withdraw_request, invoice)?;
    let RestResponse { body, .. } = rest_client.get_request(callback_url, None).await?;
    if let Ok(err) = serde_json::from_str::<LnurlErrorDetails>(&body) {
        return Ok(ValidatedCallbackResponse::EndpointError { data: err });
    }
    Ok(ValidatedCallbackResponse::EndpointSuccess {
        data: Box::new(CallbackResponse {
            invoice: invoice.clone(),
        }),
    })
}

pub fn build_withdraw_callback_url(
    withdraw_request: &LnurlWithdrawRequestDetails,
    bolt11_invoice_details: &Bolt11InvoiceDetails,
) -> LnurlResult<String> {
    let mut url = reqwest::Url::from_str(&withdraw_request.callback)
        .map_err(|e| LnurlError::InvalidUri(e.to_string()))?;

    url.query_pairs_mut()
        .append_pair("k1", &withdraw_request.k1);
    url.query_pairs_mut()
        .append_pair("pr", &bolt11_invoice_details.invoice.bolt11);

    Ok(url.to_string())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
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
    pub fn is_msat_amount_valid(&self, amount_msat: u64) -> bool {
        amount_msat >= self.min_withdrawable && amount_msat <= self.max_withdrawable
    }

    pub fn is_sat_amount_valid(&self, amount_sats: u64) -> bool {
        self.is_msat_amount_valid(amount_sats.saturating_mul(1000))
    }
}

pub enum ValidatedCallbackResponse {
    EndpointSuccess { data: Box<CallbackResponse> },
    EndpointError { data: LnurlErrorDetails },
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CallbackResponse {
    pub invoice: Bolt11InvoiceDetails,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use anyhow::Result;
    use serde_json::json;

    use crate::lnurl::tests::rand_string;
    use crate::lnurl::withdraw::{
        LnurlWithdrawRequestDetails, ValidatedCallbackResponse, validate_lnurl_withdraw,
    };
    use crate::rest::RestClient;
    use crate::test_utils::mock_rest_client::{MockResponse, MockRestClient};

    #[cfg(all(target_family = "wasm", target_os = "unknown"))]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    #[macros::async_test_all]
    async fn test_lnurl_withdraw_validate_amount_failure() -> Result<()> {
        let mock_rest_client = MockRestClient::new();
        let rest_client: Arc<dyn RestClient> = Arc::new(mock_rest_client);

        let invoice_str = "lnbc110n1p38q3gtpp5ypz09jrd8p993snjwnm68cph4ftwp22le34xd4r8ftspwshxhmnsdqqxqyjw5qcqpxsp5htlg8ydpywvsa7h3u4hdn77ehs4z4e844em0apjyvmqfkzqhhd2q9qgsqqqyssqszpxzxt9uuqzymr7zxcdccj5g69s8q7zzjs7sgxn9ejhnvdh6gqjcy22mss2yexunagm5r2gqczh8k24cwrqml3njskm548aruhpwssq9nvrvz";
        let invoice = crate::input::parse_invoice(invoice_str).unwrap();
        let withdraw_req = get_test_withdraw_req_data(0, 1);

        // Fail validation before even calling the endpoint (no mock needed)
        assert!(
            validate_lnurl_withdraw(rest_client.as_ref(), &withdraw_req, &invoice)
                .await
                .is_err()
        );

        Ok(())
    }

    /// Mock an LNURL-withdraw endpoint that responds with an OK to a withdraw attempt
    fn mock_lnurl_withdraw_callback(mock_rest_client: &MockRestClient, error: Option<String>) {
        let response_body = match error {
            None => json!({"status": "OK"}).to_string(),
            Some(err_reason) => json!({
                "status": "ERROR",
                "reason": err_reason
            })
            .to_string(),
        };

        mock_rest_client.add_response(MockResponse::new(200, response_body));
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
        let mock_rest_client = MockRestClient::new();
        let invoice_str = "lnbc110n1p38q3gtpp5ypz09jrd8p993snjwnm68cph4ftwp22le34xd4r8ftspwshxhmnsdqqxqyjw5qcqpxsp5htlg8ydpywvsa7h3u4hdn77ehs4z4e844em0apjyvmqfkzqhhd2q9qgsqqqyssqszpxzxt9uuqzymr7zxcdccj5g69s8q7zzjs7sgxn9ejhnvdh6gqjcy22mss2yexunagm5r2gqczh8k24cwrqml3njskm548aruhpwssq9nvrvz";
        let req_invoice = crate::input::parse_invoice(invoice_str).unwrap();
        let withdraw_req = get_test_withdraw_req_data(0, 100);

        mock_lnurl_withdraw_callback(&mock_rest_client, None);
        let rest_client: Arc<dyn RestClient> = Arc::new(mock_rest_client);

        assert!(matches!(
            validate_lnurl_withdraw(rest_client.as_ref(), &withdraw_req, &req_invoice).await?,
            ValidatedCallbackResponse::EndpointSuccess { data } if data.invoice == req_invoice
        ));

        Ok(())
    }

    #[macros::async_test_all]
    async fn test_lnurl_withdraw_endpoint_failure() -> Result<()> {
        let mock_rest_client = MockRestClient::new();
        let invoice_str = "lnbc110n1p38q3gtpp5ypz09jrd8p993snjwnm68cph4ftwp22le34xd4r8ftspwshxhmnsdqqxqyjw5qcqpxsp5htlg8ydpywvsa7h3u4hdn77ehs4z4e844em0apjyvmqfkzqhhd2q9qgsqqqyssqszpxzxt9uuqzymr7zxcdccj5g69s8q7zzjs7sgxn9ejhnvdh6gqjcy22mss2yexunagm5r2gqczh8k24cwrqml3njskm548aruhpwssq9nvrvz";
        let invoice = crate::input::parse_invoice(invoice_str).unwrap();
        let withdraw_req = get_test_withdraw_req_data(0, 100);

        // Generic error reported by endpoint
        mock_lnurl_withdraw_callback(&mock_rest_client, Some("error".to_string()));
        let rest_client: Arc<dyn RestClient> = Arc::new(mock_rest_client);

        assert!(matches!(
            validate_lnurl_withdraw(rest_client.as_ref(), &withdraw_req, &invoice).await?,
            ValidatedCallbackResponse::EndpointError { data: _ }
        ));

        Ok(())
    }
}
