/**
 * Result of a domain-association verification check against the platform's
 * well-known configuration source. Mirrors the Rust `DomainAssociation`
 * enum shape so cross-language callers can handle it uniformly.
 */
export type DomainAssociation =
    | { kind: 'Associated' }
    | { kind: 'NotAssociated'; source: string; reason: string }
    | { kind: 'Skipped'; reason: string };

/**
 * Thrown when `createPasskey` asks the platform to register a new
 * passkey but it refuses because an entry in `excludeCredentialIds`
 * matches a credential already on the device. Hosts should route the
 * user to the sign-in path instead of treating this as a generic
 * registration failure.
 */
export declare class PasskeyAlreadyExistsError extends Error {
    constructor(message?: string);
}

/**
 * Options for constructing a PasskeyProvider.
 */
export interface PasskeyProviderOptions {
    /**
     * Relying Party ID. Must match the domain hosting your passkeys. On native
     * platforms this corresponds to the AASA / assetlinks.json domain. On web,
     * rpId must be a registrable suffix of window.location.hostname
     * for cross-platform credential sharing.
     *
     * Changing this after users have registered passkeys will make their existing
     * credentials undiscoverable, they would need to create new passkeys.
     * @default 'keys.breez.technology'
     */
    rpId?: string;

    /**
     * RP display name shown during credential registration. Only used when
     * creating new passkeys; changing it does not affect existing credentials.
     * @default 'Breez SDK'
     */
    rpName?: string;

    /**
     * User name stored with the credential, shown as a secondary label in some
     * passkey managers. Defaults to rpName. Only used during registration;
     * changing it does not affect existing credentials.
     */
    userName?: string;

    /**
     * User display name shown as the primary label in the passkey picker.
     * Defaults to userName. Only used during registration; changing it does
     * not affect existing credentials.
     */
    userDisplayName?: string;

    /**
     * When true (default), `derivePrfSeed` automatically creates a new passkey
     * if none exists for this RP ID, then retries the assertion. When false,
     * throws an error instead, letting the caller control registration
     * separately via `createPasskey()`.
     * @default true
     */
    autoRegister?: boolean;

    /**
     * When non-empty, restricts assertion (sign-in) to one of the listed
     * credential IDs. The browser refuses any other credential for this
     * RP, even if it matches the RP. Use this to bind sign-in to a
     * specific passkey the caller has registered, instead of letting
     * the browser pick any sibling credential that happens to share the
     * RP. Critical for deterministic seed derivation when multiple
     * credentials might exist for the same RP. When empty (default),
     * the browser picks any credential matching the RP.
     * @default []
     */
    allowCredentialIds?: Uint8Array[];
}

/**
 * Built-in passkey-based PRF provider for browser environments.
 *
 * Implements the PrfProvider interface using the WebAuthn API with the PRF
 * extension (navigator.credentials.create/get).
 *
 * Uses discoverable credentials (resident keys) so no credential storage is needed.
 * On first use, if no credential exists for the RP ID, a new passkey is
 * automatically created (registered), then the assertion is retried.
 *
 * @example
 * ```typescript
 * import { Passkey } from '@breeztech/breez-sdk-spark'
 * import { PasskeyProvider } from '@breeztech/breez-sdk-spark/passkey-prf-provider'
 *
 * const prfProvider = new PasskeyProvider()
 * const passkey = new Passkey(prfProvider, undefined)
 * const wallet = await passkey.getWallet('personal')
 * ```
 */
export declare class PasskeyProvider {
    constructor(options?: PasskeyProviderOptions);

    /**
     * List of base64 credential IDs to constrain assertion to. Mirrors
     * the `allowCredentialIds` constructor option but is also settable
     * at runtime so callers can re-target sign-in to a refreshed list
     * (e.g. after a synced-storage hydrate) without reconstructing the
     * provider. Empty list (default) lets the platform pick any
     * credential matching the RP.
     */
    allowCredentialIds: Uint8Array[];

    /**
     * Optional callback fired with the credential ID returned by every
     * successful WebAuthn assertion (sign-in path). Hosts can set this
     * to record which credential was just used so they can populate
     * `excludeCredentialIds` and `allowCredentialIds` on subsequent
     * requests.
     *
     * Useful for migrating users whose passkey predates the host's own
     * credential-ID tracking: the first successful sign-in surfaces
     * the credential ID, after which the host's records are correct
     * and the platform-level "already exists" check can fire on
     * future create attempts.
     *
     * Set before calling `derivePrfSeed`. Not invoked on registration
     * (see `createPasskey`'s return value for that).
     */
    onAssertionCredentialId?: (credentialId: Uint8Array) => void;

    /**
     * Derive a 32-byte seed from passkey PRF with the given salt.
     *
     * Authenticates the user via a platform passkey and evaluates the PRF extension.
     * If no credential exists for this RP ID, a new passkey is created automatically.
     *
     * @param salt - The salt string to use for PRF evaluation.
     * @returns The 32-byte PRF output.
     * @throws If authentication fails, PRF is not supported, or the user cancels.
     */
    derivePrfSeed(salt: string): Promise<Uint8Array>;

    /**
     * Derive multiple 32-byte PRF outputs in as few user prompts as the
     * authenticator supports. The WebAuthn PRF extension allows two
     * salts per assertion (`prf.eval.first` + `prf.eval.second`),
     * collapsing two derivations into a single ceremony on browsers
     * that honor the spec.
     *
     * Salt count semantics:
     * - 0 salts: returns empty without prompting.
     * - 1 salt: equivalent to `derivePrfSeed`.
     * - 2 salts: 1 ceremony where supported, 2 otherwise.
     * - 3+ salts: pairs are batched. Trailing odd salt uses single-salt.
     *
     * Authenticators that silently drop the second salt fall back to a
     * sequential single-salt assertion for the affected salt(s); worst
     * case prompt count matches looping `derivePrfSeed`.
     *
     * Output ordering matches input ordering.
     *
     * @param salts - Salt strings in caller order.
     * @returns One 32-byte output per salt, in input order.
     */
    derivePrfSeeds(salts: string[]): Promise<Uint8Array[]>;

    /**
     * Create a new passkey with PRF support.
     *
     * Only registers the credential, no seed derivation. Triggers exactly
     * 1 WebAuthn prompt. Use this to separate credential creation from
     * derivation in multi-step onboarding flows.
     *
     * @param excludeCredentialIds - Optional list of credential IDs to exclude.
     *   Pass previously created credential IDs to prevent the authenticator
     *   from creating a duplicate on the same device.
     * @returns The credential ID of the newly created passkey.
     * @throws {PasskeyAlreadyExistsError} If an entry in `excludeCredentialIds`
     *   matches a credential already on the device.
     * @throws If the user cancels or PRF is not supported by the authenticator.
     */
    createPasskey(excludeCredentialIds?: Uint8Array[]): Promise<Uint8Array>;

    /**
     * Check if a PRF-capable passkey is available on this device.
     *
     * @returns true if WebAuthn with PRF extension is likely supported.
     */
    isPrfAvailable(): Promise<boolean>;

    /**
     * Verify the configured `rpId` is a valid scope for WebAuthn from the
     * current document's origin.
     *
     * Browsers validate `rp.id` locally at `navigator.credentials.get / create`
     * time: `rpId` must be a registrable suffix of `window.location.hostname`,
     * or equal to it. There is no AASA / assetlinks equivalent on the web,
     * no external file, no TTL, no caching.
     *
     * This method mirrors that browser-side rule so a misconfigured `rpId`
     * (e.g., a staging build pointed at `keys.breez.technology` while hosted
     * at `staging.example.com`) can be diagnosed with a concrete reason
     * before the first WebAuthn ceremony, instead of the opaque
     * `SecurityError` the browser would otherwise throw.
     *
     * @returns `Associated` when the rpId is in scope, `NotAssociated` with
     *   a concrete reason when it isn't, or `Skipped` when the check can't
     *   run (SSR / no `window.location`).
     */
    checkDomainAssociation(): Promise<DomainAssociation>;
}

/**
 * @deprecated Use PasskeyProviderOptions instead. This alias will be removed in a future release.
 */
export type PasskeyPrfProviderOptions = PasskeyProviderOptions;

/**
 * @deprecated Use PasskeyProvider instead. This alias will be removed in a future release.
 */
export { PasskeyProvider as PasskeyPrfProvider };
