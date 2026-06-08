import { PasskeyClient as SdkPasskeyClient } from '../breez_sdk_spark_wasm.js';
import type {
    PasskeyConfig,
    PasskeyProviderOptions,
    PrfProvider,
    PasskeyCredential,
    DeriveSeedsResult,
    DeriveSeedOptions
} from '../breez_sdk_spark_wasm.js';

export type { PasskeyCredential, DeriveSeedsResult, DeriveSeedOptions, PasskeyProviderOptions };

/**
 * Outcome of a domain-association check: whether `rpId` is a valid
 * WebAuthn scope for the current origin.
 */
export type DomainAssociation =
    | { kind: 'Associated' }
    | { kind: 'NotAssociated'; source: string; reason: string }
    | { kind: 'Skipped'; reason: string };

/**
 * Thrown by `createPasskey` when an entry in `excludeCredentials`
 * matches a credential already on the device. Route the user to
 * sign-in rather than treating it as a generic registration failure.
 */
export declare class PasskeyAlreadyExistsError extends Error {
    constructor(message?: string);
}

/**
 * Thrown when the OS biometric prompt times out (around 55s) before
 * any user interaction. Distinct from a cancel: hosts may auto-retry
 * since the user did not deliberately abandon the flow.
 */
export declare class PasskeyTimedOutError extends Error {
    constructor(message?: string);
}

/**
 * Thrown when `deriveSeeds` cannot match a credential for this RP on
 * the device. `message` carries diagnostic detail.
 */
export declare class PasskeyCredentialNotFoundError extends Error {
    constructor(message?: string);
}

/**
 * Thrown when the user actively dismisses the OS passkey prompt.
 * Distinct from `PasskeyTimedOutError`: hosts should not auto-retry a
 * deliberate cancel.
 */
export declare class PasskeyUserCancelledError extends Error {
    constructor(message?: string);
}

/**
 * Web-only options for the built-in {@link PasskeyProvider}, passed as the
 * second constructor argument. The cross-platform Relying Party / user
 * fields live on {@link PasskeyProviderOptions} (the first argument), and
 * are also settable on `PasskeyConfig` for the zero-config client.
 */
export interface PasskeyProviderWebOptions {
    /**
     * Narrows the create-time chooser to one authenticator class.
     * `'platform'` allows only the local authenticator (Touch ID, Face
     * ID, Windows Hello, iCloud Keychain); `'cross-platform'` only
     * roaming keys (USB, NFC, BLE, hybrid). Unset shows all.
     */
    authenticatorAttachment?: 'platform' | 'cross-platform';

    /**
     * WebAuthn L3 priority hints applied to both create and get,
     * ordering the authenticator classes a supporting browser offers
     * first (ignored otherwise). Pass `['client-device']` to favor the
     * platform authenticator. This is the only standards-track lever for
     * the sign-in picker, where `authenticatorAttachment` is not allowed.
     */
    hints?: ('client-device' | 'security-key' | 'hybrid')[];

    /**
     * Default WebAuthn `timeout` (ms) for every create and get. A hint
     * only: platforms cap around 60s. Set it 5 to 10s under the cap so a
     * host-side "looks like a timeout" heuristic can fire before the OS
     * tears the prompt down. Unset uses the platform default.
     */
    defaultTimeoutMs?: number;
}

/**
 * Built-in browser PRF provider: implements `PrfProvider` over the
 * WebAuthn PRF extension, using discoverable credentials so no
 * credential storage is needed.
 *
 * @example
 * ```typescript
 * import { PasskeyClient } from '@breeztech/breez-sdk-spark'
 * import { PasskeyProvider } from '@breeztech/breez-sdk-spark/passkey-prf-provider'
 *
 * const provider = new PasskeyProvider()
 * const client = new PasskeyClient(provider)
 * const { wallet } = await client.signIn({ label: 'personal' })
 * ```
 */
export declare class PasskeyProvider {
    /**
     * Breez's shared `keys.breez.technology` RP. Pass as `rpId` to opt
     * in (Breez-registered apps only); other apps pass their own domain.
     */
    static readonly BREEZ_RP_ID: string;

    /**
     * Default `rpName` for the zero-config {@link PasskeyClient} /
     * {@link PasskeyClientBuilder} path.
     */
    static readonly DEFAULT_RP_NAME: string;

    constructor(options?: PasskeyProviderOptions, webOptions?: PasskeyProviderWebOptions);

    /**
     * Derive one 32-byte PRF output per salt (in input order), pairing
     * salts into a single ceremony where the authenticator supports it.
     * Worst case is one prompt per salt.
     *
     * @throws If authentication fails, PRF is not supported, or the user cancels.
     */
    deriveSeeds(salts: string[], options?: DeriveSeedOptions): Promise<DeriveSeedsResult>;

    /**
     * Create a new PRF-capable passkey. Pass `excludeCredentials`
     * (already-registered IDs) to prevent re-registering the same
     * device. The `user.id` is provider-minted and returned as `userId`.
     *
     * @throws {PasskeyAlreadyExistsError} If an entry in
     *   `excludeCredentials` already exists on the device.
     * @throws If the user cancels or PRF is not supported.
     */
    createPasskey(excludeCredentials?: Uint8Array[]): Promise<PasskeyCredential>;

    /**
     * Check if a PRF-capable passkey is available on this device.
     *
     * @returns true if WebAuthn with PRF extension is likely supported.
     */
    isSupported(): Promise<boolean>;

    /**
     * Check whether the configured `rpId` is a valid WebAuthn scope for
     * the current origin (must be a registrable suffix of
     * `window.location.hostname`, or equal to it). Mirrors the browser's
     * own rule so a misconfigured `rpId` is diagnosed with a concrete
     * reason instead of an opaque `SecurityError` at ceremony time.
     *
     * @returns `Associated` when in scope, `NotAssociated` with a reason
     *   when not, or `Skipped` when no `window.location` is available.
     */
    checkDomainAssociation(): Promise<DomainAssociation>;
}

/**
 * Builder for a {@link PasskeyClient} with a caller-supplied
 * `PrfProvider`. Use it when you need a configured {@link PasskeyProvider}
 * (custom `rpId`/`rpName`, timeout overrides) or a custom PRF backend.
 * For the zero-config Breez-RP case, use the {@link PasskeyClient}
 * constructor directly.
 */
export declare class PasskeyClientBuilder {
    /**
     * @param breezApiKey - Breez relay key for authenticated (NIP-42)
     *   label storage. Omit for public relays only.
     * @param config - `rpId` / `rpName` configure the built-in provider
     *   (ignored when one is injected via {@link withPrfProvider});
     *   `defaultLabel` is the label-store default.
     */
    constructor(breezApiKey?: string, config?: PasskeyConfig);

    /** Inject the `PrfProvider` the client derives seeds through. */
    withPrfProvider(provider: PrfProvider): PasskeyClientBuilder;

    /**
     * Build the client, defaulting to a browser {@link PasskeyProvider}
     * on the Breez RP when no provider was injected.
     */
    build(): SdkPasskeyClient;
}

/**
 * High-level passkey client. The zero-config constructor wires the
 * built-in browser {@link PasskeyProvider} on the Breez shared RP, so a
 * Breez-registered app needs only its relay key. Apps with their own RP
 * or a custom PRF backend inject their own provider through
 * {@link PasskeyClientBuilder}. The instance is the underlying SDK client
 * (`checkAvailability`, `register`, `signIn`, `labels()`).
 */
export declare class PasskeyClient extends SdkPasskeyClient {
    /**
     * @param breezApiKey - Breez relay key for authenticated (NIP-42)
     *   label storage. Omit for public relays only.
     * @param config - Optional `rpId` / `rpName` for the built-in
     *   provider (default: the Breez shared RP) plus `defaultLabel`.
     */
    constructor(breezApiKey?: string, config?: PasskeyConfig);
}
