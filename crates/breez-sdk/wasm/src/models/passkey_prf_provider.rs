use std::sync::OnceLock;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, js_sys::Promise};

use breez_sdk_spark::passkey::{DeriveSeedsRequest, PrfProviderError, RegisteredCredential};

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
            "PasskeyCredentialNotFoundError" => {
                let message = js_sys::Reflect::get(&js_error, &JsValue::from_str("message"))
                    .ok()
                    .and_then(|v| v.as_string())
                    .unwrap_or_else(|| "Credential not found".to_string());
                return PrfProviderError::CredentialNotFound(message);
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
    /// Cached `createPasskey` presence probe: JS providers may omit
    /// it (only platform passkey backends implement registration).
    supports_create: OnceLock<bool>,
    /// Cached `takeLastObservedCredentialId` presence probe: JS
    /// providers may omit it (only the bundled platform-passkey
    /// provider currently implements the read-and-clear slot).
    supports_take_last_observed: OnceLock<bool>,
    /// Cached presence probes for the three known-credential methods.
    /// Custom providers (no built-in registry) may omit them and
    /// inherit the trait defaults (empty list / no-op writes).
    supports_get_known: OnceLock<bool>,
    supports_remove_known: OnceLock<bool>,
    supports_clear_known: OnceLock<bool>,
}

impl WasmPrfProvider {
    pub fn new(inner: PrfProvider) -> Self {
        Self {
            inner,
            supports_create: OnceLock::new(),
            supports_take_last_observed: OnceLock::new(),
            supports_get_known: OnceLock::new(),
            supports_remove_known: OnceLock::new(),
            supports_clear_known: OnceLock::new(),
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
    ) -> Result<Vec<Vec<u8>>, PrfProviderError> {
        let salts_array = js_sys::Array::new();
        for salt in &request.salts {
            salts_array.push(&JsValue::from_str(salt));
        }

        // Build the per-call options object so JS providers can apply
        // allow-list and immediate-mediation overrides without
        // reconstructing themselves.
        let options = build_derive_seeds_options(&request)?;

        let target: &JsValue = self.inner.as_ref();
        let func = js_sys::Reflect::get(target, &JsValue::from_str("deriveSeeds"))
            .map_err(js_error_to_prf_provider_error)?
            .dyn_into::<js_sys::Function>()
            .map_err(|_| PrfProviderError::Generic("deriveSeeds is not a function".to_string()))?;
        let result_promise = func
            .call2(target, &salts_array, &options)
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
        if len != request.salts.len() {
            return Err(PrfProviderError::Generic(format!(
                "deriveSeeds returned {} outputs, expected {}",
                len,
                request.salts.len()
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

    async fn take_last_observed_credential_id(&self) -> Option<Vec<u8>> {
        if !self.js_has_method(
            "takeLastObservedCredentialId",
            &self.supports_take_last_observed,
        ) {
            return None;
        }
        let target: &JsValue = self.inner.as_ref();
        let func = js_sys::Reflect::get(target, &JsValue::from_str("takeLastObservedCredentialId"))
            .ok()?
            .dyn_into::<js_sys::Function>()
            .ok()?;
        let raw = func.call0(target).ok()?;
        if raw.is_undefined() || raw.is_null() {
            return None;
        }
        Some(js_sys::Uint8Array::new(&raw).to_vec())
    }

    async fn create_passkey(
        &self,
        exclude_credential_ids: Vec<Vec<u8>>,
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

        let js_exclude = build_exclude_credential_ids(&exclude_credential_ids);
        let result_promise = func
            .call1(target, &js_exclude)
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

    async fn get_known_credential_ids(&self) -> Result<Vec<Vec<u8>>, PrfProviderError> {
        if !self.js_has_method("getKnownCredentialIds", &self.supports_get_known) {
            return Ok(vec![]);
        }
        let target: &JsValue = self.inner.as_ref();
        let func = js_sys::Reflect::get(target, &JsValue::from_str("getKnownCredentialIds"))
            .map_err(js_error_to_prf_provider_error)?
            .dyn_into::<js_sys::Function>()
            .map_err(|_| {
                PrfProviderError::Generic("getKnownCredentialIds is not a function".to_string())
            })?;
        let promise = func
            .call0(target)
            .map_err(js_error_to_prf_provider_error)?
            .dyn_into::<Promise>()
            .map_err(|_| {
                PrfProviderError::Generic(
                    "getKnownCredentialIds did not return a Promise".to_string(),
                )
            })?;
        let result = JsFuture::from(promise)
            .await
            .map_err(js_error_to_prf_provider_error)?;
        let array = js_sys::Array::from(&result);
        let len = array.length() as usize;
        let mut out = Vec::with_capacity(len);
        for i in 0..array.length() {
            let item = array.get(i);
            out.push(js_sys::Uint8Array::new(&item).to_vec());
        }
        Ok(out)
    }

    async fn remove_known_credential_id(&self, id: Vec<u8>) -> Result<(), PrfProviderError> {
        if !self.js_has_method("removeKnownCredentialId", &self.supports_remove_known) {
            return Ok(());
        }
        let target: &JsValue = self.inner.as_ref();
        let func = js_sys::Reflect::get(target, &JsValue::from_str("removeKnownCredentialId"))
            .map_err(js_error_to_prf_provider_error)?
            .dyn_into::<js_sys::Function>()
            .map_err(|_| {
                PrfProviderError::Generic("removeKnownCredentialId is not a function".to_string())
            })?;
        let arg = js_sys::Uint8Array::from(id.as_slice());
        let promise = func
            .call1(target, &arg)
            .map_err(js_error_to_prf_provider_error)?
            .dyn_into::<Promise>()
            .map_err(|_| {
                PrfProviderError::Generic(
                    "removeKnownCredentialId did not return a Promise".to_string(),
                )
            })?;
        JsFuture::from(promise)
            .await
            .map_err(js_error_to_prf_provider_error)?;
        Ok(())
    }

    async fn clear_known_credential_ids(&self) -> Result<(), PrfProviderError> {
        if !self.js_has_method("clearKnownCredentialIds", &self.supports_clear_known) {
            return Ok(());
        }
        let target: &JsValue = self.inner.as_ref();
        let func = js_sys::Reflect::get(target, &JsValue::from_str("clearKnownCredentialIds"))
            .map_err(js_error_to_prf_provider_error)?
            .dyn_into::<js_sys::Function>()
            .map_err(|_| {
                PrfProviderError::Generic("clearKnownCredentialIds is not a function".to_string())
            })?;
        let promise = func
            .call0(target)
            .map_err(js_error_to_prf_provider_error)?
            .dyn_into::<Promise>()
            .map_err(|_| {
                PrfProviderError::Generic(
                    "clearKnownCredentialIds did not return a Promise".to_string(),
                )
            })?;
        JsFuture::from(promise)
            .await
            .map_err(js_error_to_prf_provider_error)?;
        Ok(())
    }
}

/// Marshal a [`DeriveSeedsRequest`]'s per-call overrides into a JS
/// options object shaped per `index.d.ts#DeriveSeedOptions`. The
/// `salts` field is passed positionally; this object only carries
/// the optional shaping fields. Returned even when empty so the JS
/// provider always receives a well-formed second argument.
fn build_derive_seeds_options(request: &DeriveSeedsRequest) -> Result<JsValue, PrfProviderError> {
    let obj = js_sys::Object::new();

    if !request.allow_credential_ids.is_empty() {
        let arr = js_sys::Array::new();
        for id in &request.allow_credential_ids {
            arr.push(&js_sys::Uint8Array::from(id.as_slice()));
        }
        js_sys::Reflect::set(&obj, &JsValue::from_str("allowCredentialIds"), &arr)
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

/// Marshal `exclude_credential_ids` into a JS `Uint8Array[]` for the
/// `createPasskey(excludeCredentialIds)` provider call. `Vec<u8>`
/// entries cross as `Uint8Array` (not plain arrays) to match what
/// `navigator.credentials.create` expects.
fn build_exclude_credential_ids(exclude_credential_ids: &[Vec<u8>]) -> JsValue {
    let arr = js_sys::Array::new();
    for id in exclude_credential_ids {
        arr.push(&js_sys::Uint8Array::from(id.as_slice()));
    }
    arr.into()
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

    let user_id_raw = js_sys::Reflect::get(value, &JsValue::from_str("userId"))
        .map_err(js_error_to_prf_provider_error)?;
    if user_id_raw.is_undefined() || user_id_raw.is_null() {
        return Err(PrfProviderError::Generic(
            "createPasskey result missing userId".to_string(),
        ));
    }
    let user_id = js_sys::Uint8Array::new(&user_id_raw).to_vec();

    let aaguid = js_sys::Reflect::get(value, &JsValue::from_str("aaguid"))
        .ok()
        .and_then(|v| (!v.is_null() && !v.is_undefined()).then_some(v))
        .map(|v| js_sys::Uint8Array::new(&v).to_vec());

    let backup_eligible = js_sys::Reflect::get(value, &JsValue::from_str("backupEligible"))
        .ok()
        .and_then(|v| v.as_bool());

    Ok(RegisteredCredential {
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
     * The optional `options` object carries per-call ceremony shapers
     * (allow-list, immediate-mediation toggle). Built-in providers
     * forward them to the underlying WebAuthn / Credential Manager
     * call; custom providers without an OS picker can ignore them.
     *
     * @param salts - Salt strings in caller order
     * @param options - Optional per-call overrides
     * @returns A Promise resolving to one 32-byte output per salt
     */
    deriveSeeds(salts: string[], options?: DeriveSeedsOptionsJSON): Promise<Uint8Array[]>;

    /**
     * Optional explicit registration. Platform passkey providers
     * (browser WebAuthn, iOS / Android) implement this to drive the OS
     * create ceremony and return credential metadata (`credentialId`,
     * `userId`, optional `aaguid`, optional `backupEligible`) that
     * callers need for `excludeCredentialIds` bookkeeping and
     * server-side correlation. Custom providers without an explicit
     * creation step can omit this method.
     *
     * `excludeCredentialIds` is the only per-call knob: when any entry
     * matches a credential already on the device, the provider raises
     * `PasskeyAlreadyExistsError`. Branding fields (rpName, userName,
     * userDisplayName) live on the provider constructor.
     *
     * @throws `PasskeyAlreadyExistsError` when an entry in
     *   `excludeCredentialIds` matches a credential already on the
     *   device.
     */
    createPasskey?(excludeCredentialIds: Uint8Array[]): Promise<RegisteredCredentialJSON>;

    /**
     * Whether this provider can produce PRF outputs on the current
     * device. Hosts gate UX on the result.
     */
    isSupported(): Promise<boolean>;

    /**
     * Optional. Backs `PasskeyClient.credentials().get()`. Implementations
     * with a persistent registry (browser `CredentialRegistry`, native
     * `KnownCredentialsStore`) return the stored credential-ID set for
     * the current RP; custom providers may omit and inherit the
     * empty-list default.
     */
    getKnownCredentialIds?(): Promise<Uint8Array[]>;

    /**
     * Optional. Backs `PasskeyClient.credentials().remove(id)`. Drops
     * a single ID from the persisted registry. Omit on providers
     * without a registry (no-op default).
     */
    removeKnownCredentialId?(credentialId: Uint8Array): Promise<void>;

    /**
     * Optional. Backs `PasskeyClient.credentials().clear()`. Clears
     * the persisted credential-ID set for the current RP. Omit on
     * providers without a registry (no-op default).
     */
    clearKnownCredentialIds?(): Promise<void>;
}

/**
 * Plain-object shape passed as the second argument of
 * {@link PrfProvider.deriveSeeds}. Mirrors the bundled
 * `PasskeyProvider`'s `DeriveSeedOptions` so JS providers can apply
 * the same shaping fields the Rust-side `DeriveSeedsRequest` carries.
 */
export interface DeriveSeedsOptionsJSON {
    /** Allow-list; empty / omitted lets the provider default apply. */
    allowCredentialIds?: Uint8Array[];
    /**
     * Controls the platform's "fast-fail when no local credential is
     * available" behavior. On the web this maps to the WebAuthn
     * `mediation: 'immediate'` / `uiMode: 'immediate'` flag: `true`
     * opts into immediate mediation when the browser advertises the
     * capability; `false` falls back to the standard picker
     * (cross-device QR, hybrid transports). Omitted lets the provider
     * default apply.
     */
    preferImmediatelyAvailableCredentials?: boolean;
}

/**
 * Plain-object shape returned by {@link PrfProvider.createPasskey}. The
 * bundled `PasskeyProvider` returns a `RegisteredCredential` with the
 * same shape; this name is reserved for the Rust-bridge boundary.
 *
 * `userId` is the WebAuthn user handle the provider generated for this
 * credential. Always returned; never host-supplied.
 */
export interface RegisteredCredentialJSON {
    credentialId: Uint8Array;
    userId: Uint8Array;
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
