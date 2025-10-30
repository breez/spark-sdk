pub mod auth;
pub mod error;
pub mod pay;
pub mod withdraw;

use serde::{Deserialize, Serialize};

/// Contains the result of the entire LNURL interaction, as reported by the LNURL endpoint.
///
/// * `Ok` indicates the interaction with the endpoint was valid, and the endpoint
///  - started to pay the invoice asynchronously in the case of LNURL-withdraw,
///  - verified the client signature in the case of LNURL-auth
/// * `Error` indicates a generic issue the LNURL endpoint encountered, including a freetext
///   description of the reason.
///
/// Both cases are described in LUD-03 <https://github.com/lnurl/luds/blob/luds/03.md> & LUD-04: <https://github.com/lnurl/luds/blob/luds/04.md>
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "UPPERCASE")]
#[serde(tag = "status")]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum LnurlCallbackStatus {
    /// On-wire format is: `{"status": "OK"}`
    Ok,
    /// On-wire format is: `{"status": "ERROR", "reason": "error details..."}`
    #[serde(rename = "ERROR")]
    ErrorStatus {
        #[serde(flatten)]
        error_details: LnurlErrorDetails,
    },
}

/// Wrapped in a [`LnUrlError`], this represents a LNURL-endpoint error.
#[derive(Clone, Deserialize, Debug, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LnurlErrorDetails {
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use rand;
    use rand::distributions::{Alphanumeric, DistString};

    pub fn rand_string(len: usize) -> String {
        Alphanumeric.sample_string(&mut rand::thread_rng(), len)
    }
}
