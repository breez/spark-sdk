use breez_sdk_common::input;
use spark_wallet::{Preimage, PreimageRequest, PreimageRequestStatus};

use crate::{
    FetchLightningSendStatusRequest, FetchLightningSendStatusResponse, PaymentStatus,
    error::SdkError, sdk::BreezSdk,
};

pub(in crate::sdk) async fn fetch_lightning_send_status(
    sdk: &BreezSdk,
    request: FetchLightningSendStatusRequest,
) -> Result<FetchLightningSendStatusResponse, SdkError> {
    let invoice = input::parse_invoice(&request.invoice)
        .ok_or_else(|| SdkError::InvalidInput("Invalid Bolt11 invoice".to_string()))?;
    let preimage_requests = sdk
        .spark_wallet
        .query_sent_preimage_requests(&invoice.payment_hash)
        .await?;
    send_status(&preimage_requests)
}

fn send_status(
    preimage_requests: &[PreimageRequest],
) -> Result<FetchLightningSendStatusResponse, SdkError> {
    if let Some(completed) = preimage_requests
        .iter()
        .find(|r| r.preimage.is_some() || r.status == PreimageRequestStatus::PreimageShared)
    {
        return Ok(FetchLightningSendStatusResponse {
            status: Some(PaymentStatus::Completed),
            preimage: completed.preimage.as_ref().map(Preimage::encode_hex),
        });
    }
    if preimage_requests
        .iter()
        .any(|r| r.status == PreimageRequestStatus::Unknown)
    {
        return Err(SdkError::Generic(
            "Unrecognized preimage request status".to_string(),
        ));
    }
    let status = if preimage_requests
        .iter()
        .any(|r| r.status == PreimageRequestStatus::WaitingForPreimage)
    {
        Some(PaymentStatus::Pending)
    } else if preimage_requests.is_empty() {
        None
    } else {
        Some(PaymentStatus::Failed)
    };
    Ok(FetchLightningSendStatusResponse {
        status,
        preimage: None,
    })
}

#[cfg(test)]
mod tests {
    use bitcoin::hashes::{Hash, sha256};
    use macros::test_all;
    use platform_utils::time::SystemTime;

    use super::*;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    const PREIMAGE: &str = "0101010101010101010101010101010101010101010101010101010101010101";

    fn request(status: PreimageRequestStatus, preimage: Option<&str>) -> PreimageRequest {
        PreimageRequest {
            payment_hash: sha256::Hash::hash(b"invoice"),
            status,
            created_time: SystemTime::now(),
            expiry_time: SystemTime::now(),
            preimage: preimage.map(|p| Preimage::from_hex(p).unwrap()),
        }
    }

    fn response(
        status: Option<PaymentStatus>,
        preimage: Option<&str>,
    ) -> FetchLightningSendStatusResponse {
        FetchLightningSendStatusResponse {
            status,
            preimage: preimage.map(ToString::to_string),
        }
    }

    #[test_all]
    fn no_send_has_no_status() {
        assert_eq!(send_status(&[]).unwrap(), response(None, None));
    }

    #[test_all]
    fn a_shared_preimage_is_completed() {
        let requests = [request(
            PreimageRequestStatus::PreimageShared,
            Some(PREIMAGE),
        )];
        assert_eq!(
            send_status(&requests).unwrap(),
            response(Some(PaymentStatus::Completed), Some(PREIMAGE))
        );
    }

    #[test_all]
    fn a_waiting_send_is_pending() {
        let requests = [request(PreimageRequestStatus::WaitingForPreimage, None)];
        assert_eq!(
            send_status(&requests).unwrap(),
            response(Some(PaymentStatus::Pending), None)
        );
    }

    #[test_all]
    fn only_returned_sends_are_failed() {
        let requests = [
            request(PreimageRequestStatus::Returned, None),
            request(PreimageRequestStatus::Returned, None),
        ];
        assert_eq!(
            send_status(&requests).unwrap(),
            response(Some(PaymentStatus::Failed), None)
        );
    }

    #[test_all]
    fn a_live_send_wins_over_returned_ones() {
        let pending = [
            request(PreimageRequestStatus::Returned, None),
            request(PreimageRequestStatus::WaitingForPreimage, None),
        ];
        assert_eq!(
            send_status(&pending).unwrap(),
            response(Some(PaymentStatus::Pending), None)
        );

        let completed = [
            request(PreimageRequestStatus::Returned, None),
            request(PreimageRequestStatus::PreimageShared, Some(PREIMAGE)),
        ];
        assert_eq!(
            send_status(&completed).unwrap(),
            response(Some(PaymentStatus::Completed), Some(PREIMAGE))
        );
    }

    #[test_all]
    fn a_known_preimage_is_completed_whatever_the_status() {
        let requests = [request(PreimageRequestStatus::Unknown, Some(PREIMAGE))];
        assert_eq!(
            send_status(&requests).unwrap(),
            response(Some(PaymentStatus::Completed), Some(PREIMAGE))
        );
    }

    #[test_all]
    fn an_unknown_status_without_a_preimage_is_an_error() {
        let requests = [
            request(PreimageRequestStatus::Returned, None),
            request(PreimageRequestStatus::Unknown, None),
        ];
        assert!(send_status(&requests).is_err());
    }
}
