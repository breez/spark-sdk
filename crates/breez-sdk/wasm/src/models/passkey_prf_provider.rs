use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, js_sys::Promise};

use breez_sdk_spark::seedless_restore::PasskeyPrfError;

pub(crate) fn js_error_to_passkey_prf_error(js_error: JsValue) -> PasskeyPrfError {
    let error_message = js_error
        .as_string()
        .unwrap_or_else(|| "Passkey PRF error occurred".to_string());
    PasskeyPrfError::Generic(error_message)
}

pub struct WasmPasskeyPrfProvider {
    pub inner: PasskeyPrfProvider,
}

// This assumes that we'll always be running in a single thread (true for Wasm environments)
unsafe impl Send for WasmPasskeyPrfProvider {}
unsafe impl Sync for WasmPasskeyPrfProvider {}

#[macros::async_trait]
impl breez_sdk_spark::seedless_restore::PasskeyPrfProvider for WasmPasskeyPrfProvider {
    async fn derive_prf_seed(&self, salt: String) -> Result<Vec<u8>, PasskeyPrfError> {
        let promise = self
            .inner
            .derive_prf_seed(salt)
            .map_err(js_error_to_passkey_prf_error)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_passkey_prf_error)?;

        // Convert Uint8Array to Vec<u8>
        let array = js_sys::Uint8Array::new(&result);
        Ok(array.to_vec())
    }

    async fn is_prf_available(&self) -> Result<bool, PasskeyPrfError> {
        let promise = self
            .inner
            .is_prf_available()
            .map_err(js_error_to_passkey_prf_error)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_passkey_prf_error)?;

        result
            .as_bool()
            .ok_or_else(|| PasskeyPrfError::Generic("Expected boolean result".to_string()))
    }
}

#[wasm_bindgen(typescript_custom_section)]
const PASSKEY_PRF_PROVIDER_INTERFACE: &'static str = r#"/**
 * Interface for passkey PRF (Pseudo-Random Function) operations.
 *
 * Implement this interface to provide passkey PRF functionality for seedless wallet restore.
 *
 * @example
 * ```typescript
 * class BrowserPasskeyPrfProvider implements PasskeyPrfProvider {
 *     async derivePrfSeed(salt: string): Promise<Uint8Array> {
 *         const credential = await navigator.credentials.get({
 *             publicKey: {
 *                 challenge: new Uint8Array(32),
 *                 rpId: window.location.hostname,
 *                 allowCredentials: [], // or specific credential IDs
 *                 extensions: {
 *                     prf: { eval: { first: new TextEncoder().encode(salt) } }
 *                 }
 *             }
 *         });
 *         const results = credential.getClientExtensionResults();
 *         return new Uint8Array(results.prf.results.first);
 *     }
 *
 *     async isPrfAvailable(): Promise<boolean> {
 *         return window.PublicKeyCredential?.isUserVerifyingPlatformAuthenticatorAvailable?.() ?? false;
 *     }
 * }
 * ```
 */
export interface PasskeyPrfProvider {
    /**
     * Derive a 32-byte seed from passkey PRF with the given salt.
     *
     * The platform authenticates the user via passkey and evaluates the PRF extension.
     * The salt is used as input to the PRF to derive a deterministic output.
     *
     * @param salt - The salt string to use for PRF evaluation
     * @returns A Promise resolving to the 32-byte PRF output
     * @throws If authentication fails or PRF is not supported
     */
    derivePrfSeed(salt: string): Promise<Uint8Array>;

    /**
     * Check if a PRF-capable passkey is available on this device.
     *
     * This allows applications to gracefully degrade if passkey PRF is not supported.
     *
     * @returns A Promise resolving to true if PRF-capable passkey is available
     */
    isPrfAvailable(): Promise<boolean>;
}"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "PasskeyPrfProvider")]
    pub type PasskeyPrfProvider;

    #[wasm_bindgen(structural, method, js_name = "derivePrfSeed", catch)]
    pub fn derive_prf_seed(this: &PasskeyPrfProvider, salt: String) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "isPrfAvailable", catch)]
    pub fn is_prf_available(this: &PasskeyPrfProvider) -> Result<Promise, JsValue>;
}
