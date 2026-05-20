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
 * Thrown when the OS biometric prompt tears down without the user
 * approving or dismissing it: the platform's inactivity timeout
 * (typically ~55 seconds) fired before any user interaction. Distinct
 * from a cancel: hosts may auto-retry or surface a re-prompt UI
 * without treating this as user intent to abandon.
 */
export declare class PasskeyTimedOutError extends Error {
    constructor(message?: string);
}

/**
 * Thrown when `deriveSeeds` cannot match a credential on this device.
 * Surfaces both the WebAuthn fast-fail `NotAllowedError` (no credential
 * for this RP) and the bare "no credential available" path. The
 * `message` carries diagnostic detail and may include the
 * `CredentialRegistry` help suffix when the host had no allow-list and
 * no registry configured.
 */
export declare class PasskeyCredentialNotFoundError extends Error {
    constructor(message?: string);
}

/**
 * Per-call options for {@link PasskeyProvider.deriveSeeds}. All fields
 * optional.
 */
export interface DeriveSeedOptions {
    /**
     * Restrict assertion to one of these credential IDs. The browser
     * refuses any other credential for this RP. Use to pin sign-in to
     * a specific passkey when multiple credentials exist for the RP.
     * Empty / omitted lets the browser pick any matching credential.
     */
    allowCredentialIds?: Uint8Array[];

    /**
     * Per-call control over the platform's "fast-fail when no local
     * credential is available" behavior. On the web the closest
     * analogue is the WebAuthn L3 immediate-mediation flag.
     *
     * `true` (the historical default) opts into immediate mediation
     * when the browser advertises the capability via
     * `PublicKeyCredential.getClientCapabilities('public-key')`. The
     * browser then fast-fails a no-credential ceremony instead of
     * showing the cross-device QR sheet.
     *
     * `false` falls back to the browser's standard picker (cross-
     * device QR sign-in, hybrid transports). Useful for hosts that
     * want users to be able to sign in from a passkey on a paired
     * device.
     *
     * Omitted (the default) preserves the historical behavior.
     */
    preferImmediatelyAvailableCredentials?: boolean;

    /**
     * Fired with the asserted credential's ID after a successful
     * ceremony. Useful for migrating credential-ID tracking. Errors
     * thrown inside are swallowed; the seed return is never blocked.
     */
    onCredentialId?: (credentialId: Uint8Array) => void;
}

/**
 * App-side persistent store of credential IDs registered for an RP.
 * The SDK does not ship a built-in implementation: bring your own
 * via localStorage, IndexedDB, or any custom backend. See the
 * reference implementation in the passkey guide.
 *
 * All methods are called from the SDK as best-effort optimizations:
 * failures and timeouts (3s) are swallowed and surfaced via
 * {@link PasskeyProviderOptions.onRegistryError}; they never block
 * the WebAuthn ceremony.
 *
 * IDs are exchanged as raw `Uint8Array`s; encoding to wire format
 * is the implementation's responsibility.
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
     * Display name shown to the user in the OS passkey picker and
     * credential-management UIs (iCloud Keychain, Google Password
     * Manager, 1Password, etc.) when choosing a credential. Only used
     * at credential registration; changing it does not affect existing
     * credentials.
     */
    rpName: string;

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
     * (the host manages `excludeCredentialIds` / `allowCredentialIds`
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
     * Constant identifying Breez's shared `keys.breez.technology` RP.
     * Pass as `rpId` when opting into the Breez-managed Relying Party
     * (only valid for apps registered with Breez). Apps with their own
     * RP domain pass their own string.
     */
    static readonly BREEZ_RP_ID: string;

    constructor(options: PasskeyProviderOptions);

    /**
     * Derive one or more 32-byte PRF outputs in as few user prompts as
     * the authenticator supports. Pairs salts into `prf.eval.first` +
     * `prf.eval.second` per ceremony where the platform honors it.
     * Authenticators that drop `second` fall back to single-salt for
     * the affected salt; worst case prompt count is one per salt.
     * Output order matches input order.
     *
     * @throws If authentication fails, PRF is not supported, or the user cancels.
     */
    deriveSeeds(salts: string[], options?: DeriveSeedOptions): Promise<Uint8Array[]>;

    /**
     * Create a new passkey with PRF support.
     *
     * `excludeCredentialIds` (optional) lists credential IDs the
     * authenticator must refuse to duplicate. When any entry matches a
     * credential already on the device, `PasskeyAlreadyExistsError` is
     * raised. The `user.id` is always provider-minted and returned via
     * `RegisteredCredential.userId`.
     *
     * @throws {PasskeyAlreadyExistsError} If an entry in `excludeCredentialIds`
     *   matches a credential already on the device.
     * @throws If the user cancels or PRF is not supported by the
     *   authenticator.
     */
    createPasskey(excludeCredentialIds?: Uint8Array[]): Promise<RegisteredCredential>;

    /**
     * Check if a PRF-capable passkey is available on this device.
     *
     * @returns true if WebAuthn with PRF extension is likely supported.
     */
    isSupported(): Promise<boolean>;

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

    /**
     * Return the credential IDs the configured {@link CredentialRegistry}
     * has stored for the current `rpId`. Backs
     * `PasskeyClient.credentials().get()`. Returns an empty list when
     * no registry is configured.
     */
    getKnownCredentialIds(): Promise<Uint8Array[]>;

    /**
     * Drop a single credential ID from the configured registry. Backs
     * `PasskeyClient.credentials().remove(id)`. No-op when no registry
     * is configured.
     */
    removeKnownCredentialId(credentialId: Uint8Array): Promise<void>;

    /**
     * Clear the configured registry's persisted credential-ID list for
     * the current `rpId`. Backs `PasskeyClient.credentials().clear()`.
     * No-op when no registry is configured.
     */
    clearKnownCredentialIds(): Promise<void>;
}
