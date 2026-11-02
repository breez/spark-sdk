use std::fmt::Display;

use breez_sdk_spark::{ParseError, SdkError, SignerError, X402ErrorKind, passkey::PasskeyError};
use tracing_subscriber::util::TryInitError;
use wasm_bindgen::{JsError, JsValue, prelude::wasm_bindgen};

#[derive(Clone, Debug)]
pub struct WasmError(JsValue);

pub type WasmResult<T> = Result<T, WasmError>;

impl WasmError {
    pub fn new<T: Display>(val: T) -> Self {
        WasmError(JsValue::from(format!("{val}")))
    }
}

impl From<WasmError> for JsValue {
    fn from(err: WasmError) -> Self {
        err.0
    }
}

impl From<JsValue> for WasmError {
    fn from(err: JsValue) -> Self {
        Self(err)
    }
}

impl From<TryInitError> for WasmError {
    fn from(value: TryInitError) -> Self {
        SdkError::from(value).into()
    }
}

macro_rules! wasm_error_wrapper {
    ($($t:ty),*) => {
        $(
            impl From<$t> for WasmError {
                fn from(err: $t) -> Self {
                    WasmError(JsError::new(format!("{}", err).as_str()).into())
                }
            }
        )*
    }
}

wasm_error_wrapper!(ParseError, PasskeyError, SignerError);

#[wasm_bindgen(typescript_custom_section)]
const SDK_ERROR: &'static str = r#"/**
 * The error SDK methods throw. `docsUrl` links to the guide section that
 * explains the error, when it has one. `x402` is set for an x402 failure:
 * `refused` means nothing was paid, and `paymentFailed` that the payment was
 * sent and failed, so nothing was paid either.
 */
export interface SdkError extends Error {
    docsUrl?: string;
    x402?: {
        kind: 'refused' | 'paymentFailed';
        reason: string;
        details: string;
    };
}"#;

impl From<SdkError> for WasmError {
    fn from(err: SdkError) -> Self {
        let js_error: JsValue = JsError::new(&err.to_string()).into();
        // Setting a property on a fresh object cannot fail.
        let set = |target: &JsValue, name: &str, value: &str| {
            let _ =
                js_sys::Reflect::set(target, &JsValue::from_str(name), &JsValue::from_str(value));
        };
        if let Some(docs_url) = err.docs_url() {
            set(&js_error, "docsUrl", docs_url);
        }
        if let SdkError::X402Error {
            kind,
            reason,
            details,
            ..
        } = &err
        {
            let x402: JsValue = js_sys::Object::new().into();
            let kind = match kind {
                X402ErrorKind::Refused => "refused",
                X402ErrorKind::PaymentFailed => "paymentFailed",
            };
            set(&x402, "kind", kind);
            set(&x402, "reason", reason);
            set(&x402, "details", details);
            let _ = js_sys::Reflect::set(&js_error, &JsValue::from_str("x402"), &x402);
        }
        WasmError(js_error)
    }
}

#[cfg(test)]
mod tests {
    use breez_sdk_spark::SdkError;
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::WasmError;

    fn thrown(err: SdkError) -> js_sys::Error {
        JsValue::from(WasmError::from(err))
            .dyn_into()
            .expect("SdkError is thrown as a JS Error")
    }

    fn docs_url(err: &js_sys::Error) -> JsValue {
        js_sys::Reflect::get(err, &JsValue::from_str("docsUrl")).unwrap()
    }

    #[wasm_bindgen_test]
    fn exposes_the_docs_url_as_a_property() {
        let err = thrown(SdkError::CrossChainDisabled {
            docs_url: "https://example.com/guide".to_string(),
        });
        assert_eq!(
            docs_url(&err).as_string().as_deref(),
            Some("https://example.com/guide")
        );
        assert!(!String::from(err.message()).contains("https://"));
    }

    #[wasm_bindgen_test]
    fn leaves_docs_url_undefined_when_the_error_has_none() {
        assert!(docs_url(&thrown(SdkError::Generic("boom".to_string()))).is_undefined());
    }

    #[wasm_bindgen_test]
    fn exposes_x402_failures_as_a_property() {
        let err = thrown(SdkError::X402Error {
            kind: breez_sdk_spark::X402ErrorKind::Refused,
            reason: "invalid_exact_lnbtc_request_mismatch".to_string(),
            details: "requestHash does not match the request".to_string(),
            docs_url: "https://example.com/guide".to_string(),
        });
        let x402 = js_sys::Reflect::get(&err, &JsValue::from_str("x402")).unwrap();
        let field = |name: &str| {
            js_sys::Reflect::get(&x402, &JsValue::from_str(name))
                .unwrap()
                .as_string()
        };
        assert_eq!(field("kind").as_deref(), Some("refused"));
        assert_eq!(
            field("reason").as_deref(),
            Some("invalid_exact_lnbtc_request_mismatch")
        );
        assert!(
            js_sys::Reflect::get(
                &thrown(SdkError::Generic("boom".to_string())),
                &"x402".into()
            )
            .unwrap()
            .is_undefined()
        );
    }
}
