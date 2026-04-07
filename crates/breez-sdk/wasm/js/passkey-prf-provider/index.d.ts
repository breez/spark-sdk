/**
 * Options for constructing a WebAuthnPrfProvider.
 */
export interface WebAuthnPrfProviderOptions {
    /**
     * Relying Party ID. Must match the domain configured in .well-known/webauthn
     * for cross-platform credential sharing.
     *
     * Changing this after users have registered passkeys will make their existing
     * credentials undiscoverable — they would need to create new passkeys.
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
 * WebAuthn-based PRF provider for browser environments.
 *
 * Implements the PasskeyPrfProvider interface using the WebAuthn API
 * with the PRF extension (navigator.credentials.create/get).
 *
 * Uses discoverable credentials (resident keys) so no credential storage is needed.
 * On first use, if no credential exists for the RP ID, a new passkey is
 * automatically created (registered), then the assertion is retried.
 *
 * @example
 * ```typescript
 * import { WebAuthnPrfProvider, Passkey } from '@breeztech/breez-sdk-spark'
 *
 * const prfProvider = new WebAuthnPrfProvider()
 * const passkey = new Passkey(prfProvider, undefined)
 * const wallet = await passkey.getWallet('personal')
 * ```
 */
export declare class WebAuthnPrfProvider {
    constructor(options?: WebAuthnPrfProviderOptions);

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
     * Only registers the credential — no seed derivation. Triggers exactly
     * 1 WebAuthn prompt. Use this to separate credential creation from
     * derivation in multi-step onboarding flows.
     *
     * @throws If the user cancels or PRF is not supported by the authenticator.
     */
    createPasskey(): Promise<void>;

    /**
     * Check if a PRF-capable passkey is available on this device.
     *
     * @returns true if WebAuthn with PRF extension is likely supported.
     */
    isPrfAvailable(): Promise<boolean>;
}
