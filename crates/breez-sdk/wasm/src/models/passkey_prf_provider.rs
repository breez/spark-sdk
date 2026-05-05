use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, js_sys::Promise};

use breez_sdk_spark::passkey::PasskeyPrfError;

pub(crate) fn js_error_to_passkey_prf_error(js_error: JsValue) -> PasskeyPrfError {
    let error_message = js_error
        .as_string()
        .unwrap_or_else(|| "Passkey PRF error occurred".to_string());
    PasskeyPrfError::Generic(error_message)
}

pub struct WasmPrfProvider {
    pub inner: PrfProvider,
}

// This assumes that we'll always be running in a single thread (true for Wasm environments)
unsafe impl Send for WasmPrfProvider {}
unsafe impl Sync for WasmPrfProvider {}

#[macros::async_trait]
impl breez_sdk_spark::passkey::PrfProvider for WasmPrfProvider {
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

    async fn derive_prf_seeds(&self, salts: Vec<String>) -> Result<Vec<Vec<u8>>, PasskeyPrfError> {
        // Probe the JS side: if the foreign object exposes
        // `derivePrfSeeds`, prefer the bulk fast path. Custom JS
        // providers that only implement the legacy `derivePrfSeed`
        // method fall back to the trait's default loop, which
        // produces N prompts for N salts.
        let target: &wasm_bindgen::JsValue = self.inner.as_ref();
        let key = wasm_bindgen::JsValue::from_str("derivePrfSeeds");
        let supports_bulk = js_sys::Reflect::has(target, &key).unwrap_or(false)
            && js_sys::Reflect::get(target, &key)
                .map(|v| v.is_function())
                .unwrap_or(false);

        if !supports_bulk {
            let mut out = Vec::with_capacity(salts.len());
            for salt in salts {
                out.push(self.derive_prf_seed(salt).await?);
            }
            return Ok(out);
        }

        // Build a JS array of strings to pass to derivePrfSeeds.
        let salts_array = js_sys::Array::new();
        for salt in &salts {
            salts_array.push(&wasm_bindgen::JsValue::from_str(salt));
        }

        let func = js_sys::Reflect::get(target, &key)
            .map_err(js_error_to_passkey_prf_error)?
            .dyn_into::<js_sys::Function>()
            .map_err(|_| {
                PasskeyPrfError::Generic("derivePrfSeeds is not a function".to_string())
            })?;
        let result_promise = func
            .call1(target, &salts_array)
            .map_err(js_error_to_passkey_prf_error)?
            .dyn_into::<Promise>()
            .map_err(|_| {
                PasskeyPrfError::Generic("derivePrfSeeds did not return a Promise".to_string())
            })?;
        let result = JsFuture::from(result_promise)
            .await
            .map_err(js_error_to_passkey_prf_error)?;

        // Result should be Uint8Array[]. Convert each entry.
        let array = js_sys::Array::from(&result);
        let len = array.length() as usize;
        if len != salts.len() {
            return Err(PasskeyPrfError::Generic(format!(
                "derivePrfSeeds returned {} outputs, expected {}",
                len,
                salts.len()
            )));
        }
        let mut out = Vec::with_capacity(len);
        for i in 0..array.length() {
            let item = array.get(i);
            let bytes = js_sys::Uint8Array::new(&item).to_vec();
            out.push(bytes);
        }
        Ok(out)
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
const PRF_PROVIDER_INTERFACE: &'static str = r#"/**
 * Interface for PRF (Pseudo-Random Function) operations backing seedless
 * wallet restore.
 *
 * Implemented by the built-in `PasskeyProvider` (browser passkey via the
 * WebAuthn PRF extension); also implementable directly for custom
 * deterministic sources (YubiKey HMAC challenge, FIDO2 hmac-secret, on-disk
 * key material, hardware HSMs).
 *
 * @example
 * ```typescript
 * class BrowserPasskeyProvider implements PrfProvider {
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
export interface PrfProvider {
    /**
     * Derive a 32-byte seed from PRF with the given salt.
     *
     * The platform authenticates the user (typically via passkey) and
     * evaluates the PRF extension or equivalent. The salt is used as input
     * to the PRF to derive a deterministic output.
     *
     * @param salt - The salt string to use for PRF evaluation
     * @returns A Promise resolving to the 32-byte PRF output
     * @throws If authentication fails or PRF is not supported
     */
    derivePrfSeed(salt: string): Promise<Uint8Array>;

    /**
     * Optional bulk PRF derivation. Implementations that can collapse
     * multiple derivations into a single user prompt (e.g. WebAuthn PRF
     * with `prf.eval.first` + `prf.eval.second`) should override this.
     * The SDK detects the presence of this method at runtime and falls
     * back to looping `derivePrfSeed` when absent or unavailable.
     *
     * Output ordering matches input ordering.
     *
     * @param salts - Salt strings in caller order
     * @returns A Promise resolving to one 32-byte output per salt
     */
    derivePrfSeeds?(salts: string[]): Promise<Uint8Array[]>;

    /**
     * Check if a PRF-capable source is available on this device.
     *
     * This allows applications to gracefully degrade if passkey PRF is not supported.
     *
     * @returns A Promise resolving to true if a PRF-capable source is available
     */
    isPrfAvailable(): Promise<boolean>;
}"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "PrfProvider")]
    pub type PrfProvider;

    #[wasm_bindgen(structural, method, js_name = "derivePrfSeed", catch)]
    pub fn derive_prf_seed(this: &PrfProvider, salt: String) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "isPrfAvailable", catch)]
    pub fn is_prf_available(this: &PrfProvider) -> Result<Promise, JsValue>;
}
