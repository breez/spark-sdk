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
 * Options for constructing a PasskeyPrfProvider.
 */
export interface PasskeyPrfProviderOptions {
    /**
     * Relying Party ID. Must match the domain configured in .well-known/webauthn
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
 * import { PasskeyPrfProvider } from '@breeztech/breez-sdk-spark/passkey-prf-provider'
 *
 * const prfProvider = new PasskeyPrfProvider()
 * const passkey = new Passkey(prfProvider, undefined)
 * const wallet = await passkey.getWallet('personal')
 * ```
 */
export declare class PasskeyPrfProvider {
    constructor(options?: PasskeyPrfProviderOptions);

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
