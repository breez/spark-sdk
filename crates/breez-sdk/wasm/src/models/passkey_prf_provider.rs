use std::sync::OnceLock;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, js_sys::Promise};

use breez_sdk_spark::passkey::{CreatePasskeyRequest, PrfProviderError, RegisteredCredential};

pub(crate) fn js_error_to_prf_provider_error(js_error: JsValue) -> PrfProviderError {
    // Map typed JS error classes thrown by the bundled JS provider
    // back to the typed Rust variant so callers don't have to
    // substring-match `error.message`. Other errors fall through to
    // `Generic`.
    if let Some(name) = js_sys::Reflect::get(&js_error, &JsValue::from_str("name"))
        .ok()
        .and_then(|v| v.as_string())
    {
        match name.as_str() {
            "PasskeyAlreadyExistsError" => {
                let message = js_sys::Reflect::get(&js_error, &JsValue::from_str("message"))
                    .ok()
                    .and_then(|v| v.as_string())
                    .unwrap_or_else(|| "credential already exists".to_string());
                return PrfProviderError::CredentialAlreadyExists(message);
            }
            "PasskeyTimedOutError" => {
                return PrfProviderError::UserTimedOut;
            }
            _ => {}
        }
    }

    let error_message = js_error
        .as_string()
        .unwrap_or_else(|| "Passkey PRF error occurred".to_string());
    PrfProviderError::Generic(error_message)
}

pub struct WasmPrfProvider {
    pub inner: PrfProvider,
    /// Cached `createPasskey` presence probe — JS providers may omit
    /// it (only platform passkey backends implement registration).
    supports_create: OnceLock<bool>,
}

impl WasmPrfProvider {
    pub fn new(inner: PrfProvider) -> Self {
        Self {
            inner,
            supports_create: OnceLock::new(),
        }
    }

    /// Probe whether the JS provider exposes a method named `name`.
    /// Cached in `cell` so subsequent calls are free.
    fn js_has_method(&self, name: &str, cell: &OnceLock<bool>) -> bool {
        let target: &JsValue = self.inner.as_ref();
        let key = JsValue::from_str(name);
        *cell.get_or_init(|| {
            js_sys::Reflect::has(target, &key).unwrap_or(false)
                && js_sys::Reflect::get(target, &key)
                    .map(|v| v.is_function())
                    .unwrap_or(false)
        })
    }
}

// This assumes that we'll always be running in a single thread (true for Wasm environments)
unsafe impl Send for WasmPrfProvider {}
unsafe impl Sync for WasmPrfProvider {}

#[macros::async_trait]
impl breez_sdk_spark::passkey::PrfProvider for WasmPrfProvider {
    async fn derive_seeds(&self, salts: Vec<String>) -> Result<Vec<Vec<u8>>, PrfProviderError> {
        let salts_array = js_sys::Array::new();
        for salt in &salts {
            salts_array.push(&JsValue::from_str(salt));
        }

        let target: &JsValue = self.inner.as_ref();
        let func = js_sys::Reflect::get(target, &JsValue::from_str("deriveSeeds"))
            .map_err(js_error_to_prf_provider_error)?
            .dyn_into::<js_sys::Function>()
            .map_err(|_| PrfProviderError::Generic("deriveSeeds is not a function".to_string()))?;
        let result_promise = func
            .call1(target, &salts_array)
            .map_err(js_error_to_prf_provider_error)?
            .dyn_into::<Promise>()
            .map_err(|_| {
                PrfProviderError::Generic("deriveSeeds did not return a Promise".to_string())
            })?;
        let result = JsFuture::from(result_promise)
            .await
            .map_err(js_error_to_prf_provider_error)?;

        let array = js_sys::Array::from(&result);
        let len = array.length() as usize;
        if len != salts.len() {
            return Err(PrfProviderError::Generic(format!(
                "deriveSeeds returned {} outputs, expected {}",
                len,
                salts.len()
            )));
        }
        let mut out = Vec::with_capacity(len);
        for i in 0..array.length() {
            let item = array.get(i);
            out.push(js_sys::Uint8Array::new(&item).to_vec());
        }
        Ok(out)
    }

    async fn is_supported(&self) -> Result<bool, PrfProviderError> {
        let promise = self
            .inner
            .is_supported()
            .map_err(js_error_to_prf_provider_error)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_prf_provider_error)?;

        result
            .as_bool()
            .ok_or_else(|| PrfProviderError::Generic("Expected boolean result".to_string()))
    }

    async fn create_passkey(
        &self,
        request: CreatePasskeyRequest,
    ) -> Result<RegisteredCredential, PrfProviderError> {
        // Custom providers may not implement explicit creation; fall
        // back to the trait default (`PrfNotSupported`).
        if !self.js_has_method("createPasskey", &self.supports_create) {
            return Err(PrfProviderError::PrfNotSupported);
        }

        let target: &JsValue = self.inner.as_ref();
        let func = js_sys::Reflect::get(target, &JsValue::from_str("createPasskey"))
            .map_err(js_error_to_prf_provider_error)?
            .dyn_into::<js_sys::Function>()
            .map_err(|_| {
                PrfProviderError::Generic("createPasskey is not a function".to_string())
            })?;

        let js_request = build_create_passkey_request(&request)?;
        let result_promise = func
            .call1(target, &js_request)
            .map_err(js_error_to_prf_provider_error)?
            .dyn_into::<Promise>()
            .map_err(|_| {
                PrfProviderError::Generic("createPasskey did not return a Promise".to_string())
            })?;
        let result = JsFuture::from(result_promise)
            .await
            .map_err(js_error_to_prf_provider_error)?;

        parse_registered_credential(&result)
    }
}

/// Marshal a [`CreatePasskeyRequest`] into a JS object literal shaped
/// per `index.d.ts#CreatePasskeyRequest`. `Vec<u8>` payloads cross as
/// `Uint8Array` (not plain arrays) to match what the JS provider's
/// `navigator.credentials.create` call expects.
fn build_create_passkey_request(
    request: &CreatePasskeyRequest,
) -> Result<JsValue, PrfProviderError> {
    let obj = js_sys::Object::new();

    if !request.exclude_credential_ids.is_empty() {
        let arr = js_sys::Array::new();
        for id in &request.exclude_credential_ids {
            arr.push(&js_sys::Uint8Array::from(id.as_slice()));
        }
        js_sys::Reflect::set(&obj, &JsValue::from_str("excludeCredentialIds"), &arr)
            .map_err(js_error_to_prf_provider_error)?;
    }
    if let Some(user_id) = &request.user_id {
        js_sys::Reflect::set(
            &obj,
            &JsValue::from_str("userId"),
            &js_sys::Uint8Array::from(user_id.as_slice()),
        )
        .map_err(js_error_to_prf_provider_error)?;
    }
    if let Some(user_name) = &request.user_name {
        js_sys::Reflect::set(
            &obj,
            &JsValue::from_str("userName"),
            &JsValue::from_str(user_name),
        )
        .map_err(js_error_to_prf_provider_error)?;
    }
    if let Some(user_display_name) = &request.user_display_name {
        js_sys::Reflect::set(
            &obj,
            &JsValue::from_str("userDisplayName"),
            &JsValue::from_str(user_display_name),
        )
        .map_err(js_error_to_prf_provider_error)?;
    }

    Ok(obj.into())
}

/// Parse the JS `RegisteredCredential` returned by the provider's
/// `createPasskey` into the Rust core type. Tolerates `aaguid` /
/// `backupEligible` being missing or null since some platforms can't
/// surface them (Safari without `getAuthenticatorData()`).
fn parse_registered_credential(value: &JsValue) -> Result<RegisteredCredential, PrfProviderError> {
    let credential_id_raw = js_sys::Reflect::get(value, &JsValue::from_str("credentialId"))
        .map_err(js_error_to_prf_provider_error)?;
    if credential_id_raw.is_undefined() || credential_id_raw.is_null() {
        return Err(PrfProviderError::Generic(
            "createPasskey result missing credentialId".to_string(),
        ));
    }
    let credential_id = js_sys::Uint8Array::new(&credential_id_raw).to_vec();

    let aaguid = js_sys::Reflect::get(value, &JsValue::from_str("aaguid"))
        .ok()
        .and_then(|v| (!v.is_null() && !v.is_undefined()).then_some(v))
        .map(|v| js_sys::Uint8Array::new(&v).to_vec());

    let backup_eligible = js_sys::Reflect::get(value, &JsValue::from_str("backupEligible"))
        .ok()
        .and_then(|v| v.as_bool());

    Ok(RegisteredCredential {
        credential_id,
        aaguid,
        backup_eligible,
    })
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
 *     async deriveSeeds(salts: string[]): Promise<Uint8Array[]> {
 *         const out: Uint8Array[] = [];
 *         for (const salt of salts) {
 *             const credential = await navigator.credentials.get({
 *                 publicKey: {
 *                     challenge: new Uint8Array(32),
 *                     rpId: window.location.hostname,
 *                     allowCredentials: [],
 *                     extensions: {
 *                         prf: { eval: { first: new TextEncoder().encode(salt) } }
 *                     }
 *                 }
 *             });
 *             const ext = credential.getClientExtensionResults();
 *             out.push(new Uint8Array(ext.prf.results.first));
 *         }
 *         return out;
 *     }
 *
 *     async isSupported(): Promise<boolean> {
 *         return window.PublicKeyCredential?.isUserVerifyingPlatformAuthenticatorAvailable?.() ?? false;
 *     }
 * }
 * ```
 */
export interface PrfProvider {
    /**
     * Derive 32-byte PRF outputs for one or more salts. Implementations
     * with bulk capability (e.g. WebAuthn dual-salt via
     * `prf.eval.first` + `prf.eval.second`) should pack two salts per
     * ceremony where supported; otherwise loop the single-salt path
     * internally. Output ordering matches input ordering.
     *
     * @param salts - Salt strings in caller order
     * @returns A Promise resolving to one 32-byte output per salt
     */
    deriveSeeds(salts: string[]): Promise<Uint8Array[]>;

    /**
     * Optional explicit registration. Platform passkey providers
     * (browser WebAuthn, iOS / Android) implement this to drive the OS
     * create ceremony and return credential metadata (`credentialId`,
     * optional `aaguid`, optional `backupEligible`) that callers need
     * for `excludeCredentialIds` bookkeeping. Custom providers without
     * an explicit creation step can omit this method.
     *
     * @throws `PasskeyAlreadyExistsError` when an entry in
     *   `excludeCredentialIds` matches a credential already on the
     *   device.
     */
    createPasskey?(request: CreatePasskeyRequestJSON): Promise<RegisteredCredentialJSON>;

    /**
     * Whether this provider can produce PRF outputs on the current
     * device. Hosts gate UX on the result.
     */
    isSupported(): Promise<boolean>;
}

/**
 * Plain-object shape passed to {@link PrfProvider.createPasskey}. The
 * bundled `PasskeyProvider` accepts the same shape under the name
 * `CreatePasskeyRequest`; this name is reserved for the Rust-bridge
 * boundary.
 */
export interface CreatePasskeyRequestJSON {
    excludeCredentialIds?: Uint8Array[];
    userId?: Uint8Array;
    userName?: string;
    userDisplayName?: string;
}

/**
 * Plain-object shape returned by {@link PrfProvider.createPasskey}. The
 * bundled `PasskeyProvider` returns a `RegisteredCredential` with the
 * same shape; this name is reserved for the Rust-bridge boundary.
 */
export interface RegisteredCredentialJSON {
    credentialId: Uint8Array;
    aaguid?: Uint8Array | null;
    backupEligible?: boolean | null;
}"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "PrfProvider")]
    pub type PrfProvider;

    #[wasm_bindgen(structural, method, js_name = "isSupported", catch)]
    pub fn is_supported(this: &PrfProvider) -> Result<Promise, JsValue>;
}
