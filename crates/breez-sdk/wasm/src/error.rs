use std::fmt::Display;

use breez_sdk_spark::{ParseError, SdkError, SignerError, passkey::PasskeyError};
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
 * explains the error, when it has one.
 */
export interface SdkError extends Error {
    docsUrl?: string;
}"#;

impl From<SdkError> for WasmError {
    fn from(err: SdkError) -> Self {
        let js_error: JsValue = JsError::new(&err.to_string()).into();
        if let Some(docs_url) = err.docs_url() {
            // Setting a property on a fresh `Error` object cannot fail.
            let _ = js_sys::Reflect::set(
                &js_error,
                &JsValue::from_str("docsUrl"),
                &JsValue::from_str(docs_url),
            );
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
}
