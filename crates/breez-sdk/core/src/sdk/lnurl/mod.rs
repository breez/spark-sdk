use breez_sdk_common::lnurl::{self, error::LnurlError};

use crate::{
    LnurlAuthRequestDetails, LnurlCallbackStatus, LnurlPayRequest, LnurlPayResponse,
    LnurlWithdrawInfo, LnurlWithdrawRequest, LnurlWithdrawResponse, PrepareLnurlPayRequest,
    PrepareLnurlPayResponse, WaitForPaymentIdentifier,
    error::SdkError,
    persist::{ObjectCacheRepository, PaymentMetadata},
};
use breez_sdk_common::lnurl::withdraw::execute_lnurl_withdraw;

use super::BreezSdk;

mod pay;

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[allow(clippy::needless_pass_by_value)]
impl BreezSdk {
    pub async fn prepare_lnurl_pay(
        &self,
        request: PrepareLnurlPayRequest,
    ) -> Result<PrepareLnurlPayResponse, SdkError> {
        pay::prepare(self, request).await
    }

    pub async fn lnurl_pay(&self, request: LnurlPayRequest) -> Result<LnurlPayResponse, SdkError> {
        pay::send(self, request).await
    }

    /// Performs an LNURL withdraw operation for the amount of satoshis to
    /// withdraw and the LNURL withdraw request details. The LNURL withdraw request
    /// details can be obtained from calling [`BreezSdk::parse`].
    ///
    /// The method generates a Lightning invoice for the withdraw amount, stores
    /// the LNURL withdraw metadata, and performs the LNURL withdraw using  the generated
    /// invoice.
    ///
    /// If the `completion_timeout_secs` parameter is provided and greater than 0, the
    /// method will wait for the payment to be completed within that period. If the
    /// withdraw is completed within the timeout, the `payment` field in the response
    /// will be set with the payment details. If the `completion_timeout_secs`
    /// parameter is not provided or set to 0, the method will not wait for the payment
    /// to be completed. If the withdraw is not completed within the
    /// timeout, the `payment` field will be empty.
    ///
    /// # Arguments
    ///
    /// * `request` - The LNURL withdraw request
    ///
    /// # Returns
    ///
    /// Result containing either:
    /// * `LnurlWithdrawResponse` - The payment details if the withdraw request was successful
    /// * `SdkError` - If there was an error during the withdraw process
    pub async fn lnurl_withdraw(
        &self,
        request: LnurlWithdrawRequest,
    ) -> Result<LnurlWithdrawResponse, SdkError> {
        self.maybe_ensure_spark_private_mode_initialized().await?;
        let LnurlWithdrawRequest {
            amount_sats,
            withdraw_request,
            completion_timeout_secs,
        } = request;
        let withdraw_request: breez_sdk_common::lnurl::withdraw::LnurlWithdrawRequestDetails =
            withdraw_request.into();
        if !withdraw_request.is_amount_valid(amount_sats) {
            return Err(SdkError::InvalidInput(
                "Amount must be within min/max LNURL withdrawable limits".to_string(),
            ));
        }

        // Generate a Lightning invoice for the withdraw, keeping the SSP-side
        // receive id for the targeted wait below.
        let receive = self
            .receive_bolt11_invoice_inner(
                withdraw_request.default_description.clone(),
                Some(amount_sats),
                None,
                None,
            )
            .await?;
        let payment_request = receive.invoice.clone();
        let ssp_receive_id = receive.id;

        // Store the LNURL withdraw metadata before executing the withdraw
        let cache = ObjectCacheRepository::new(self.storage.clone());
        cache
            .save_payment_metadata(
                &payment_request,
                &PaymentMetadata {
                    lnurl_withdraw_info: Some(LnurlWithdrawInfo {
                        withdraw_url: withdraw_request.callback.clone(),
                    }),
                    lnurl_description: Some(withdraw_request.default_description.clone()),
                    ..Default::default()
                },
            )
            .await?;

        // Perform the LNURL withdraw using the generated invoice
        let withdraw_response = execute_lnurl_withdraw(
            self.lnurl_client.as_ref(),
            &withdraw_request,
            &payment_request,
        )
        .await?;
        if let lnurl::withdraw::ValidatedCallbackResponse::EndpointError { data } =
            withdraw_response
        {
            return Err(LnurlError::EndpointError(data.reason).into());
        }

        let completion_timeout_secs = match completion_timeout_secs {
            Some(secs) if secs > 0 => secs,
            _ => {
                return Ok(LnurlWithdrawResponse {
                    payment_request,
                    payment: None,
                });
            }
        };

        // Wait for the LNURL service to pay the invoice
        let payment = self
            .wait_for_incoming_payment(
                WaitForPaymentIdentifier::LightningReceive {
                    invoice: payment_request.clone(),
                    ssp_id: ssp_receive_id,
                },
                completion_timeout_secs,
            )
            .await
            .ok();
        Ok(LnurlWithdrawResponse {
            payment_request,
            payment,
        })
    }

    /// Performs LNURL-auth with the service.
    ///
    /// This method implements the LNURL-auth protocol as specified in LUD-04 and LUD-05.
    /// It derives a domain-specific linking key, signs the challenge, and sends the
    /// authentication request to the service.
    pub async fn lnurl_auth(
        &self,
        request_data: LnurlAuthRequestDetails,
    ) -> Result<LnurlCallbackStatus, SdkError> {
        let request: breez_sdk_common::lnurl::auth::LnurlAuthRequestDetails = request_data.into();
        let status = breez_sdk_common::lnurl::auth::perform_lnurl_auth(
            self.lnurl_client.as_ref(),
            &request,
            self.lnurl_auth_signer.as_ref(),
        )
        .await
        .map_err(|e| match e {
            LnurlError::ServiceConnectivity(msg) => SdkError::NetworkError(msg.to_string()),
            LnurlError::InvalidUri(msg) => SdkError::InvalidInput(msg),
            _ => SdkError::Generic(e.to_string()),
        })?;
        Ok(status.into())
    }
}
