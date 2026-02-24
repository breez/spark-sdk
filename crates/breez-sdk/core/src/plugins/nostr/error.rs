use crate::SdkError;
use breez_nostr::error::NostrError;

impl From<SdkError> for NostrError {
    fn from(val: SdkError) -> Self {
        NostrError::generic(val.to_string())
    }
}

impl From<NostrError> for SdkError {
    fn from(value: NostrError) -> Self {
        Self::Generic(value.to_string())
    }
}
