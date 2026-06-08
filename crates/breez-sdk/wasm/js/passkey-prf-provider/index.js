/**
 * Built-in browser PRF provider: implements `PrfProvider` over the
 * WebAuthn PRF extension, using discoverable credentials so no
 * credential storage is needed.
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
 * Extract AAGUID + BE flag from a create response via WebAuthn L2
 * `getAuthenticatorData()`. authData layout once the AT flag is set:
 * byte 32 = flags (BE=bit3, AT=bit6), bytes 37 to 53 = AAGUID.
 * Returns null when the accessor or attested credential data is absent.
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
 * Thrown by `createPasskey` when an entry in `excludeCredentials`
 * matches a credential already on the device. Route the user to
 * sign-in rather than treating it as a generic registration failure.
 */
export class PasskeyAlreadyExistsError extends Error {
    constructor(message = 'A passkey for this RP already exists on this device') {
        super(message);
        this.name = 'PasskeyAlreadyExistsError';
    }
}

/**
 * Thrown when the OS biometric prompt times out (around 55s) before
 * any user interaction. Distinct from a cancel: hosts may auto-retry
 * since the user did not deliberately abandon the flow.
 */
export class PasskeyTimedOutError extends Error {
    constructor(message = 'Authenticator timed out') {
        super(message);
        this.name = 'PasskeyTimedOutError';
    }
}

/**
 * Thrown when `deriveSeeds` cannot match a credential for this RP on
 * the device. `message` carries diagnostic detail.
 */
export class PasskeyCredentialNotFoundError extends Error {
    constructor(message = 'Credential not found') {
        super(message);
        this.name = 'PasskeyCredentialNotFoundError';
    }
}

/**
 * Thrown when the user actively dismisses the OS passkey prompt.
 * Distinct from `PasskeyTimedOutError`: hosts should not auto-retry a
 * deliberate cancel.
 */
export class PasskeyUserCancelledError extends Error {
    constructor(message = 'User cancelled authentication') {
        super(message);
        this.name = 'PasskeyUserCancelledError';
    }
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
     * @param {import('../breez_sdk_spark_wasm.js').PasskeyProviderOptions} [options]
     *   Relying Party and user identity. `rpId` defaults to
     *   `PasskeyProvider.BREEZ_RP_ID`, `rpName` to `"Breez"`, `userName` to
     *   `rpName`, `userDisplayName` to `userName`. The same
     *   `PasskeyProviderOptions` is settable on `PasskeyConfig` for the
     *   zero-config client.
     * @param {object} [webOptions] - Web-only knobs.
     * @param {'platform'|'cross-platform'} [webOptions.authenticatorAttachment]
     *   Narrows the create-time chooser to one authenticator class.
     *   `'platform'` allows only the local authenticator (Touch ID, Face
     *   ID, Windows Hello, iCloud Keychain); `'cross-platform'` only
     *   roaming keys (USB, NFC, BLE, hybrid). Unset shows all.
     * @param {Array<'client-device'|'security-key'|'hybrid'>} [webOptions.hints]
     *   WebAuthn L3 priority hints applied to both create and get,
     *   ordering the classes a supporting browser offers first (ignored
     *   otherwise). Pass `['client-device']` to favor the platform
     *   authenticator. Only standards-track lever for the sign-in picker,
     *   where `authenticatorAttachment` is not allowed.
     * @param {number} [webOptions.defaultTimeoutMs] - Default WebAuthn
     *   `timeout` (ms) for create and get. A hint only: platforms cap
     *   around 60s. Set it 5 to 10s under the cap so a host-side timeout
     *   heuristic can fire first.
     */
    constructor(options = {}, webOptions = {}) {
        const rpId = options.rpId ?? BREEZ_RP_ID;
        const rpName = options.rpName ?? DEFAULT_RP_NAME;
        if (typeof rpId !== 'string' || rpId.length === 0) {
            throw new Error('PasskeyProvider: rpId must be a non-empty string.');
        }
        if (typeof rpName !== 'string' || rpName.length === 0) {
            throw new Error('PasskeyProvider: rpName must be a non-empty string.');
        }
        this.rpId = rpId;
        this.rpName = rpName;
        this.userName = options.userName || this.rpName;
        this.userDisplayName = options.userDisplayName || this.userName;
        this.authenticatorAttachment = webOptions.authenticatorAttachment;
        this.hints = webOptions.hints;
        this.defaultTimeoutMs = webOptions.defaultTimeoutMs;

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

        // After the first assertion, pin the rest of this call to the
        // credential it resolved to, so every salt derives from one passkey
        // even when a chunk splits (dropped `prf.eval.second`, or 3+ salts).
        let activeOptions = options;
        const pinToObserved = () => {
            if (this._lastObservedCredentialId != null) {
                activeOptions = { ...options, _pinnedCredentialId: this._lastObservedCredentialId };
            }
        };

        let idx = 0;
        while (idx < salts.length) {
            if (idx + 1 < salts.length) {
                const pair = await this._tryDualSaltAssertion(salts[idx], salts[idx + 1], activeOptions);
                pinToObserved();
                seeds.push(pair[0]);
                if (pair[1] != null) {
                    seeds.push(pair[1]);
                    idx += 2;
                    continue;
                }
                seeds.push(await this._deriveSeed(salts[idx + 1], activeOptions));
                idx += 2;
            } else {
                seeds.push(await this._deriveSeed(salts[idx], activeOptions));
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
     * @returns {Promise<PasskeyCredential>} `aaguid`/`backupEligible`
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
     * Check whether the configured rpId is a valid WebAuthn scope for
     * the current origin (must be a registrable suffix of
     * `window.location.hostname`, or equal to it). Mirrors the browser's
     * own rule so a misconfigured rpId is diagnosed with a concrete
     * reason instead of an opaque `SecurityError` at ceremony time.
     * `Skipped` (no `window.location`) is never a false-negative: the
     * browser enforces the rule itself at call time.
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

        if (rpId === hostname) {
            return { kind: 'Associated' };
        }

        // Registrable-suffix rule: rpId must be an ancestor domain of
        // hostname (rpId="example.com" is valid at "app.example.com").
        // Dot-aligned suffix is the spec shortcut; skipping the full
        // Public Suffix List check (would reject rpId="co.uk") is fine
        // for Breez's deployment profile and avoids a heavy dependency.
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
     * null when the authenticator drops `prf.eval.second`.
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
     * Build assertion options and run the ceremony. Shared by single-
     * and dual-salt paths.
     *
     * @param {{ first: Uint8Array, second?: Uint8Array }} prfEval
     * @param {DeriveSeedOptions} options
     * @returns {Promise<PublicKeyCredential>}
     * @private
     */
    async _performAssertion(prfEval, options) {
        let allowList;
        if (options._pinnedCredentialId != null) {
            // A later assertion in a multi-ceremony derive: pin to the
            // credential the first one resolved to, so the pin can't be
            // re-widened to another credential.
            allowList = [options._pinnedCredentialId];
        } else {
            allowList = options.allowCredentials || [];
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
            throw this._mapAssertionError(error, elapsed);
        }
        if (!credential) {
            throw new PasskeyCredentialNotFoundError();
        }

        this._lastObservedCredentialId = new Uint8Array(credential.rawId);
        return credential;
    }

    /**
     * Register a new discoverable credential with PRF extension enabled.
     * @param {Uint8Array[]} [excludeCredentials=[]] - Already-registered
     *   IDs; a match makes the authenticator refuse, preventing a
     *   duplicate registration on the same device.
     * @returns {Promise<{ credentialId: Uint8Array, userId: Uint8Array, aaguid: Uint8Array | null, backupEligible: boolean | null }>}
     * @private
     */
    async _registerCredential(excludeCredentials = []) {
        // Fresh per-call user.id: reusing one across creates on the same
        // rpId silently overwrites the prior credential on some
        // authenticators. (WebAuthn requires 1 to 64 bytes.)
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

        if (Array.isArray(excludeCredentials) && excludeCredentials.length > 0) {
            publicKeyOptions.excludeCredentials = excludeCredentials.map((id) => ({
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
            // The browser raises InvalidStateError when an
            // excludeCredentials entry already exists on the device.
            // Surface it as a typed error so callers can route to sign-in.
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

        // Fail loudly if the provider didn't acknowledge PRF: without it
        // the assertion side fails silently later, and WebAuthn has no
        // deletion API so the orphan credential lingers until removed in
        // OS settings.
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
        return {
            credentialId: new Uint8Array(credential.rawId),
            userId: resolvedUserId,
            aaguid: meta ? meta.aaguid : null,
            backupEligible: meta ? meta.backupEligible : null,
        };
    }

    /**
     * Threshold (ms) at which an elapsed `NotAllowedError` is treated as
     * a timeout rather than a user cancel. The OS biometric inactivity
     * window is the ceiling; a host-configured `defaultTimeoutMs` shorter
     * than that wins so a real timeout near the host limit isn't
     * misreported as a cancel.
     * @returns {number}
     * @private
     */
    _cancelVsTimeoutThresholdMs() {
        return (typeof this.defaultTimeoutMs === 'number'
            && this.defaultTimeoutMs > 0
            && this.defaultTimeoutMs < BIOMETRIC_TIMEOUT_MS)
            ? this.defaultTimeoutMs
            : BIOMETRIC_TIMEOUT_MS;
    }

    /**
     * Map a `navigator.credentials.get` failure into a typed error.
     * `elapsedMs` resolves the `NotAllowedError` ambiguity (cancel vs
     * no-credential vs timeout, which all share the error) by elapsed
     * time, since only the cancel path shows dismissable UI.
     * @param {Error} error
     * @param {number} elapsedMs
     * @returns {Error}
     * @private
     */
    _mapAssertionError(error, elapsedMs) {
        if (!error) return new Error('Unknown WebAuthn error');
        if (error.name === 'NotAllowedError') {
            if (elapsedMs < NO_CRED_FAST_FAIL_MS) {
                return new PasskeyCredentialNotFoundError();
            }
            if (elapsedMs >= this._cancelVsTimeoutThresholdMs()) {
                return new PasskeyTimedOutError();
            }
            return new PasskeyUserCancelledError();
        }
        return this._mapError(error);
    }

    /**
     * Map non-assertion WebAuthn errors (registration path).
     * @param {Error} error
     * @param {number} [elapsedMs] When given, reclassifies a long-running
     *   NotAllowedError as a timeout instead of a user-cancel; otherwise
     *   a substring heuristic applies.
     * @returns {Error}
     * @private
     */
    _mapError(error, elapsedMs) {
        if (!error) return new Error('Unknown WebAuthn error');
        switch (error.name) {
            case 'NotAllowedError':
                if (typeof elapsedMs === 'number'
                    && elapsedMs >= this._cancelVsTimeoutThresholdMs()) {
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
 * (web-specific options like `authenticatorAttachment` or timeout overrides)
 * or a custom PRF backend.
 * For the zero-config Breez-RP case, use the {@link PasskeyClient}
 * constructor directly.
 *
 * @example
 * ```javascript
 * const provider = new PasskeyProvider({ rpId, rpName })
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
     *   `providerOptions` configures the built-in provider (ignored when
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
     * superseding the config's `providerOptions`.
     * @param {PrfProvider} provider
     * @returns {PasskeyClientBuilder} this, for chaining.
     */
    withPrfProvider(provider) {
        this._provider = provider;
        return this;
    }

    /**
     * Build the client, defaulting to a browser {@link PasskeyProvider}
     * on the config's `providerOptions` (default: the Breez RP) when no
     * provider was injected.
     * @returns {import('../breez_sdk_spark_wasm.js').PasskeyClient}
     */
    build() {
        const provider =
            this._provider ??
            new PasskeyProvider(this._config.providerOptions ?? {});
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
 * Apps with their own RP or a custom PRF backend inject their own
 * provider through {@link PasskeyClientBuilder}. The instance is the
 * underlying SDK client (`checkAvailability`, `register`, `signIn`,
 * `labels()`).
 */
export class PasskeyClient {
    /**
     * @param {string} [breezApiKey] - Breez relay key for authenticated
     *   (NIP-42) label storage. Omit for public relays only.
     * @param {import('../breez_sdk_spark_wasm.js').PasskeyConfig} [config] -
     *   Optional `providerOptions` for the built-in provider (default: the
     *   Breez shared RP) plus `defaultLabel`.
     * @returns {import('../breez_sdk_spark_wasm.js').PasskeyClient}
     */
    constructor(breezApiKey, config) {
        // Returning an object from a constructor yields it from `new`, so
        // callers get the underlying SDK client with no delegation layer.
        return new PasskeyClientBuilder(breezApiKey, config).build();
    }
}
