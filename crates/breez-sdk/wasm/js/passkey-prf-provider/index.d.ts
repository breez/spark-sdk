import { PasskeyClient as SdkPasskeyClient } from '../breez_sdk_spark_wasm';
import type { PasskeyConfig, PrfProvider } from '../breez_sdk_spark_wasm';

/**
 * Outcome of a domain-association check: whether `rpId` is a valid
 * WebAuthn scope for the current origin.
 */
export type DomainAssociation =
    | { kind: 'Associated' }
    | { kind: 'NotAssociated'; source: string; reason: string }
    | { kind: 'Skipped'; reason: string };

/**
 * Authenticator data captured at registration. `aaguid` is the 16-byte
 * Authenticator Attestation GUID (provider identifier); `backupEligible`
 * is the BE flag indicating whether the credential can sync across
 * devices. Both are `null` when the platform doesn't expose enough
 * authenticator data to extract them.
 *
 * `userId` is the WebAuthn user handle the provider generated for this
 * credential. Always returned; never host-supplied.
 *
 * AAGUID is unverified attestation. Use as a display hint only, never
 * for trust decisions.
 */
export interface RegisteredCredential {
    credentialId: Uint8Array;
    userId: Uint8Array;
    aaguid: Uint8Array | null;
    backupEligible: boolean | null;
}

/**
 * Result of {@link PasskeyProvider.deriveSeeds}: the 32-byte outputs in
 * input order plus the credential ID observed in the same assertion
 * (null when `salts` was empty).
 */
export interface DeriveSeedsResult {
    seeds: Uint8Array[];
    credentialId: Uint8Array | null;
}

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
 * the device. `message` carries diagnostic detail and may append a
 * `CredentialRegistry` hint when no allow-list and no registry were
 * configured.
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

/** Per-call options for {@link PasskeyProvider.deriveSeeds}. */
export interface DeriveSeedOptions {
    /**
     * Credential IDs the assertion is restricted to, for
     * reauthenticating a known user without an account picker. Empty or
     * unset lets the browser pick any matching credential for this RP.
     */
    allowCredentials?: Uint8Array[];

    /**
     * Requests fast-fail when no local credential is available. Ignored
     * by the web provider: the WebAuthn flag it maps to is still
     * experimental, so the standard browser picker is always shown.
     */
    preferImmediatelyAvailableCredentials?: boolean;
}

/**
 * App-supplied persistent store of credential IDs for an RP (the SDK
 * ships no implementation: back it with localStorage, IndexedDB, etc).
 * All calls are best-effort: failures and 3s timeouts are swallowed and
 * surfaced via {@link PasskeyProviderOptions.onRegistryError}, never
 * blocking the WebAuthn ceremony. IDs are raw `Uint8Array`s.
 */
export interface CredentialRegistry {
    read(rpId: string): Promise<Uint8Array[]>;
    add(rpId: string, credentialId: Uint8Array): Promise<void>;
    remove(rpId: string, credentialId: Uint8Array): Promise<void>;
    clear(rpId: string): Promise<void>;
}

/** Discriminator for {@link PasskeyProviderOptions.onRegistryError}. */
export type RegistryOperation = 'read' | 'add' | 'remove' | 'clear';

/**
 * Options for constructing a PasskeyProvider. `rpId` is required: pass
 * {@link PasskeyProvider.BREEZ_RP_ID} to opt into Breez's shared RP.
 */
export interface PasskeyProviderOptions {
    /**
     * Relying Party ID. Must match the domain hosting your passkeys. On
     * native platforms this corresponds to the AASA / assetlinks.json
     * domain. On web, rpId must be a registrable suffix of
     * window.location.hostname for cross-platform credential sharing.
     *
     * Changing this after users have registered passkeys will make
     * their existing credentials undiscoverable, they would need to
     * create new passkeys. Pass {@link PasskeyProvider.BREEZ_RP_ID} to
     * opt into Breez's shared `keys.breez.technology` RP (only valid
     * for Breez-registered apps).
     */
    rpId: string;

    /**
     * Maps to the WebAuthn `rp.name`. Deprecated in WebAuthn L3 but
     * still required by all current browser implementations: the
     * provider rejects an empty string at construction to keep
     * registrations interoperable. Surfaces in some credential-
     * management UIs (iCloud Keychain, Google Password Manager,
     * 1Password); platform UIs increasingly ignore it. Only used at
     * credential registration; changing it does not affect existing
     * credentials.
     */
    rpName: string;

    /**
     * Maps to the WebAuthn `user.name`. Treated as the user's unique
     * identifier for the credential and shown in the account picker
     * during sign-in (Apple's Passwords app, in particular, dedupes
     * credentials by `(rpId, user.name)`). Pass a stable per-user
     * value if you want each registration to surface as a distinct
     * entry. Defaults to `rpName`. Only used at registration; changing
     * it does not affect existing credentials.
     */
    userName?: string;

    /**
     * Maps to the WebAuthn `user.displayName`. The user-friendly label
     * the browser MAY (but is not required to) show in the picker.
     * Behavior varies by platform: some show it as the primary label,
     * others only as a secondary one, others ignore it entirely.
     * Defaults to `userName`. Only used at registration; changing it
     * does not affect existing credentials.
     */
    userDisplayName?: string;

    /**
     * When set, narrows the create-time UI to the chosen authenticator
     * class. `'platform'` scopes registration to the local platform
     * authenticator (Touch ID / Face ID / Windows Hello / iCloud
     * Keychain), suppressing security-key and hybrid (cross-device)
     * options in the browser's chooser. `'cross-platform'` is the
     * inverse: only roaming authenticators (USB / NFC / BLE security
     * keys, hybrid). When omitted, the browser shows all available
     * authenticators.
     */
    authenticatorAttachment?: 'platform' | 'cross-platform';

    /**
     * WebAuthn L3 priority hints, applied to both create() and get()
     * public-key options. Soft signal compared to
     * `authenticatorAttachment` (which is create-only): browsers that
     * honor it surface the listed authenticator classes first;
     * browsers that ignore it fall back to default ordering. Stacks
     * with `authenticatorAttachment` on create. Pass `['client-device']`
     * to nudge platform authenticator before security-key / hybrid
     * options. The get() side is the only standards-track lever for
     * influencing the sign-in picker (since `authenticatorAttachment`
     * is not allowed there).
     */
    hints?: ('client-device' | 'security-key' | 'hybrid')[];

    /**
     * Default WebAuthn `timeout` (milliseconds) applied to every
     * create() and get() ceremony. The value surfaces as a hint to
     * the user agent; platforms still apply their own internal cap
     * (60s on iOS, similar on Android Credential Manager). Pass a
     * value 5 to 10 seconds under the platform cap so a host-side
     * "looks like a timeout" heuristic can fire before the OS
     * tears the prompt down. When undefined (default), the platform
     * default applies.
     */
    defaultTimeoutMs?: number;

    /**
     * Optional opt-in registry. When set, the provider auto-merges
     * stored IDs into `excludeCredentials` on `createPasskey` and
     * into `allowCredentials` on assertion, then auto-adds new
     * credential IDs after success. Omit to disable auto-population
     * (the host manages `excludeCredentials` / `allowCredentials`
     * manually). All registry calls are best-effort with a 3s
     * timeout; failures fire {@link onRegistryError} and the
     * ceremony proceeds.
     *
     * The constructor performs a conformance check: the supplied
     * object must expose `read`, `add`, `remove`, `clear` as
     * functions. Missing methods cause an immediate throw so
     * misconfiguration surfaces at startup rather than at first
     * sign-in.
     */
    credentialRegistry?: CredentialRegistry;

    /**
     * Fired when a {@link CredentialRegistry} call throws or times
     * out. Best-effort: invocation never blocks ceremony progress.
     */
    onRegistryError?: (operation: RegistryOperation, error: Error) => void;
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

    constructor(options: PasskeyProviderOptions);

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
    createPasskey(excludeCredentials?: Uint8Array[]): Promise<RegisteredCredential>;

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

    /**
     * Credential IDs the configured registry has stored for the current
     * `rpId`. Empty when no registry is configured. Backs
     * `PasskeyClient.credentials().get()`.
     */
    getKnownCredentialIds(): Promise<Uint8Array[]>;

    /**
     * Drop one credential ID from the registry. No-op when no registry
     * is configured. Backs `PasskeyClient.credentials().remove(id)`.
     */
    removeKnownCredentialId(credentialId: Uint8Array): Promise<void>;

    /**
     * Clear the registry's stored IDs for the current `rpId`. No-op when
     * no registry is configured. Backs `PasskeyClient.credentials().clear()`.
     */
    clearKnownCredentialIds(): Promise<void>;
}

/**
 * Builder for a {@link PasskeyClient} with a caller-supplied
 * `PrfProvider`. Use it when you need a configured {@link PasskeyProvider}
 * (custom `rpId`/`rpName`, a `credentialRegistry`, timeout overrides) or
 * a custom PRF backend. For the zero-config Breez-RP case, use the
 * {@link PasskeyClient} constructor directly.
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
 * Breez-registered app needs only its relay key. Apps with their own RP,
 * a credential registry, or a custom PRF backend inject their own
 * provider through {@link PasskeyClientBuilder}. The instance is the
 * underlying SDK client (`checkAvailability`, `register`, `signIn`,
 * `labels()`, `credentials()`).
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
