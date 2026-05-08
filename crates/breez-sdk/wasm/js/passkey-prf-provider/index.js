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

const DEFAULT_RP_ID = 'keys.breez.technology';
const DEFAULT_RP_NAME = 'Breez SDK';

/**
 * Wall-clock threshold (ms) used to discriminate a NotAllowedError
 * raised by `navigator.credentials.get`. WebAuthn deliberately
 * collapses "no matching credential" and "user dismissed" into the
 * same error for privacy reasons, but a no-credential fast-fail
 * resolves before any UI is shown (typically 50-150ms), while
 * dismissing a visible prompt takes seconds. Anything below this
 * threshold is classified as no-credential; anything at or above
 * is classified as user-cancel.
 */
const NO_CRED_FAST_FAIL_MS = 250;

/**
 * Wall-clock threshold (ms) above which a NotAllowedError is
 * reclassified as the OS biometric inactivity timeout instead of a
 * user-dismissed prompt. iOS and Android both tear the biometric
 * sheet down around 55s of inactivity and surface the same generic
 * NotAllowedError; the duration of the call is the only in-process
 * signal that distinguishes "the user walked away" from "the user
 * tapped Cancel".
 */
const BIOMETRIC_TIMEOUT_MS = 55_000;

/**
 * Module-level caches. Survive `PasskeyProvider` reconstruction
 * (e.g. host signs out and signs back in within the same tab), so
 * the capability probes only run once per page load.
 *   _immediateGetCache: undefined = not probed, null = unsupported, true/false = result
 *   _chromeMajorCache:  undefined = not parsed, NaN = non-Chrome, number = major
 */
let _immediateGetCache;
let _chromeMajorCache;

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
 * passkey but it refuses because an entry in `excludeCredentialIds`
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

/**
 * Default `CredentialRegistry` backed by `window.localStorage`. One
 * JSON-encoded array of base64url credential IDs per RP, keyed
 * `<keyPrefix><rpId>`. Cleared by the browser on site-data clear;
 * not synced across devices.
 */
export class LocalStorageCredentialRegistry {
    constructor(keyPrefix = 'breez.spark.passkey.knownCredentials.') {
        this._prefix = keyPrefix;
    }

    _key(rpId) {
        return this._prefix + rpId;
    }

    _read(rpId) {
        try {
            const raw = globalThis.localStorage?.getItem(this._key(rpId));
            if (!raw) return [];
            const parsed = JSON.parse(raw);
            return Array.isArray(parsed) ? parsed.filter((s) => typeof s === 'string') : [];
        } catch {
            return [];
        }
    }

    _write(rpId, ids) {
        try {
            if (ids.length === 0) {
                globalThis.localStorage?.removeItem(this._key(rpId));
            } else {
                globalThis.localStorage?.setItem(this._key(rpId), JSON.stringify(ids));
            }
        } catch {
            // best-effort: localStorage quota / disabled
        }
    }

    async list(rpId) {
        return this._read(rpId).map(_base64UrlToBytes);
    }

    async add(rpId, credentialId) {
        const encoded = _bytesToBase64Url(credentialId);
        const ids = this._read(rpId);
        if (ids.includes(encoded)) return;
        ids.push(encoded);
        this._write(rpId, ids);
    }

    async remove(rpId, credentialId) {
        const encoded = _bytesToBase64Url(credentialId);
        const ids = this._read(rpId).filter((s) => s !== encoded);
        this._write(rpId, ids);
    }

    async clear(rpId) {
        this._write(rpId, []);
    }
}

/**
 * Built-in passkey-based PRF provider for browser environments.
 */
export class PasskeyProvider {
    /**
     * @param {object} [options]
     * @param {string} [options.rpId='keys.breez.technology'] - Relying Party ID.
     *   Must match the domain hosting your passkeys. On native platforms this
     *   corresponds to the AASA / assetlinks.json domain. Changing this after users have registered passkeys will
     *   make their existing credentials undiscoverable — they would need to create
     *   new passkeys with the new RP ID.
     * @param {string} [options.rpName='Breez SDK'] - RP display name shown during
     *   credential registration. Only used when creating new passkeys; changing it
     *   does not affect existing credentials.
     * @param {string} [options.userName] - User name stored with the credential,
     *   shown as a secondary label in some passkey managers. Defaults to rpName.
     *   Only used during registration; changing it does not affect existing credentials.
     * @param {string} [options.userDisplayName] - User display name shown as the
     *   primary label in the passkey picker. Defaults to userName. Only used during
     *   registration; changing it does not affect existing credentials.
     * @param {boolean} [options.autoRegister=false] - When `true`,
     *   `deriveSeed` automatically creates a new passkey if none exists
     *   for this RP ID, then retries the assertion. When `false`
     *   (default), throws on missing credential — hosts drive
     *   registration explicitly via `PasskeyClient.register` /
     *   `createPasskey()`.
     * @param {'platform'|'cross-platform'} [options.authenticatorAttachment]
     *   When set, narrows the create-time UI to the chosen authenticator
     *   class. `'platform'` scopes registration to the local platform
     *   authenticator (Touch ID / Face ID / Windows Hello / iCloud
     *   Keychain), suppressing security-key and hybrid (cross-device)
     *   options in the browser's chooser. Useful for hosts that do not
     *   want users registering security keys or QR-paired devices.
     *   When omitted, the browser shows all available authenticators.
     * @param {Array<'client-device'|'security-key'|'hybrid'>} [options.hints]
     *   WebAuthn L3 priority hints, applied to both create() and get()
     *   public-key options. Soft signal compared to
     *   `authenticatorAttachment` (which is create-only): browsers
     *   that honor it surface the listed authenticator classes first;
     *   browsers that ignore it fall back to default ordering. Stacks
     *   with `authenticatorAttachment` on create. Pass
     *   `['client-device']` to nudge platform authenticator before
     *   security-key / hybrid options. The get() side is the only
     *   standards-track lever for influencing the sign-in picker
     *   (since `authenticatorAttachment` is not allowed there).
     */
    constructor(options = {}) {
        this.rpId = options.rpId || DEFAULT_RP_ID;
        this.rpName = options.rpName || DEFAULT_RP_NAME;
        this.userName = options.userName || this.rpName;
        this.userDisplayName = options.userDisplayName || this.userName;
        this.autoRegister = options.autoRegister === true;
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
         * `excludeCredentials` on create and writes successful
         * create / assert IDs back. Omit to disable
         * auto-population entirely.
         * @type {CredentialRegistry | undefined}
         */
        this.credentialRegistry = options.credentialRegistry;

        /** @private shared Promise so concurrent auto-register paths fire one ceremony */
        this._autoRegisterInFlight = null;
    }

    /**
     * Whether `mediation`/`uiMode: 'immediate'` is supported in this
     * tab. Result cached at module scope (see top of file) so
     * `PasskeyProvider` reconstruction doesn't re-probe.
     * @returns {Promise<boolean>}
     * @private
     */
    async _supportsImmediateGet() {
        if (_immediateGetCache !== undefined) return _immediateGetCache === true;
        try {
            if (typeof PublicKeyCredential === 'undefined'
                || typeof PublicKeyCredential.getClientCapabilities !== 'function') {
                _immediateGetCache = null;
                return false;
            }
            const caps = await PublicKeyCredential.getClientCapabilities('public-key');
            _immediateGetCache = caps?.immediateGet === true;
        } catch {
            _immediateGetCache = null;
        }
        return _immediateGetCache === true;
    }

    /**
     * Set the immediate-mediation option on the get() request, picking
     * the right field name across the spec transition. Chrome ≤ 144
     * implements the original `mediation: 'immediate'`; Chrome 145+
     * renamed it to `uiMode: 'immediate'`. Both names are set when
     * the field-rename milestone is unknown so older Chromes ignore
     * `uiMode` (unknown property) and newer Chromes ignore the
     * `mediation` value because the enum no longer accepts it — but
     * since both throw TypeError if a known property gets an unknown
     * value, we pick by Chrome major when detectable and fall back
     * to `uiMode` for non-Chrome / unknown UA.
     * @private
     */
    _applyImmediateOption(options) {
        if (_chromeMajorCache === undefined) {
            const m = (typeof navigator !== 'undefined' && /Chrome\/(\d+)/.exec(navigator.userAgent || ''));
            _chromeMajorCache = m ? parseInt(m[1], 10) : NaN;
        }
        if (Number.isFinite(_chromeMajorCache) && _chromeMajorCache <= 144) {
            options.mediation = 'immediate';
        } else {
            options.uiMode = 'immediate';
        }
    }

    /**
     * Derive a 32-byte seed from passkey PRF.
     *
     * @param {string} salt
     * @param {DeriveSeedOptions} [options]
     * @returns {Promise<Uint8Array>}
     */
    async deriveSeed(salt, options = {}) {
        const saltBytes = new TextEncoder().encode(salt);
        try {
            return await this._getAssertionWithPrf(saltBytes, options);
        } catch (error) {
            if (this.autoRegister && this._isNoCredentialError(error)) {
                await this._autoRegister();
                return await this._getAssertionWithPrf(saltBytes, options);
            }
            throw error;
        }
    }

    /**
     * Derive multiple 32-byte PRF outputs in as few user prompts as the
     * authenticator supports. Pairs salts into `prf.eval.first` +
     * `prf.eval.second` per ceremony where the platform honors it;
     * authenticators that silently drop `second` trigger a single-salt
     * fallback for the affected salt. Worst case is the same prompt
     * count as looping `deriveSeed`.
     *
     * @param {string[]} salts - Caller-ordered.
     * @param {DeriveSeedOptions} [options]
     * @returns {Promise<Uint8Array[]>} One 32-byte output per salt, in input order.
     */
    async deriveSeeds(salts, options = {}) {
        if (!Array.isArray(salts) || salts.length === 0) {
            return [];
        }
        if (salts.length === 1) {
            return [await this.deriveSeed(salts[0], options)];
        }

        const out = [];
        let idx = 0;
        while (idx < salts.length) {
            if (idx + 1 < salts.length) {
                const pair = await this._tryDualSaltAssertion(salts[idx], salts[idx + 1], options);
                out.push(pair[0]);
                if (pair[1] != null) {
                    out.push(pair[1]);
                    idx += 2;
                    continue;
                }
                out.push(await this.deriveSeed(salts[idx + 1], options));
                idx += 2;
            } else {
                out.push(await this.deriveSeed(salts[idx], options));
                idx += 1;
            }
        }
        return out;
    }

    /**
     * Register a new passkey with PRF support. One ceremony, no seed
     * derivation. Browsers allow multiple credentials per RP, so this
     * may add a sibling credential — pass `excludeCredentialIds` to
     * surface that case as `PasskeyAlreadyExistsError`.
     *
     * @param {CreatePasskeyRequest} [request]
     * @returns {Promise<RegisteredCredential>} `aaguid`/`backupEligible`
     *   are null on browsers without `getAuthenticatorData()`.
     */
    async createPasskey(request = {}) {
        return await this._registerCredential(request);
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
     * Links API — no external file, no TTL, no caching. The browser's own
     * check is synchronous and deterministic.
     *
     * This method mirrors that browser-side rule so a misconfigured `rpId`
     * (e.g. running a staging build pointed at `keys.breez.technology`
     * while hosted at `staging.example.com`) can be diagnosed before any
     * WebAuthn ceremony runs — producing a `NotAssociated` result with a
     * concrete reason instead of the opaque `SecurityError` the browser
     * would throw.
     *
     * # Return semantics
     *
     * - `rpId` matches the registrable-suffix rule → `Associated`
     * - `rpId` violates the rule → `NotAssociated` with a concrete reason
     * - No `window` / `location.hostname` available (SSR, test runner,
     *   Deno) → `Skipped` — the browser will enforce its own rule at
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
        try {
            return await this._getDualSaltAssertionWithPrf(salt1Bytes, salt2Bytes, options);
        } catch (error) {
            if (this.autoRegister && this._isNoCredentialError(error)) {
                await this._autoRegister();
                return await this._getDualSaltAssertionWithPrf(salt1Bytes, salt2Bytes, options);
            }
            throw error;
        }
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
        const allowList = options.allowCredentialIds || [];
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
        // Immediate mediation is privacy-gated to empty allowCredentials
        // and only enabled when getClientCapabilities() advertises it.
        if (allowCredentials.length === 0 && await this._supportsImmediateGet()) {
            this._applyImmediateOption({ publicKey });
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
            throw new Error('Credential not found');
        }

        const credentialIdBytes = new Uint8Array(credential.rawId);
        if (typeof options.onCredentialId === 'function') {
            try {
                options.onCredentialId(credentialIdBytes);
            } catch {
                // best-effort: bookkeeping must not block seed return
            }
        }
        if (this.credentialRegistry) {
            try {
                await this.credentialRegistry.add(this.rpId, credentialIdBytes);
            } catch {
                // best-effort: registry write must not block seed return
            }
        }
        return credential;
    }

    /**
     * Deduped auto-register. Public `createPasskey()` skips this and
     * always issues a fresh ceremony.
     * @private
     */
    async _autoRegister() {
        if (this._autoRegisterInFlight) {
            return this._autoRegisterInFlight;
        }
        this._autoRegisterInFlight = (async () => {
            try {
                return await this._registerCredential();
            } finally {
                this._autoRegisterInFlight = null;
            }
        })();
        return this._autoRegisterInFlight;
    }

    /**
     * Register a new discoverable credential with PRF extension enabled.
     * @param {CreatePasskeyRequest} [request={}] - Per-call overrides.
     * @returns {Promise<{ credentialId: Uint8Array, aaguid: Uint8Array | null, backupEligible: boolean | null }>}
     * @private
     */
    async _registerCredential(request = {}) {
        const {
            excludeCredentialIds = [],
            userId,
            userName,
            userDisplayName,
        } = request;

        // WebAuthn spec: user.id must be 1-64 bytes.
        let resolvedUserId;
        if (userId !== undefined) {
            if (!(userId instanceof Uint8Array) || userId.length < 1 || userId.length > 64) {
                throw new Error(
                    'CreatePasskeyRequest.userId must be a Uint8Array of length 1-64 bytes'
                );
            }
            resolvedUserId = userId;
        } else {
            resolvedUserId = randomBytes(16);
        }

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
                name: userName ?? this.userName,
                displayName: userDisplayName ?? this.userDisplayName,
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
        const mergedExcludeIds = await this._buildExcludeCredentialIds(excludeCredentialIds);
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
        // 1Password on iOS — only iCloud Keychain implements PRF), the
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
            try {
                await this.credentialRegistry.add(this.rpId, credentialIdBytes);
            } catch {
                // best-effort; registry write must not block return
            }
        }
        return {
            credentialId: credentialIdBytes,
            aaguid: meta ? meta.aaguid : null,
            backupEligible: meta ? meta.backupEligible : null,
        };
    }

    /**
     * Merge caller-supplied excludeCredentialIds with whatever the
     * registry has stored for `this.rpId`. Dedupes by base64url
     * encoding so the same credential isn't sent twice.
     * @private
     */
    async _buildExcludeCredentialIds(callerIds) {
        if (!this.credentialRegistry) {
            return Array.isArray(callerIds) ? [...callerIds] : [];
        }
        let stored;
        try {
            stored = await this.credentialRegistry.list(this.rpId);
        } catch {
            stored = [];
        }
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
     * Check if the error indicates no credential was found.
     * @param {Error} error
     * @returns {boolean}
     * @private
     */
    _isNoCredentialError(error) {
        if (!error) return false;
        const message = error.message || '';
        // Browsers throw NotAllowedError when user cancels or no credential is available.
        // Some browsers distinguish these; we treat "no credentials" as recoverable.
        // The message varies by browser, so check common patterns.
        return (
            message.includes('Credential not found') ||
            message.includes('no credentials') ||
            message.includes('No credentials') ||
            message.includes('empty allowCredentials') ||
            // Chrome-specific: no credential available for the given RP
            (error.name === 'NotAllowedError' && message.includes('not allowed'))
        );
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
    _mapAssertionError(error, elapsedMs) {
        if (!error) return new Error('Unknown WebAuthn error');
        if (error.name === 'NotAllowedError') {
            if (elapsedMs < NO_CRED_FAST_FAIL_MS) {
                return new Error('Credential not found');
            }
            if (elapsedMs >= BIOMETRIC_TIMEOUT_MS) {
                return new PasskeyTimedOutError();
            }
            return new Error('User cancelled authentication');
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
                    return new Error('User cancelled authentication');
                }
                return error;
            case 'SecurityError':
            case 'InvalidStateError':
                return new Error(`Authentication failed: ${error.message}`);
            case 'AbortError':
                return new Error('User cancelled authentication');
            default:
                return error;
        }
    }
}
