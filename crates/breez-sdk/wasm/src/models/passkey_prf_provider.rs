use std::sync::OnceLock;

use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, js_sys::Promise};

use breez_sdk_spark::passkey::{
    DeriveSeedsOutput, DeriveSeedsRequest, PrfProviderError, RegisteredCredential,
};

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
            "PasskeyUserCancelledError" => {
                return PrfProviderError::UserCancelled;
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
    ) -> Result<RegisteredCredential, PrfProviderError> {
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

        parse_registered_credential(&result)
    }

    async fn get_known_credential_ids(&self) -> Result<Vec<Vec<u8>>, PrfProviderError> {
        if !self.js_has_method("getKnownCredentialIds", &self.supports_get_known) {
            return Ok(vec![]);
        }
        let promise = self
            .inner
            .get_known_credential_ids()
            .map_err(js_error_to_prf_provider_error)?;
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
        let arg = js_sys::Uint8Array::from(id.as_slice());
        let promise = self
            .inner
            .remove_known_credential_id(arg)
            .map_err(js_error_to_prf_provider_error)?;
        JsFuture::from(promise)
            .await
            .map_err(js_error_to_prf_provider_error)?;
        Ok(())
    }

    async fn clear_known_credential_ids(&self) -> Result<(), PrfProviderError> {
        if !self.js_has_method("clearKnownCredentialIds", &self.supports_clear_known) {
            return Ok(());
        }
        let promise = self
            .inner
            .clear_known_credential_ids()
            .map_err(js_error_to_prf_provider_error)?;
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
     * Resolves to `{ seeds, credentialId }`: the 32-byte outputs in
     * input order plus the credential ID observed in the same assertion
     * (`null` when the provider does not surface one). The SDK reads
     * `credentialId` to surface the signed-in credential to callers, so
     * providers without an OS picker may return `null`.
     *
     * @param salts - Salt strings in caller order
     * @param options - Optional per-call overrides
     * @returns A Promise resolving to the seeds plus observed credential ID
     */
    deriveSeeds(salts: string[], options?: DeriveSeedsOptionsJSON): Promise<DeriveSeedsResultJSON>;

    /**
     * Optional explicit registration. Platform passkey providers
     * (browser WebAuthn, iOS / Android) implement this to drive the OS
     * create ceremony and return credential metadata (`credentialId`,
     * `userId`, optional `aaguid`, optional `backupEligible`) that
     * callers need for `excludeCredentials` bookkeeping and
     * server-side correlation. Custom providers without an explicit
     * creation step can omit this method.
     *
     * `excludeCredentials` is a list of already-registered credential
     * IDs. Prevents registering the same device twice: when any entry
     * matches a credential already on the device, the provider raises
     * `PasskeyAlreadyExistsError`. Branding fields (rpName, userName,
     * userDisplayName) live on the provider constructor.
     *
     * @throws `PasskeyAlreadyExistsError` when an entry in
     *   `excludeCredentials` matches a credential already on the
     *   device.
     */
    createPasskey?(excludeCredentials: Uint8Array[]): Promise<RegisteredCredentialJSON>;

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
    /**
     * A list of credential IDs the assertion is restricted to. The
     * primary use case is reauthentication when the user is already
     * known: if any of the listed credentials is available locally,
     * the platform prompts for device unlock straight away (no
     * account picker); otherwise the user is asked to present another
     * device (paired phone or security key) that holds a valid
     * credential. Empty / omitted lets the provider default apply.
     */
    allowCredentials?: Uint8Array[];
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
 * Plain-object shape returned by {@link PrfProvider.deriveSeeds}: the
 * derived 32-byte outputs in input order plus the credential ID observed
 * in the same assertion. `credentialId` is `null` when the provider does
 * not surface one (custom deterministic sources without an OS picker).
 */
export interface DeriveSeedsResultJSON {
    seeds: Uint8Array[];
    credentialId?: Uint8Array | null;
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

    #[wasm_bindgen(structural, method, js_name = "deriveSeeds", catch)]
    pub fn derive_seeds(
        this: &PrfProvider,
        salts: js_sys::Array,
        options: JsValue,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "isSupported", catch)]
    pub fn is_supported(this: &PrfProvider) -> Result<Promise, JsValue>;

    // Optional methods. Custom providers may omit them; callers probe
    // with `js_has_method` before invoking.
    #[wasm_bindgen(structural, method, js_name = "createPasskey", catch)]
    pub fn create_passkey(
        this: &PrfProvider,
        exclude_credentials: JsValue,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "getKnownCredentialIds", catch)]
    pub fn get_known_credential_ids(this: &PrfProvider) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "removeKnownCredentialId", catch)]
    pub fn remove_known_credential_id(
        this: &PrfProvider,
        id: js_sys::Uint8Array,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "clearKnownCredentialIds", catch)]
    pub fn clear_known_credential_ids(this: &PrfProvider) -> Result<Promise, JsValue>;
}
