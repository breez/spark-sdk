use super::error::OperatorRpcError;
use tonic::metadata::{Ascii, MetadataMap, MetadataValue};

const IDEMPOTENCY_KEY_HEADER: &str = "x-idempotency-key";

/// Sets the `x-idempotency-key` gRPC metadata header on a request.
///
/// When the signing operator receives duplicate requests with the same
/// idempotency key, it returns the cached response from the first
/// successful call instead of processing the request again.
pub(crate) fn set_idempotency_key(
    metadata: &mut MetadataMap,
    idempotency_key: Option<String>,
) -> Result<(), OperatorRpcError> {
    if let Some(key) = idempotency_key {
        let value = key
            .parse::<MetadataValue<Ascii>>()
            .map_err(|e| OperatorRpcError::Generic(format!("invalid idempotency key: {e}")))?;
        metadata.insert(IDEMPOTENCY_KEY_HEADER, value);
    }
    Ok(())
}
