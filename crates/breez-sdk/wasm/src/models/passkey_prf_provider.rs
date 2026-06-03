use std::sync::OnceLock;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, js_sys::Promise};

use breez_sdk_spark::passkey::{
    DeriveSeedsOutput, DeriveSeedsRequest, PasskeyCredential, PrfProviderError,
};

pub(crate) fn js_error_to_prf_provider_error(js_error: JsValue) -> PrfProviderError {
    // Map typed JS error classes thrown by the bundled JS provider back to
    // the typed Rust variant so callers don't have to substring-match
    // `error.message`. The provider's classes extend Error and set both
    // `name` and `message`, so read them from the structured Error object;
    // a bare string throw falls back to the string itself. The real message
    // is always surfaced, never replaced by a canned string.
    let (name, message) = match js_error.dyn_ref::<js_sys::Error>() {
        Some(err) => (Some(String::from(err.name())), String::from(err.message())),
        None => (None, js_error.as_string().unwrap_or_default()),
    };
    let message = if message.is_empty() {
        "Passkey PRF error occurred".to_string()
    } else {
        message
    };

    match name.as_deref() {
        Some("PasskeyAlreadyExistsError") => PrfProviderError::CredentialAlreadyExists(message),
        Some("PasskeyTimedOutError") => PrfProviderError::UserTimedOut,
        Some("PasskeyUserCancelledError") => PrfProviderError::UserCancelled,
        Some("PasskeyCredentialNotFoundError") => PrfProviderError::CredentialNotFound(message),
        _ => PrfProviderError::Generic(message),
    }
}

pub struct WasmPrfProvider {
    pub inner: PrfProvider,
    /// Cached `createPasskey` presence probe: JS providers may omit
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
    async fn derive_seeds(
        &self,
        request: DeriveSeedsRequest,
    ) -> Result<DeriveSeedsOutput, PrfProviderError> {
        let salts_array = js_sys::Array::new();
        for salt in &request.salts {
            salts_array.push(&JsValue::from_str(salt));
        }

        // Build the per-call options object so JS providers can apply
        // allow-list and immediate-mediation overrides without
        // reconstructing themselves.
        let options = build_derive_seeds_options(&request)?;

        let result_promise = self
            .inner
            .derive_seeds(salts_array, options)
            .map_err(js_error_to_prf_provider_error)?;
        let result = JsFuture::from(result_promise)
            .await
            .map_err(js_error_to_prf_provider_error)?;

        // The JS provider resolves to `{ seeds, credentialId }`: seeds
        // in input order plus the credential ID observed in the same
        // assertion (null when the provider does not surface one).
        let seeds_raw = js_sys::Reflect::get(&result, &JsValue::from_str("seeds"))
            .map_err(js_error_to_prf_provider_error)?;
        let array = js_sys::Array::from(&seeds_raw);
        let len = array.length() as usize;
        if len != request.salts.len() {
            return Err(PrfProviderError::Generic(format!(
                "deriveSeeds returned {} outputs, expected {}",
                len,
                request.salts.len()
            )));
        }
        let mut seeds = Vec::with_capacity(len);
        for i in 0..array.length() {
            let item = array.get(i);
            seeds.push(js_sys::Uint8Array::new(&item).to_vec());
        }

        let credential_id = js_sys::Reflect::get(&result, &JsValue::from_str("credentialId"))
            .ok()
            .and_then(|v| (!v.is_null() && !v.is_undefined()).then_some(v))
            .map(|v| js_sys::Uint8Array::new(&v).to_vec());

        Ok(DeriveSeedsOutput {
            seeds,
            credential_id,
        })
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
        exclude_credentials: Vec<Vec<u8>>,
    ) -> Result<PasskeyCredential, PrfProviderError> {
        // Custom providers may not implement explicit creation; fall
        // back to the trait default (`PrfNotSupported`).
        if !self.js_has_method("createPasskey", &self.supports_create) {
            return Err(PrfProviderError::PrfNotSupported);
        }

        let js_exclude = build_exclude_credentials(&exclude_credentials);
        let result_promise = self
            .inner
            .create_passkey(js_exclude)
            .map_err(js_error_to_prf_provider_error)?;
        let result = JsFuture::from(result_promise)
            .await
            .map_err(js_error_to_prf_provider_error)?;

        parse_passkey_credential(&result)
    }
}

/// Marshal a [`DeriveSeedsRequest`]'s per-call overrides into a JS
/// options object shaped per `index.d.ts#DeriveSeedOptions`. The
/// `salts` field is passed positionally; this object only carries
/// the optional shaping fields. Returned even when empty so the JS
/// provider always receives a well-formed second argument.
fn build_derive_seeds_options(request: &DeriveSeedsRequest) -> Result<JsValue, PrfProviderError> {
    let obj = js_sys::Object::new();

    if !request.allow_credentials.is_empty() {
        let arr = js_sys::Array::new();
        for id in &request.allow_credentials {
            arr.push(&js_sys::Uint8Array::from(id.as_slice()));
        }
        js_sys::Reflect::set(&obj, &JsValue::from_str("allowCredentials"), &arr)
            .map_err(js_error_to_prf_provider_error)?;
    }
    if let Some(prefer) = request.prefer_immediately_available_credentials {
        js_sys::Reflect::set(
            &obj,
            &JsValue::from_str("preferImmediatelyAvailableCredentials"),
            &JsValue::from_bool(prefer),
        )
        .map_err(js_error_to_prf_provider_error)?;
    }

    Ok(obj.into())
}

/// Marshal `exclude_credentials` into a JS `Uint8Array[]` for the
/// `createPasskey(excludeCredentials)` provider call. `Vec<u8>`
/// entries cross as `Uint8Array` (not plain arrays) to match what
/// `navigator.credentials.create` expects.
fn build_exclude_credentials(exclude_credentials: &[Vec<u8>]) -> JsValue {
    let arr = js_sys::Array::new();
    for id in exclude_credentials {
        arr.push(&js_sys::Uint8Array::from(id.as_slice()));
    }
    arr.into()
}

/// Parse the JS credential object returned by the provider's
/// `createPasskey` into the Rust core type. `credentialId` is required;
/// the attestation fields tolerate missing / null values since some
/// platforms can't surface them (Safari without `getAuthenticatorData()`).
fn parse_passkey_credential(value: &JsValue) -> Result<PasskeyCredential, PrfProviderError> {
    let credential_id_raw = js_sys::Reflect::get(value, &JsValue::from_str("credentialId"))
        .map_err(js_error_to_prf_provider_error)?;
    if credential_id_raw.is_undefined() || credential_id_raw.is_null() {
        return Err(PrfProviderError::Generic(
            "createPasskey result missing credentialId".to_string(),
        ));
    }
    let credential_id = js_sys::Uint8Array::new(&credential_id_raw).to_vec();

    let user_id = js_sys::Reflect::get(value, &JsValue::from_str("userId"))
        .ok()
        .and_then(|v| (!v.is_null() && !v.is_undefined()).then_some(v))
        .map(|v| js_sys::Uint8Array::new(&v).to_vec());

    let aaguid = js_sys::Reflect::get(value, &JsValue::from_str("aaguid"))
        .ok()
        .and_then(|v| (!v.is_null() && !v.is_undefined()).then_some(v))
        .map(|v| js_sys::Uint8Array::new(&v).to_vec());

    let backup_eligible = js_sys::Reflect::get(value, &JsValue::from_str("backupEligible"))
        .ok()
        .and_then(|v| v.as_bool());

    Ok(PasskeyCredential {
        credential_id,
        user_id,
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
 */
export interface PrfProvider {
    /**
     * Derive 32-byte PRF outputs for one or more salts, in input order.
     * Implementations with bulk capability (WebAuthn dual-salt via
     * `prf.eval.first` + `prf.eval.second`) pack two salts per ceremony
     * where supported; others loop the single-salt path. `options` carries
     * per-call ceremony shapers that built-in providers forward to the OS
     * call and custom providers may ignore.
     *
     * @param salts - Salt strings in caller order
     * @param options - Optional per-call overrides
     * @returns Promise of the 32-byte outputs in input order plus the
     *   credential ID observed in the same assertion (`null` when the
     *   provider surfaces none, e.g. sources without an OS picker)
     */
    deriveSeeds(salts: string[], options?: DeriveSeedOptions): Promise<DeriveSeedsResult>;

    /**
     * Optional explicit registration. Platform passkey providers (browser
     * WebAuthn, iOS / Android) implement this to drive the OS create
     * ceremony and return credential metadata (`credentialId`, `userId`,
     * optional `aaguid` / `backupEligible`) callers need for
     * `excludeCredentials` bookkeeping and server-side correlation. Custom
     * providers without a creation step can omit it.
     *
     * `excludeCredentials` lists already-registered IDs; a match raises
     * `PasskeyAlreadyExistsError`.
     *
     * @throws `PasskeyAlreadyExistsError` when an entry in
     *   `excludeCredentials` matches a credential already on the device.
     */
    createPasskey?(excludeCredentials: Uint8Array[]): Promise<PasskeyCredential>;

    /**
     * Whether this provider can produce PRF outputs on the current
     * device. Hosts gate UX on the result.
     */
    isSupported(): Promise<boolean>;
}

/**
 * Per-call options passed as the second argument of
 * {@link PrfProvider.deriveSeeds}.
 */
export interface DeriveSeedOptions {
    /**
     * Credential IDs the assertion is restricted to, for reauthenticating
     * a known user without an account picker. Empty or unset lets the
     * provider's default apply.
     */
    allowCredentials?: Uint8Array[];
    /**
     * Fast-fail when no local credential is available. On the web this maps
     * to WebAuthn `mediation: 'immediate'`: `true` opts in where the browser
     * advertises support, `false` uses the standard picker. Unset uses the
     * provider default.
     */
    preferImmediatelyAvailableCredentials?: boolean;
}

/**
 * Returned by {@link PrfProvider.deriveSeeds}: the derived 32-byte outputs
 * in input order plus the credential ID observed in the same assertion.
 * `credentialId` is `null` when the provider does not surface one (custom
 * deterministic sources without an OS picker).
 */
export interface DeriveSeedsResult {
    seeds: Uint8Array[];
    credentialId?: Uint8Array | null;
}"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "PrfProvider")]
    pub type PrfProvider;

    #[wasm_bindgen(structural, method, js_name = "deriveSeeds", catch)]
    pub fn derive_seeds(
        this: &PrfProvider,
        salts: js_sys::Array,
        options: JsValue,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "isSupported", catch)]
    pub fn is_supported(this: &PrfProvider) -> Result<Promise, JsValue>;

    // Optional method. Custom providers may omit it; callers probe
    // with `js_has_method` before invoking.
    #[wasm_bindgen(structural, method, js_name = "createPasskey", catch)]
    pub fn create_passkey(
        this: &PrfProvider,
        exclude_credentials: JsValue,
    ) -> Result<Promise, JsValue>;
}
