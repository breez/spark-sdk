/**
 * Built-in passkey-based PRF provider for browser environments.
 *
 * Implements the PrfProvider interface using the WebAuthn API with the PRF
 * extension (navigator.credentials.create/get).
 *
 * Uses discoverable credentials (resident keys) so no credential storage is needed.
 * The credential lives on the authenticator and is discovered by rpId.
 *
 * On first use, if no credential exists for the RP ID, a new passkey is
 * automatically created (registered), then the assertion is retried.
 *
 * @example
 * ```javascript
 * import { PasskeyClient } from '@breeztech/breez-sdk-spark'
 * import { PasskeyProvider } from '@breeztech/breez-sdk-spark/passkey-prf-provider'
 *
 * const provider = new PasskeyProvider()
 * const client = new PasskeyClient(provider)
 * const { wallet } = await client.signIn({ label: 'personal' })
 * ```
 */

import { PasskeyClient as SdkPasskeyClient } from '../breez_sdk_spark_wasm.js';

/** Breez's shared RP ID, exposed as `PasskeyProvider.BREEZ_RP_ID`. */
const BREEZ_RP_ID = 'keys.breez.technology';

/** Default `rpName` for the zero-config {@link PasskeyClient} path. */
const DEFAULT_RP_NAME = 'Breez';

// WebAuthn collapses "no matching credential" and "user dismissed" into
// one NotAllowedError, but a no-credential fast-fail resolves before any
// UI shows while a dismiss takes seconds. Elapsed time below this is
// classified as no-credential, at or above as user-cancel.
const NO_CRED_FAST_FAIL_MS = 250;

// iOS and Android tear the biometric sheet down around 55s of inactivity
// with the same generic NotAllowedError. Elapsed time at or above this is
// reclassified as a timeout rather than a user-cancel.
const BIOMETRIC_TIMEOUT_MS = 55_000;

/**
 * Generate cryptographically random bytes.
 * @param {number} length
 * @returns {Uint8Array}
 */
function randomBytes(length) {
    const bytes = new Uint8Array(length);
    crypto.getRandomValues(bytes);
    return bytes;
}

/**
 * Extract AAGUID + BE flag from a successful create response via the
 * WebAuthn Level 2 `getAuthenticatorData()` accessor.
 *
 * authData layout when AT flag is set (always on a successful create):
 *   [32]      flags (UP=0, UV=2, BE=3, BS=4, AT=6)
 *   [37..53)  AAGUID (16 bytes)
 *
 * @param {PublicKeyCredential} credential
 * @returns {{ aaguid: Uint8Array, backupEligible: boolean } | null}
 */
function extractRegistrationMetadata(credential) {
    try {
        const response = credential.response;
        if (!response || typeof response.getAuthenticatorData !== 'function') {
            return null;
        }
        const authData = new Uint8Array(response.getAuthenticatorData());
        if (authData.length < 53) return null;
        const flags = authData[32];
        const hasAttestedCredData = (flags & 0x40) !== 0;
        if (!hasAttestedCredData) return null;
        const backupEligible = (flags & 0x08) !== 0;
        const aaguid = authData.slice(37, 53);
        return { aaguid, backupEligible };
    } catch {
        return null;
    }
}

/**
 * Thrown when `createPasskey` asks the platform to register a new
 * passkey but it refuses because an entry in `excludeCredentials`
 * matches a credential already on the device. Hosts should route the
 * user to the sign-in path instead of treating this as a generic
 * registration failure.
 */
export class PasskeyAlreadyExistsError extends Error {
    constructor(message = 'A passkey for this RP already exists on this device') {
        super(message);
        this.name = 'PasskeyAlreadyExistsError';
    }
}

/**
 * Thrown when the OS biometric prompt tears down without the user
 * approving or dismissing it: the platform's inactivity timeout
 * (typically ~55 seconds) fired before any user interaction. Distinct
 * from a cancel: the user did not actively abandon the flow, so hosts
 * may auto-retry or surface a re-prompt UI without treating it as
 * user intent to abandon.
 */
export class PasskeyTimedOutError extends Error {
    constructor(message = 'Authenticator timed out') {
        super(message);
        this.name = 'PasskeyTimedOutError';
    }
}

/**
 * Thrown when `deriveSeeds` cannot match a credential. Surfaces both
 * the WebAuthn fast-fail `NotAllowedError` (no credential on this
 * device for the RP) and the bare "no credential available" path.
 * `error.message` carries diagnostic detail and may include the
 * `CredentialRegistry` help suffix when the host had no allow-list
 * and no registry configured.
 */
export class PasskeyCredentialNotFoundError extends Error {
    constructor(message = 'Credential not found') {
        super(message);
        this.name = 'PasskeyCredentialNotFoundError';
    }
}

/**
 * Thrown when the user actively dismisses the OS passkey prompt.
 * Distinct from `PasskeyTimedOutError` (the prompt timed out with no
 * interaction): hosts should not auto-retry a deliberate cancel.
 */
export class PasskeyUserCancelledError extends Error {
    constructor(message = 'User cancelled authentication') {
        super(message);
        this.name = 'PasskeyUserCancelledError';
    }
}

function _bytesToBase64Url(bytes) {
    let s = '';
    for (const b of bytes) s += String.fromCharCode(b);
    return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

function _base64UrlToBytes(s) {
    const pad = s.length % 4 === 0 ? 0 : 4 - (s.length % 4);
    const b64 = s.replace(/-/g, '+').replace(/_/g, '/') + '='.repeat(pad);
    const bin = atob(b64);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
}

/** Hard cap on registry calls. Failures are logged and reported via
 *  `onRegistryError`; the ceremony proceeds either way.
 */
const REGISTRY_TIMEOUT_MS = 3_000;

/** Suffix appended to a `Credential not found` error when the host
 *  had no `allowCredentials` and no `CredentialRegistry`. Lets
 *  integrators discover the opt-in auto-discovery path from the error.
 */
const CREDENTIAL_REGISTRY_HELP_SUFFIX =
    ' (No CredentialRegistry was supplied to PasskeyProvider; '
    + 'if you expect the SDK to auto-discover known credentials, see '
    + 'https://sdk-doc-spark.breez.technology/guide/passkey.html#credentialregistry)';

/** Sentinel used to distinguish a timeout from a thrown error. */
const _REGISTRY_TIMEOUT = Symbol('registryTimeout');

function _withRegistryTimeout(promise) {
    return Promise.race([
        promise,
        new Promise((resolve) => setTimeout(() => resolve(_REGISTRY_TIMEOUT), REGISTRY_TIMEOUT_MS)),
    ]);
}

async function _registryReadBestEffort(registry, rpId, onRegistryError) {
    try {
        const result = await _withRegistryTimeout(registry.read(rpId));
        if (result === _REGISTRY_TIMEOUT) {
            const err = new Error('CredentialRegistry.read timed out');
            console.warn('[CredentialRegistry] read timed out');
            onRegistryError?.('read', err);
            return [];
        }
        return Array.isArray(result) ? result : [];
    } catch (err) {
        console.warn('[CredentialRegistry] read failed', err);
        onRegistryError?.('read', err);
        return [];
    }
}

function _registryAddFireAndForget(registry, rpId, credentialId, onRegistryError) {
    _withRegistryTimeout(registry.add(rpId, credentialId))
        .then((result) => {
            if (result === _REGISTRY_TIMEOUT) {
                const err = new Error('CredentialRegistry.add timed out');
                console.warn('[CredentialRegistry] add timed out');
                onRegistryError?.('add', err);
            }
        })
        .catch((err) => {
            console.warn('[CredentialRegistry] add failed', err);
            onRegistryError?.('add', err);
        });
}

/**
 * Built-in passkey-based PRF provider for browser environments.
 */
export class PasskeyProvider {
    /**
     * Breez's shared `keys.breez.technology` RP. Pass as `rpId` to opt
     * in (Breez-registered apps only); other apps pass their own domain.
     */
    static get BREEZ_RP_ID() { return BREEZ_RP_ID; }

    /**
     * Default `rpName` for the zero-config {@link PasskeyClient} /
     * {@link PasskeyClientBuilder} path.
     */
    static get DEFAULT_RP_NAME() { return DEFAULT_RP_NAME; }

    /**
     * @param {object} options
     * @param {string} options.rpId - **Required.** Relying Party ID, the
     *   domain hosting your passkeys; on web a registrable suffix of
     *   `window.location.hostname` or equal to it. Pass
     *   `PasskeyProvider.BREEZ_RP_ID` to use Breez's shared RP
     *   (Breez-registered apps only).
     * @param {string} options.rpName - **Required.** WebAuthn `rp.name`,
     *   shown in some credential-manager UIs (iCloud Keychain, Google
     *   Password Manager). Deprecated in L3 but still required by current
     *   browsers, so empty is rejected. Used only at registration.
     * @param {string} [options.userName] - WebAuthn `user.name`, shown in
     *   the sign-in picker. Apple's Passwords app dedupes by
     *   `(rpId, user.name)`, so pass a stable per-user value for distinct
     *   entries. Defaults to `rpName`. Used only at registration.
     * @param {string} [options.userDisplayName] - WebAuthn
     *   `user.displayName`, a label the browser may show in the picker
     *   (behavior varies). Defaults to `userName`. Used only at
     *   registration.
     * @param {'platform'|'cross-platform'} [options.authenticatorAttachment]
     *   Narrows the create-time chooser to one authenticator class.
     *   `'platform'` allows only the local authenticator (Touch ID, Face
     *   ID, Windows Hello, iCloud Keychain); `'cross-platform'` only
     *   roaming keys (USB, NFC, BLE, hybrid). Unset shows all.
     * @param {Array<'client-device'|'security-key'|'hybrid'>} [options.hints]
     *   WebAuthn L3 priority hints applied to both create and get,
     *   ordering the classes a supporting browser offers first (ignored
     *   otherwise). Pass `['client-device']` to favor the platform
     *   authenticator. Only standards-track lever for the sign-in picker,
     *   where `authenticatorAttachment` is not allowed.
     */
    constructor(options) {
        if (!options || typeof options.rpId !== 'string' || options.rpId.length === 0) {
            throw new Error(
                'PasskeyProvider: rpId is required. Pass your app\'s RP domain, '
                + 'or PasskeyProvider.BREEZ_RP_ID if you registered with Breez.'
            );
        }
        if (typeof options.rpName !== 'string' || options.rpName.length === 0) {
            throw new Error(
                'PasskeyProvider: rpName is required. Pass your app name; it is '
                + 'shown to the user in the OS passkey picker.'
            );
        }
        this.rpId = options.rpId;
        this.rpName = options.rpName;
        this.userName = options.userName || this.rpName;
        this.userDisplayName = options.userDisplayName || this.userName;
        this.authenticatorAttachment = options.authenticatorAttachment;
        this.hints = options.hints;
        /**
         * Default WebAuthn `timeout` (milliseconds) for both create()
         * and get() ceremonies. Hint only; platforms cap at ~60s.
         * Pass 5 to 10s under that cap to let a host-side "looks like
         * a timeout" heuristic fire before the OS tears the prompt
         * down.
         * @type {number | undefined}
         */
        this.defaultTimeoutMs = options.defaultTimeoutMs;

        /**
         * Optional persistence for known credential IDs. When set,
         * the provider auto-merges its contents into
         * `excludeCredentials` on create and `allowCredentials` on
         * assert, and writes successful create / assert IDs back.
         * Omit to disable auto-population entirely.
         * @type {CredentialRegistry | undefined}
         */
        this.credentialRegistry = options.credentialRegistry;
        if (this.credentialRegistry) {
            for (const method of ['read', 'add', 'remove', 'clear']) {
                if (typeof this.credentialRegistry[method] !== 'function') {
                    throw new Error(
                        `PasskeyProvider: credentialRegistry is missing a "${method}" `
                        + 'method. Implementations must provide read / add / remove / clear.'
                    );
                }
            }
        }
        /**
         * Callback for `CredentialRegistry` failures. Never blocks the
         * ceremony.
         * @type {((operation: string, error: Error) => void) | undefined}
         */
        this.onRegistryError = options.onRegistryError;

        /**
         * Credential ID asserted in the most recent ceremony. Reset at
         * the start of {@link deriveSeeds}, read into its return value.
         * @private
         */
        this._lastObservedCredentialId = null;
    }

    /**
     * Single-salt seed derivation. Private helper backing
     * {@link deriveSeeds}; the public surface only exposes the bulk
     * form.
     * @private
     */
    async _deriveSeed(salt, options = {}) {
        const saltBytes = new TextEncoder().encode(salt);
        return await this._getAssertionWithPrf(saltBytes, options);
    }

    /**
     * Derive one 32-byte PRF output per salt (in input order), pairing
     * salts into a single ceremony where the authenticator supports it.
     * Worst case is one prompt per salt.
     *
     * @param {string[]} salts - Caller-ordered.
     * @param {DeriveSeedOptions} [options]
     * @returns {Promise<{ seeds: Uint8Array[], credentialId: Uint8Array | null }>}
     *   One output per salt plus the credential ID observed in the same
     *   assertion (null when none was seen).
     */
    async deriveSeeds(salts, options = {}) {
        if (!Array.isArray(salts) || salts.length === 0) {
            return { seeds: [], credentialId: null };
        }

        // Reset so the result reflects only this call's ceremonies.
        this._lastObservedCredentialId = null;

        const seeds = [];
        if (salts.length === 1) {
            seeds.push(await this._deriveSeed(salts[0], options));
            return { seeds, credentialId: this._lastObservedCredentialId };
        }

        let idx = 0;
        while (idx < salts.length) {
            if (idx + 1 < salts.length) {
                const pair = await this._tryDualSaltAssertion(salts[idx], salts[idx + 1], options);
                seeds.push(pair[0]);
                if (pair[1] != null) {
                    seeds.push(pair[1]);
                    idx += 2;
                    continue;
                }
                seeds.push(await this._deriveSeed(salts[idx + 1], options));
                idx += 2;
            } else {
                seeds.push(await this._deriveSeed(salts[idx], options));
                idx += 1;
            }
        }
        return { seeds, credentialId: this._lastObservedCredentialId };
    }

    /**
     * Register a new PRF-capable passkey (one ceremony, no seed
     * derivation). Browsers allow multiple credentials per RP, so pass
     * `excludeCredentials` (already-registered IDs) to surface a repeat
     * registration as `PasskeyAlreadyExistsError`.
     *
     * @param {Uint8Array[]} [excludeCredentials]
     * @returns {Promise<RegisteredCredential>} `aaguid`/`backupEligible`
     *   are null on browsers without `getAuthenticatorData()`.
     */
    async createPasskey(excludeCredentials = []) {
        return await this._registerCredential(excludeCredentials);
    }

    /**
     * Check if a PRF-capable passkey is available on this device.
     *
     * @returns {Promise<boolean>} true if WebAuthn with PRF extension is likely supported.
     */
    async isSupported() {
        try {
            if (typeof window === 'undefined' || !window.PublicKeyCredential) {
                return false;
            }
            if (typeof PublicKeyCredential.isUserVerifyingPlatformAuthenticatorAvailable !== 'function') {
                return false;
            }
            return await PublicKeyCredential.isUserVerifyingPlatformAuthenticatorAvailable();
        } catch {
            return false;
        }
    }

    /**
     * Verify the configured rpId is a valid scope for WebAuthn from the
     * current document's origin.
     *
     * # Web vs. iOS/Android
     *
     * Browsers verify the `rp.id` constraint locally at
     * `navigator.credentials.get / create` time: `rpId` must be a
     * registrable suffix of `window.location.hostname`, OR equal to it.
     * There is no equivalent of Apple's AASA CDN or Google's Digital Asset
     * Links API: no external file, no TTL, no caching. The browser's own
     * check is synchronous and deterministic.
     *
     * This method mirrors that browser-side rule so a misconfigured `rpId`
     * (e.g. running a staging build pointed at `keys.breez.technology`
     * while hosted at `staging.example.com`) can be diagnosed before any
     * WebAuthn ceremony runs: producing a `NotAssociated` result with a
     * concrete reason instead of the opaque `SecurityError` the browser
     * would throw.
     *
     * # Return semantics
     *
     * - `rpId` matches the registrable-suffix rule → `Associated`
     * - `rpId` violates the rule → `NotAssociated` with a concrete reason
     * - No `window` / `location.hostname` available (SSR, test runner,
     *   Deno) → `Skipped`: the browser will enforce its own rule at
     *   WebAuthn call time anyway, so this is never a false-negative.
     *
     * @returns {Promise<{kind: 'Associated'} |
     *                   {kind: 'NotAssociated', source: string, reason: string} |
     *                   {kind: 'Skipped', reason: string}>}
     */
    async checkDomainAssociation() {
        if (typeof window === 'undefined' || !window.location || !window.location.hostname) {
            return {
                kind: 'Skipped',
                reason: 'No window.location context (SSR / test runner); browser will enforce rpId scope at WebAuthn call time',
            };
        }

        const hostname = window.location.hostname.toLowerCase();
        const rpId = (this.rpId || '').toLowerCase();

        if (!rpId) {
            return {
                kind: 'NotAssociated',
                source: 'WebAuthn rpId scope check',
                reason: 'Provider was constructed with empty rpId; WebAuthn ceremonies will fail',
            };
        }

        // Exact match covers the common case (rpId = hostname).
        if (rpId === hostname) {
            return { kind: 'Associated' };
        }

        // Registrable-suffix rule: rpId must be an ancestor domain of
        // hostname (e.g. rpId="example.com" is valid at
        // hostname="app.example.com"). Dot-aligned suffix match is the
        // spec-level shortcut; the full eTLD+1 check against the Public
        // Suffix List would catch pathological cases like
        // rpId="co.uk" but is an order-of-magnitude heavier dependency.
        // For Breez's deployment profile this is sufficient.
        if (hostname.endsWith('.' + rpId)) {
            return { kind: 'Associated' };
        }

        return {
            kind: 'NotAssociated',
            source: 'WebAuthn rpId scope check',
            reason: `rpId "${rpId}" is not a registrable suffix of window.location.hostname "${hostname}". ` +
                `WebAuthn ceremonies from this origin will fail with SecurityError.`,
        };
    }

    /**
     * @param {Uint8Array} saltBytes
     * @param {DeriveSeedOptions} options
     * @returns {Promise<Uint8Array>}
     * @private
     */
    async _getAssertionWithPrf(saltBytes, options) {
        const credential = await this._performAssertion(
            { first: saltBytes },
            options,
        );
        const ext = credential.getClientExtensionResults();
        if (!ext.prf || !ext.prf.results || !ext.prf.results.first) {
            throw new Error('PRF not supported by authenticator');
        }
        return new Uint8Array(ext.prf.results.first);
    }

    /**
     * Dual-salt assertion. Returns `[first, second|null]`; `second` is
     * null when the authenticator drops `prf.eval.second` (caller
     * single-salts the dropped one).
     *
     * @param {string} salt1
     * @param {string} salt2
     * @param {DeriveSeedOptions} options
     * @returns {Promise<[Uint8Array, Uint8Array|null]>}
     * @private
     */
    async _tryDualSaltAssertion(salt1, salt2, options) {
        const enc = new TextEncoder();
        const salt1Bytes = enc.encode(salt1);
        const salt2Bytes = enc.encode(salt2);
        return await this._getDualSaltAssertionWithPrf(salt1Bytes, salt2Bytes, options);
    }

    /**
     * @param {Uint8Array} salt1Bytes
     * @param {Uint8Array} salt2Bytes
     * @param {DeriveSeedOptions} options
     * @returns {Promise<[Uint8Array, Uint8Array|null]>}
     * @private
     */
    async _getDualSaltAssertionWithPrf(salt1Bytes, salt2Bytes, options) {
        const credential = await this._performAssertion(
            { first: salt1Bytes, second: salt2Bytes },
            options,
        );
        const ext = credential.getClientExtensionResults();
        if (!ext.prf || !ext.prf.results || !ext.prf.results.first) {
            throw new Error('PRF not supported by authenticator');
        }
        const first = new Uint8Array(ext.prf.results.first);
        const second = ext.prf.results.second
            ? new Uint8Array(ext.prf.results.second)
            : null;
        return [first, second];
    }

    /**
     * Build assertion options, run the ceremony, and fire the
     * onCredentialId callback. Shared by single- and dual-salt paths.
     *
     * @param {{ first: Uint8Array, second?: Uint8Array }} prfEval
     * @param {DeriveSeedOptions} options
     * @returns {Promise<PublicKeyCredential>}
     * @private
     */
    async _performAssertion(prfEval, options) {
        let allowList = options.allowCredentials || [];
        // Auto-merge registry IDs into the allow-list.
        if (this.credentialRegistry) {
            const registryIds = await _registryReadBestEffort(
                this.credentialRegistry, this.rpId, this.onRegistryError,
            );
            if (registryIds.length > 0) {
                const seen = new Set(allowList.map((id) => _bytesToBase64Url(id)));
                const merged = [...allowList];
                for (const id of registryIds) {
                    const key = _bytesToBase64Url(id);
                    if (!seen.has(key)) {
                        seen.add(key);
                        merged.push(id);
                    }
                }
                allowList = merged;
            }
        }
        const allowCredentials = allowList.map((id) => ({
            type: 'public-key',
            id,
        }));

        const publicKey = {
            challenge: randomBytes(32),
            rpId: this.rpId,
            allowCredentials,
            userVerification: 'required',
            extensions: { prf: { eval: prfEval } },
        };
        if (Array.isArray(this.hints) && this.hints.length > 0) {
            publicKey.hints = [...this.hints];
        }
        if (typeof this.defaultTimeoutMs === 'number' && this.defaultTimeoutMs > 0) {
            publicKey.timeout = this.defaultTimeoutMs;
        }

        const requestOptions = { publicKey };

        let credential;
        const startedAt = (typeof performance !== 'undefined' && performance.now)
            ? performance.now()
            : Date.now();
        try {
            credential = await navigator.credentials.get(requestOptions);
        } catch (error) {
            const elapsed = ((typeof performance !== 'undefined' && performance.now)
                ? performance.now()
                : Date.now()) - startedAt;
            // Append registry help suffix when host had nothing for us
            // to populate the allow-list with: no per-call IDs, no
            // registry. Tells integrators about the opt-in path.
            const augmentNoCredHelp =
                allowCredentials.length === 0 && !this.credentialRegistry;
            throw this._mapAssertionError(error, elapsed, augmentNoCredHelp);
        }
        if (!credential) {
            throw new PasskeyCredentialNotFoundError();
        }

        const credentialIdBytes = new Uint8Array(credential.rawId);
        this._lastObservedCredentialId = credentialIdBytes;
        if (this.credentialRegistry) {
            _registryAddFireAndForget(
                this.credentialRegistry, this.rpId, credentialIdBytes, this.onRegistryError,
            );
        }
        return credential;
    }

    /**
     * Return the credential IDs the configured `CredentialRegistry`
     * has stored for the current `rpId`. Empty list when no registry
     * is configured. Backs `PasskeyClient.credentials().get()`.
     * @returns {Promise<Uint8Array[]>}
     */
    async getKnownCredentialIds() {
        if (!this.credentialRegistry) {
            return [];
        }
        const stored = await _registryReadBestEffort(
            this.credentialRegistry, this.rpId, this.onRegistryError,
        );
        return Array.isArray(stored) ? stored : [];
    }

    /**
     * Drop a single credential ID from the configured registry. No-op
     * when no registry is configured. Backs
     * `PasskeyClient.credentials().remove(id)`.
     * @param {Uint8Array} credentialId
     * @returns {Promise<void>}
     */
    async removeKnownCredentialId(credentialId) {
        if (!this.credentialRegistry) {
            return;
        }
        try {
            const result = await _withRegistryTimeout(
                this.credentialRegistry.remove(this.rpId, credentialId),
            );
            if (result === _REGISTRY_TIMEOUT) {
                const err = new Error('CredentialRegistry.remove timed out');
                console.warn('[CredentialRegistry] remove timed out');
                this.onRegistryError?.('remove', err);
            }
        } catch (err) {
            console.warn('[CredentialRegistry] remove failed', err);
            this.onRegistryError?.('remove', err);
        }
    }

    /**
     * Clear the configured registry's persisted credential-ID list for
     * the current `rpId`. No-op when no registry is configured. Backs
     * `PasskeyClient.credentials().clear()`.
     * @returns {Promise<void>}
     */
    async clearKnownCredentialIds() {
        if (!this.credentialRegistry) {
            return;
        }
        try {
            const result = await _withRegistryTimeout(
                this.credentialRegistry.clear(this.rpId),
            );
            if (result === _REGISTRY_TIMEOUT) {
                const err = new Error('CredentialRegistry.clear timed out');
                console.warn('[CredentialRegistry] clear timed out');
                this.onRegistryError?.('clear', err);
            }
        } catch (err) {
            console.warn('[CredentialRegistry] clear failed', err);
            this.onRegistryError?.('clear', err);
        }
    }

    /**
     * Register a new discoverable credential with PRF extension enabled.
     * @param {Uint8Array[]} [excludeCredentials=[]] - A list of
     *   already-registered credential IDs. The authenticator refuses
     *   to register if any entry matches a credential already on the
     *   device, preventing duplicate registrations on the same device.
     * @returns {Promise<{ credentialId: Uint8Array, userId: Uint8Array, aaguid: Uint8Array | null, backupEligible: boolean | null }>}
     * @private
     */
    async _registerCredential(excludeCredentials = []) {
        // WebAuthn spec: user.id must be 1-64 bytes. The provider
        // always mints a fresh 16 random bytes per call and returns
        // them via `RegisteredCredential.userId` (never host-supplied).
        // Reusing a userId across creates on the same rpId silently
        // overwrites the prior credential on some authenticators.
        const resolvedUserId = randomBytes(16);

        const authenticatorSelection = {
            residentKey: 'required',
            requireResidentKey: true,
            userVerification: 'required',
        };
        if (this.authenticatorAttachment) {
            authenticatorSelection.authenticatorAttachment = this.authenticatorAttachment;
        }

        const publicKeyOptions = {
            challenge: randomBytes(32),
            rp: {
                id: this.rpId,
                name: this.rpName,
            },
            user: {
                id: resolvedUserId,
                name: this.userName,
                displayName: this.userDisplayName,
            },
            pubKeyCredParams: [
                { type: 'public-key', alg: -7 },   // ES256 (P-256)
                { type: 'public-key', alg: -257 },  // RS256
            ],
            authenticatorSelection,
            // Explicit so future security review can't read it as ambient.
            attestation: 'none',
            extensions: { prf: {} },
        };

        if (Array.isArray(this.hints) && this.hints.length > 0) {
            // Defensive copy; the host could otherwise mutate mid-ceremony.
            publicKeyOptions.hints = [...this.hints];
        }

        // Merge caller-supplied IDs with the registry, dedupe by base64url.
        const mergedExcludeIds = await this._buildExcludeCredentials(excludeCredentials);
        if (mergedExcludeIds.length > 0) {
            publicKeyOptions.excludeCredentials = mergedExcludeIds.map((id) => ({
                type: 'public-key',
                id,
            }));
        }

        if (typeof this.defaultTimeoutMs === 'number' && this.defaultTimeoutMs > 0) {
            publicKeyOptions.timeout = this.defaultTimeoutMs;
        }

        const createOptions = { publicKey: publicKeyOptions };

        let credential;
        const createStartedAt = (typeof performance !== 'undefined' && performance.now)
            ? performance.now()
            : Date.now();
        try {
            credential = await navigator.credentials.create(createOptions);
        } catch (error) {
            // Surface the duplicate-prevention check as a typed error so
            // callers can route the user to sign-in instead of treating
            // it as a generic registration failure. The browser raises
            // InvalidStateError DOMException when an entry in
            // excludeCredentials matches a credential already on the
            // device. Only meaningful in the create path.
            if (error instanceof DOMException && error.name === 'InvalidStateError') {
                throw new PasskeyAlreadyExistsError(error.message);
            }
            const elapsed = ((typeof performance !== 'undefined' && performance.now)
                ? performance.now()
                : Date.now()) - createStartedAt;
            throw this._mapError(error, elapsed);
        }

        if (!credential) {
            throw new Error('Credential creation failed');
        }

        // Verify PRF extension was acknowledged. The credential is now
        // registered with the active credential provider, but if that
        // provider lacks PRF support (e.g. Chrome Password Manager and
        // 1Password on iOS: only iCloud Keychain implements PRF), the
        // assertion side will silently fail later. Surface this here as
        // an actionable message so the user knows where to look.
        // WebAuthn doesn't expose a deletion API, so the orphan
        // credential remains in the provider's store until manually
        // removed in OS settings.
        const extensionResults = credential.getClientExtensionResults();
        if (!extensionResults.prf || !extensionResults.prf.enabled) {
            throw new Error(
                'Passkey created, but the active credential provider does not '
                + 'support the WebAuthn PRF extension. On iOS, only iCloud '
                + 'Keychain currently supports PRF: switch the default '
                + 'provider in Settings → Passwords → Password Options. '
                + 'The orphan passkey can be removed in the same settings panel.'
            );
        }

        const meta = extractRegistrationMetadata(credential);
        const credentialIdBytes = new Uint8Array(credential.rawId);
        if (this.credentialRegistry) {
            _registryAddFireAndForget(
                this.credentialRegistry, this.rpId, credentialIdBytes, this.onRegistryError,
            );
        }
        return {
            credentialId: credentialIdBytes,
            userId: resolvedUserId,
            aaguid: meta ? meta.aaguid : null,
            backupEligible: meta ? meta.backupEligible : null,
        };
    }

    /**
     * Merge caller-supplied excludeCredentials with whatever the
     * registry has stored for `this.rpId`. Dedupes by base64url
     * encoding so the same credential isn't sent twice.
     * @private
     */
    async _buildExcludeCredentials(callerIds) {
        if (!this.credentialRegistry) {
            return Array.isArray(callerIds) ? [...callerIds] : [];
        }
        const stored = await _registryReadBestEffort(
            this.credentialRegistry, this.rpId, this.onRegistryError,
        );
        const seen = new Set();
        const out = [];
        const push = (id) => {
            const key = _bytesToBase64Url(id);
            if (seen.has(key)) return;
            seen.add(key);
            out.push(id);
        };
        if (Array.isArray(callerIds)) for (const id of callerIds) push(id);
        for (const id of stored) push(id);
        return out;
    }

    /**
     * Map a `navigator.credentials.get` failure into a typed message.
     * `elapsedMs` lets us discriminate the WebAuthn `NotAllowedError`
     * ambiguity: cancel vs no-credential collapse to the same error
     * by spec, but only the cancel path shows UI to dismiss, so the
     * call's wall-clock time tells them apart.
     * @param {Error} error
     * @param {number} elapsedMs
     * @returns {Error}
     * @private
     */
    _mapAssertionError(error, elapsedMs, augmentNoCredHelp = false) {
        if (!error) return new Error('Unknown WebAuthn error');
        if (error.name === 'NotAllowedError') {
            if (elapsedMs < NO_CRED_FAST_FAIL_MS) {
                const msg = 'Credential not found'
                    + (augmentNoCredHelp ? CREDENTIAL_REGISTRY_HELP_SUFFIX : '');
                return new PasskeyCredentialNotFoundError(msg);
            }
            if (elapsedMs >= BIOMETRIC_TIMEOUT_MS) {
                return new PasskeyTimedOutError();
            }
            return new PasskeyUserCancelledError();
        }
        return this._mapError(error);
    }

    /**
     * Map non-assertion WebAuthn errors (registration path).
     * @param {Error} error
     * @param {number} [elapsedMs] Wall-clock duration of the failed
     *   ceremony, when available. Used to reclassify a long-running
     *   NotAllowedError as the OS biometric inactivity timeout
     *   (`PasskeyTimedOutError`) instead of a user-cancel; without it
     *   the historical substring heuristic applies.
     * @returns {Error}
     * @private
     */
    _mapError(error, elapsedMs) {
        if (!error) return new Error('Unknown WebAuthn error');
        switch (error.name) {
            case 'NotAllowedError':
                if (typeof elapsedMs === 'number' && elapsedMs >= BIOMETRIC_TIMEOUT_MS) {
                    return new PasskeyTimedOutError();
                }
                // Registration NotAllowedError isn't usefully timed
                // (no fast-fail equivalent), so keep the substring
                // heuristic and fall back to the raw error.
                if (error.message && (
                    error.message.includes('cancelled') ||
                    error.message.includes('canceled') ||
                    error.message.includes('denied') ||
                    error.message.includes('The operation either timed out or was not allowed')
                )) {
                    return new PasskeyUserCancelledError();
                }
                return error;
            case 'SecurityError':
            case 'InvalidStateError':
                return new Error(`Authentication failed: ${error.message}`);
            case 'AbortError':
                return new PasskeyUserCancelledError();
            default:
                return error;
        }
    }
}

/**
 * Builder for a {@link PasskeyClient} with a caller-supplied
 * `PrfProvider`. Use it when you need a configured {@link PasskeyProvider}
 * (custom `rpId`/`rpName`, a `credentialRegistry`, timeout overrides) or
 * a custom PRF backend. For the zero-config Breez-RP case, use the
 * {@link PasskeyClient} constructor directly.
 *
 * @example
 * ```javascript
 * const provider = new PasskeyProvider({ rpId, rpName, credentialRegistry })
 * const client = new PasskeyClientBuilder(breezApiKey)
 *     .withPrfProvider(provider)
 *     .build()
 * ```
 */
export class PasskeyClientBuilder {
    /**
     * @param {string} [breezApiKey] - Breez relay key for authenticated
     *   (NIP-42) label storage. Omit for public relays only.
     * @param {import('../breez_sdk_spark_wasm.js').PasskeyConfig} [config] -
     *   `rpId` / `rpName` configure the built-in provider (ignored when
     *   one is injected via {@link withPrfProvider}); `defaultLabel` is
     *   the label-store default.
     */
    constructor(breezApiKey, config = {}) {
        this._breezApiKey = breezApiKey;
        this._config = config ?? {};
        this._provider = null;
    }

    /**
     * Inject the `PrfProvider` the client derives seeds through,
     * superseding the config's `rpId` / `rpName`.
     * @param {PrfProvider} provider
     * @returns {PasskeyClientBuilder} this, for chaining.
     */
    withPrfProvider(provider) {
        this._provider = provider;
        return this;
    }

    /**
     * Build the client, defaulting to a browser {@link PasskeyProvider}
     * on the config's `rpId` / `rpName` (default: the Breez RP) when no
     * provider was injected.
     * @returns {import('../breez_sdk_spark_wasm.js').PasskeyClient}
     */
    build() {
        const provider =
            this._provider ??
            new PasskeyProvider({
                rpId: this._config.rpId ?? BREEZ_RP_ID,
                rpName: this._config.rpName ?? DEFAULT_RP_NAME,
            });
        return new SdkPasskeyClient(provider, this._breezApiKey, this._config);
    }
}

/**
 * High-level passkey client. The zero-config constructor wires the
 * built-in browser {@link PasskeyProvider} on the Breez shared RP, so a
 * Breez-registered app needs only its relay key.
 *
 * ```javascript
 * const client = new PasskeyClient(breezApiKey)
 * const { wallet } = await client.signIn({ label: 'personal' })
 * ```
 *
 * Apps with their own RP, a credential registry, or a custom PRF backend
 * inject their own provider through {@link PasskeyClientBuilder}. The
 * instance is the underlying SDK client (`checkAvailability`, `register`,
 * `signIn`, `labels()`, `credentials()`).
 */
export class PasskeyClient {
    /**
     * @param {string} [breezApiKey] - Breez relay key for authenticated
     *   (NIP-42) label storage. Omit for public relays only.
     * @param {import('../breez_sdk_spark_wasm.js').PasskeyConfig} [config] -
     *   Optional `rpId` / `rpName` for the built-in provider (default: the
     *   Breez shared RP) plus `defaultLabel`.
     * @returns {import('../breez_sdk_spark_wasm.js').PasskeyClient}
     */
    constructor(breezApiKey, config) {
        // Returning an object from a constructor yields it from `new`, so
        // callers get the underlying SDK client with no delegation layer.
        return new PasskeyClientBuilder(breezApiKey, config).build();
    }
}
