import { NativeModules, Platform } from 'react-native';

const { BreezSdkSparkPasskey } = NativeModules;

/**
 * Build a diagnostic error thrown when the native passkey module isn't
 * reachable from JS. The most common cause on iOS is running on a version
 * older than iOS 18: the Swift class is marked `@available(iOS 18.0, *)`
 * so the ObjC runtime cannot load it on earlier releases. On Android the
 * native module should always load, so the fallback message blames linking.
 */
function passkeyModuleUnavailableError(operation: string): Error {
  if (Platform.OS === 'ios') {
    const version = parseFloat(String(Platform.Version));
    if (!Number.isNaN(version) && version < 18) {
      return new Error(
        `Passkey PRF requires iOS 18.0 or later. ` +
          `This device is running iOS ${Platform.Version}, where ` +
          `ASAuthorizationPlatformPublicKeyCredentialPRFAssertionInput is not available. ` +
          `${operation} is unsupported on this device.`
      );
    }
    return new Error(
      `Passkey PRF native module (BreezSdkSparkPasskey) failed to load on iOS. ` +
        `This normally means the iOS deployment target is lower than 18.0 or ` +
        `the pod was not linked. ${operation} is unavailable.`
    );
  }
  if (Platform.OS === 'android') {
    return new Error(
      `Passkey PRF native module (BreezSdkSparkPasskey) is not registered. ` +
        `Check that @breeztech/breez-sdk-spark-react-native is autolinked and ` +
        `that BreezSdkSparkPasskeyModule appears in BreezSdkSparkReactNativePackage. ` +
        `${operation} is unavailable.`
    );
  }
  return new Error(
    `Passkey PRF is only supported on iOS 18+ and Android 9+. ` +
      `${operation} is not available on ${Platform.OS}.`
  );
}

/**
 * Authenticator data captured at registration. `aaguid` is the 16-byte
 * Authenticator Attestation GUID (provider identifier); `backupEligible`
 * is the BE flag indicating whether the credential can sync across
 * devices. Both are `null` when the attestation can't be parsed. AAGUID
 * is unverified attestation: display hint only, never a trust decision.
 *
 * `userId` is the WebAuthn user handle the native plugin minted for
 * this credential. Always returned; never host-supplied.
 */
export interface RegisteredCredential {
  credentialId: Uint8Array;
  userId: Uint8Array;
  aaguid: Uint8Array | null;
  backupEligible: boolean | null;
}


/**
 * Result of {@link PasskeyProvider.checkDomainAssociation}. Mirrors the
 * Rust `DomainAssociation` enum shape so cross-language callers can
 * switch on `kind` regardless of which platform produced the result.
 */
export type DomainAssociation =
  | { kind: 'Associated' }
  | { kind: 'NotAssociated'; source: string; reason: string }
  | { kind: 'Skipped'; reason: string };

/**
 * App-side persistent store of credential IDs registered for an RP.
 * The SDK does not ship a built-in implementation: bring your own
 * (Keychain on iOS, Block Store + SharedPreferences on Android, or
 * any custom backend). See the reference implementations in the
 * passkey guide.
 *
 * All methods are called from the SDK as best-effort optimizations:
 * failures and timeouts (3s) are swallowed and surfaced via
 * {@link PasskeyProviderOptions.onRegistryError}; they never block
 * the WebAuthn ceremony.
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
 * Options for constructing a PasskeyProvider.
 */
export interface PasskeyProviderOptions {
  /**
   * Relying Party ID. Must match the domain configured for cross-platform
   * credential sharing.
   *
   * Changing this after users have registered passkeys will make their
   * existing credentials undiscoverable: they would need to create
   * new passkeys. Pass {@link PasskeyProvider.BREEZ_RP_ID} to opt into
   * Breez's shared `keys.breez.technology` RP (only valid for
   * Breez-registered apps).
   */
  rpId: string;

  /**
   * Maps to the WebAuthn `rp.name`. Deprecated in WebAuthn L3 but
   * still required by current OS prompts. Surfaces in some
   * credential-management UIs (iCloud Keychain, Google Password
   * Manager, 1Password); platform UIs increasingly ignore it. Only
   * used at credential registration; changing it does not affect
   * existing credentials.
   */
  rpName: string;

  /**
   * Maps to the WebAuthn `user.name`. Treated as the user's unique
   * identifier for the credential and shown in the account picker
   * during sign-in. Pass a stable per-user value if each registration
   * should surface as a distinct entry (Apple's Passwords app, in
   * particular, dedupes credentials by `(rpId, user.name)`). Defaults
   * to `rpName`. Only used at registration; changing it does not
   * affect existing credentials.
   */
  userName?: string;

  /**
   * Maps to the WebAuthn `user.displayName`. The user-friendly label
   * the OS / browser MAY (but is not required to) show in the
   * picker; behavior varies by platform. Defaults to `userName`. Only
   * used at registration; changing it does not affect existing
   * credentials.
   */
  userDisplayName?: string;

  /**
   * Optional opt-in registry. When set, the JS-side wrapper merges
   * stored IDs into `allowCredentials` before each native call and
   * writes the asserted / created credential ID back after success.
   * The native module never sees the registry: all bookkeeping is
   * done in JS. All registry calls are best-effort with a 3s
   * timeout; failures fire {@link onRegistryError} and the ceremony
   * proceeds.
   */
  credentialRegistry?: CredentialRegistry;

  /**
   * Fired when a {@link CredentialRegistry} call throws or times
   * out. Best-effort: invocation never blocks ceremony progress.
   */
  onRegistryError?: (operation: RegistryOperation, error: Error) => void;
}

const REGISTRY_TIMEOUT_MS = 3_000;

const CREDENTIAL_REGISTRY_HELP_SUFFIX =
  ' (No CredentialRegistry was supplied to PasskeyProvider; ' +
  'if you expect the SDK to auto-discover known credentials, see ' +
  'https://sdk-doc-spark.breez.technology/guide/passkey.html#credentialregistry)';

const _REGISTRY_TIMEOUT = Symbol('registryTimeout');

function _withRegistryTimeout<T>(p: Promise<T>): Promise<T | typeof _REGISTRY_TIMEOUT> {
  return Promise.race([
    p,
    new Promise<typeof _REGISTRY_TIMEOUT>((resolve) =>
      setTimeout(() => resolve(_REGISTRY_TIMEOUT), REGISTRY_TIMEOUT_MS)
    ),
  ]);
}

async function _registryReadBestEffort(
  registry: CredentialRegistry,
  rpId: string,
  onRegistryError?: (op: RegistryOperation, err: Error) => void
): Promise<Uint8Array[]> {
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
    onRegistryError?.('read', err as Error);
    return [];
  }
}

function _registryAddFireAndForget(
  registry: CredentialRegistry,
  rpId: string,
  credentialId: Uint8Array,
  onRegistryError?: (op: RegistryOperation, err: Error) => void
): void {
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
      onRegistryError?.('add', err as Error);
    });
}

/**
 * Error thrown by [PasskeyProvider] when a passkey operation fails.
 * Provides a structured `code` for programmatic handling.
 *
 * `code` values: `userCancelled`, `userTimedOut`, `prfNotSupported`,
 * `noCredential`, `configuration`, `credentialAlreadyExists`, `unknown`.
 *
 * `userTimedOut` distinguishes the OS biometric inactivity timeout
 * (~55s+ with no user interaction) from `userCancelled` (the user
 * actively dismissed the prompt). Hosts may auto-retry on
 * `userTimedOut` without treating it as user intent to abandon.
 */
export class PasskeyPrfException extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = 'PasskeyPrfException';
    this.code = code;
  }
}

/**
 * Map a native bridge rejection (RN passes `{ code, message }` on the
 * thrown error) into a typed [PasskeyPrfException].
 */
function mapNativeError(err: unknown): PasskeyPrfException {
  // The RN bridge encodes the native error code as `err.code` (matching the
  // first arg passed to `promise.reject(...)` on the native side).
  const anyErr = err as { code?: string; message?: string };
  const message = anyErr?.message ?? 'Unknown passkey error';
  switch (anyErr?.code) {
    case 'ERR_USER_CANCELLED':
      return new PasskeyPrfException('userCancelled', message);
    case 'ERR_USER_TIMED_OUT':
      return new PasskeyPrfException('userTimedOut', message);
    case 'ERR_PRF_NOT_SUPPORTED':
      return new PasskeyPrfException('prfNotSupported', message);
    case 'ERR_NO_CREDENTIAL':
      return new PasskeyPrfException('noCredential', message);
    case 'ERR_CONFIGURATION':
      return new PasskeyPrfException('configuration', message);
    case 'ERR_CREDENTIAL_ALREADY_EXISTS':
      return new PasskeyPrfException('credentialAlreadyExists', message);
    case 'ERR_AUTHENTICATION_FAILED':
      return new PasskeyPrfException('authenticationFailed', message);
    case 'ERR_PRF_EVALUATION_FAILED':
      return new PasskeyPrfException('prfEvaluationFailed', message);
    default:
      return new PasskeyPrfException('unknown', message);
  }
}

/**
 * Built-in React Native passkey PRF provider using platform-native APIs.
 *
 * Implements the PrfProvider interface using:
 * - iOS: AuthenticationServices framework (iOS 18+)
 * - Android: Credential Manager API (Android 14+)
 *
 * On first use, if no credential exists for the RP ID, a new passkey is
 * automatically created (registered), then the assertion is retried.
 *
 * Requirements:
 * - iOS 18.0+ or Android 14+ (API 34)
 * - Associated Domains entitlement (iOS) or assetlinks.json (Android) for the RP domain
 *
 * @example
 * ```typescript
 * import { PasskeyClient } from '@breeztech/breez-sdk-spark-react-native'
 * import { PasskeyProvider } from '@breeztech/breez-sdk-spark-react-native/passkey-prf-provider'
 *
 * const prfProvider = new PasskeyProvider()
 * const passkey = new PasskeyClient(prfProvider as any, undefined)
 * const response = await passkey.signIn({ label: 'personal', extraSalts: [] })
 * ```
 */
export class PasskeyProvider {
  /**
   * Constant identifying Breez's shared `keys.breez.technology` RP.
   * Pass as `rpId` when opting into the Breez-managed Relying Party
   * (only valid for apps registered with Breez). Apps with their own
   * RP domain pass their own string.
   */
  static readonly BREEZ_RP_ID: string = 'keys.breez.technology';

  private rpId: string;
  private rpName: string;
  private userName: string;
  private userDisplayName: string;
  private credentialRegistry: CredentialRegistry | undefined;
  private onRegistryError: ((op: RegistryOperation, err: Error) => void) | undefined;

  constructor(options: PasskeyProviderOptions) {
    if (!options?.rpId || options.rpId.length === 0) {
      throw new Error(
        "PasskeyProvider: rpId is required. Pass your app's RP domain, " +
          'or PasskeyProvider.BREEZ_RP_ID if you registered with Breez.'
      );
    }
    if (!options.rpName || options.rpName.length === 0) {
      throw new Error(
        'PasskeyProvider: rpName is required. Pass your app name; it is ' +
          'shown to the user in the OS passkey picker.'
      );
    }
    this.rpId = options.rpId;
    this.rpName = options.rpName;
    this.userName = options.userName ?? this.rpName;
    this.userDisplayName = options.userDisplayName ?? this.userName;
    this.credentialRegistry = options.credentialRegistry;
    if (this.credentialRegistry) {
      for (const method of ['read', 'add', 'remove', 'clear'] as const) {
        if (typeof this.credentialRegistry[method] !== 'function') {
          throw new Error(
            `PasskeyProvider: credentialRegistry is missing a "${method}" ` +
              'method. Implementations must provide read / add / remove / clear.'
          );
        }
      }
    }
    this.onRegistryError = options.onRegistryError;
  }

  /**
   * Derive multiple 32-byte seeds from passkey PRF with the given salts in
   * as few OS ceremonies as the platform supports (dual-salt assertion
   * when available). The `salts.length === 1` case short-circuits to a
   * single-salt assertion under the hood (one prompt). Used by the
   * SDK's `setup_wallet` orchestration to collapse master + label
   * derivation into one prompt.
   *
   * Accepts the SDK's `DeriveSeedsRequest` shape. Per-call
   * `allowCredentials` (a list of credential IDs the assertion is
   * restricted to, primary use case: reauthentication) overrides the
   * constructor default when non-empty;
   * `preferImmediatelyAvailableCredentials` overrides the platform
   * default when non-null.
   */
  async deriveSeeds(request: {
    salts: string[];
    allowCredentials?: Uint8Array[];
    preferImmediatelyAvailableCredentials?: boolean | null;
  }): Promise<Uint8Array[]> {
    if (!BreezSdkSparkPasskey) {
      throw passkeyModuleUnavailableError('deriveSeeds');
    }

    let effectiveAllow = request.allowCredentials ?? [];
    // Auto-merge registry IDs into the allow-list. JS-side dance:
    // the native module never sees the registry.
    if (this.credentialRegistry) {
      const registryIds = await _registryReadBestEffort(
        this.credentialRegistry,
        this.rpId,
        this.onRegistryError
      );
      if (registryIds.length > 0) {
        const seen = new Set(effectiveAllow.map(uint8ArrayToBase64));
        const merged: Uint8Array[] = [...effectiveAllow];
        for (const id of registryIds) {
          const key = uint8ArrayToBase64(id);
          if (!seen.has(key)) {
            seen.add(key);
            merged.push(id);
          }
        }
        effectiveAllow = merged;
      }
    }
    const allowBase64 = effectiveAllow.map(id => uint8ArrayToBase64(id));
    const preferImmediate = request.preferImmediatelyAvailableCredentials ?? null;

    let base64Results: string[];
    try {
      base64Results = await BreezSdkSparkPasskey.deriveSeeds(
        request.salts,
        this.rpId,
        this.rpName,
        this.userName,
        this.userDisplayName,
        false,
        allowBase64,
        preferImmediate
      );
      if (!Array.isArray(base64Results)) {
        throw new PasskeyPrfException('unknown', 'deriveSeeds returned a non-array');
      }
    } catch (err) {
      if (err instanceof PasskeyPrfException) {
        throw err;
      }
      const mapped = mapNativeError(err);
      // Augment CredentialNotFound when host had no allow-list and no
      // registry so integrators can discover the opt-in path.
      if (
        mapped.code === 'noCredential' &&
        effectiveAllow.length === 0 &&
        !this.credentialRegistry
      ) {
        throw new PasskeyPrfException(
          mapped.code,
          mapped.message + CREDENTIAL_REGISTRY_HELP_SUFFIX
        );
      }
      throw mapped;
    }

    return base64Results.map(b64 => base64ToUint8Array(b64));
  }

  /**
   * Create a new passkey with PRF support.
   *
   * Only registers the credential, no seed derivation. Triggers
   * exactly 1 platform prompt. Use this to separate credential
   * creation from derivation in multi-step onboarding flows.
   *
   * `excludeCredentials` is a list of already-registered credential
   * IDs. Prevents registering the same device twice: when any entry
   * matches a credential already on the device, the platform raises
   * `CredentialAlreadyExists`. Branding fields (`userName`,
   * `userDisplayName`) live on the constructor. `user.id` is never
   * host-supplied: the native plugin mints a fresh random 16-byte
   * handle per call and returns it via the result's `userId` field.
   *
   * @returns Credential ID, the generated user handle, plus AAGUID and
   *   backup-eligibility parsed from the attestation object. AAGUID and
   *   `backupEligible` are null when the attestation can't be parsed.
   * @throws If the user cancels or PRF is not supported by the authenticator.
   */
  async createPasskey(excludeCredentials: Uint8Array[] = []): Promise<RegisteredCredential> {
    if (!BreezSdkSparkPasskey) {
      throw passkeyModuleUnavailableError('createPasskey');
    }

    let excludeIds = excludeCredentials;
    // Merge registry IDs into the exclude list before the native call.
    if (this.credentialRegistry) {
      const registryIds = await _registryReadBestEffort(
        this.credentialRegistry,
        this.rpId,
        this.onRegistryError
      );
      if (registryIds.length > 0) {
        const seen = new Set(excludeIds.map(uint8ArrayToBase64));
        const merged: Uint8Array[] = [...excludeIds];
        for (const id of registryIds) {
          const key = uint8ArrayToBase64(id);
          if (!seen.has(key)) {
            seen.add(key);
            merged.push(id);
          }
        }
        excludeIds = merged;
      }
    }
    const excludeBase64 = excludeIds.map(id => uint8ArrayToBase64(id));

    try {
      const result: {
        credentialId: string;
        userId: string;
        aaguid: string | null;
        backupEligible: boolean | null;
      } = await BreezSdkSparkPasskey.createPasskey(
        this.rpId,
        this.rpName,
        this.userName,
        this.userDisplayName,
        excludeBase64
      );

      const credentialId = base64ToUint8Array(result.credentialId);
      const userId = base64ToUint8Array(result.userId);
      // Persist new credential ID to the registry post-success.
      if (this.credentialRegistry) {
        _registryAddFireAndForget(
          this.credentialRegistry,
          this.rpId,
          credentialId,
          this.onRegistryError
        );
      }
      return {
        credentialId,
        userId,
        aaguid: result.aaguid ? base64ToUint8Array(result.aaguid) : null,
        backupEligible: result.backupEligible,
      };
    } catch (err) {
      throw mapNativeError(err);
    }
  }

  /**
   * Return the credential IDs the configured {@link CredentialRegistry}
   * has stored for the current `rpId`. Empty list when no registry is
   * configured. Backs `PasskeyClient.credentials().get()`.
   */
  async getKnownCredentialIds(): Promise<Uint8Array[]> {
    if (!this.credentialRegistry) {
      return [];
    }
    return _registryReadBestEffort(
      this.credentialRegistry,
      this.rpId,
      this.onRegistryError
    );
  }

  /**
   * Drop a single credential ID from the configured registry. No-op
   * when no registry is configured. Backs
   * `PasskeyClient.credentials().remove(id)`.
   */
  async removeKnownCredentialId(credentialId: Uint8Array): Promise<void> {
    if (!this.credentialRegistry) {
      return;
    }
    try {
      const result = await _withRegistryTimeout(
        this.credentialRegistry.remove(this.rpId, credentialId)
      );
      if (result === _REGISTRY_TIMEOUT) {
        const err = new Error('CredentialRegistry.remove timed out');
        console.warn('[CredentialRegistry] remove timed out');
        this.onRegistryError?.('remove', err);
      }
    } catch (err) {
      console.warn('[CredentialRegistry] remove failed', err);
      this.onRegistryError?.('remove', err as Error);
    }
  }

  /**
   * Clear the configured registry's persisted credential-ID list for
   * the current `rpId`. No-op when no registry is configured. Backs
   * `PasskeyClient.credentials().clear()`.
   */
  async clearKnownCredentialIds(): Promise<void> {
    if (!this.credentialRegistry) {
      return;
    }
    try {
      const result = await _withRegistryTimeout(
        this.credentialRegistry.clear(this.rpId)
      );
      if (result === _REGISTRY_TIMEOUT) {
        const err = new Error('CredentialRegistry.clear timed out');
        console.warn('[CredentialRegistry] clear timed out');
        this.onRegistryError?.('clear', err);
      }
    } catch (err) {
      console.warn('[CredentialRegistry] clear failed', err);
      this.onRegistryError?.('clear', err as Error);
    }
  }

  /**
   * Check if a PRF-capable passkey is available on this device.
   *
   * @returns true if the platform supports passkeys with PRF extension.
   */
  async isSupported(): Promise<boolean> {
    if (!BreezSdkSparkPasskey) {
      return false;
    }

    return await BreezSdkSparkPasskey.isSupported();
  }

  /**
   * Verify the configured `rpId` is a valid scope for WebAuthn from
   * the running app's identity. Returns a typed {@link DomainAssociation}.
   * On Android the native plugin degrades a `NotAssociated` result to
   * `Skipped` because Credential Manager runs its own check internally
   * with a fresher GMS-cached statement set.
   */
  async checkDomainAssociation(): Promise<DomainAssociation> {
    if (!BreezSdkSparkPasskey) {
      return {
        kind: 'Skipped',
        reason: 'Native passkey module unavailable on this platform',
      };
    }
    try {
      const result = await BreezSdkSparkPasskey.checkDomainAssociation(this.rpId);
      const kind = result?.kind;
      if (kind === 'Associated') {
        return { kind: 'Associated' };
      }
      if (kind === 'NotAssociated') {
        return {
          kind: 'NotAssociated',
          source: result?.source ?? 'unknown',
          reason: result?.reason ?? '',
        };
      }
      return { kind: 'Skipped', reason: result?.reason ?? '' };
    } catch (err) {
      const anyErr = err as { message?: string };
      return {
        kind: 'Skipped',
        reason: anyErr?.message ?? 'Domain association probe failed',
      };
    }
  }
}

/**
 * Decode a base64 string to Uint8Array.
 */
function base64ToUint8Array(base64: string): Uint8Array {
  const binaryString = atob(base64);
  const bytes = new Uint8Array(binaryString.length);
  for (let i = 0; i < binaryString.length; i++) {
    bytes[i] = binaryString.charCodeAt(i);
  }
  return bytes;
}

/**
 * Encode a Uint8Array to base64 string.
 */
function uint8ArrayToBase64(bytes: Uint8Array): string {
  let binary = '';
  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }
  return btoa(binary);
}

