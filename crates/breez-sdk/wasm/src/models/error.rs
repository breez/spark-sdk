use breez_sdk_common::error::ServiceConnectivityError;
use wasm_bindgen::JsValue;

pub(crate) fn js_error_to_chain_service_error(
    js_error: JsValue,
) -> breez_sdk_spark::ChainServiceError {
    let error_message = js_error
        .as_string()
        .unwrap_or_else(|| "Chain service error occurred".to_string());
    breez_sdk_spark::ChainServiceError::Generic(error_message)
}

pub(crate) fn js_error_to_payment_observer_error(
    js_error: JsValue,
) -> breez_sdk_spark::PaymentObserverError {
    let error_message = js_error
        .as_string()
        .unwrap_or_else(|| "Payment observer error occurred".to_string());
    breez_sdk_spark::PaymentObserverError::Generic(error_message)
}

pub(crate) fn js_error_to_service_connectivity_error(
    js_error: JsValue,
) -> ServiceConnectivityError {
    let error_message = js_error
        .as_string()
        .unwrap_or_else(|| "Service connectivity error occurred".to_string());
    ServiceConnectivityError::Other(error_message)
}
