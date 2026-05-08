import { NativeModules, Platform } from 'react-native';

const { BreezSdkSparkPasskey } = NativeModules;

/**
 * Build a diagnostic error thrown when the native passkey module isn't
 * reachable from JS. The most common cause on iOS is running on a version
 * older than iOS 18 — the Swift class is marked `@available(iOS 18.0, *)`
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
 */
export interface RegisteredCredential {
  credentialId: Uint8Array;
  aaguid: Uint8Array | null;
  backupEligible: boolean | null;
}

/**
 * Per-call overrides for `createPasskey`. All fields are optional;
 * omitted fields fall back to the constructor values (random 16-byte
 * `userId`, ctor `userName`, ctor `userDisplayName`).
 */
export interface CreatePasskeyRequest {
  /**
   * Credential IDs the authenticator must refuse to duplicate. When
   * any entry matches a credential already on the device, the platform
   * raises an "already exists" error which surfaces as a typed
   * `PasskeyPrfException` with code `credentialAlreadyExists`. Defaults
   * to none.
   */
  excludeCredentialIds?: Uint8Array[];

  /**
   * Override for the WebAuthn `user.id` field. Must be 1-64 bytes per
   * WebAuthn spec. Defaults to a fresh random 16-byte value chosen by
   * the native plugin per call. Always randomize per call: a hardcoded
   * value across a fresh-install + create flow can silently destroy
   * the user's prior credential and the data tied to it on consumer
   * authenticators.
   */
  userId?: Uint8Array;

  /** Override for the WebAuthn `user.name` field. */
  userName?: string;

  /** Override for the WebAuthn `user.displayName` field. */
  userDisplayName?: string;
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
 * Options for constructing a PasskeyProvider.
 */
export interface PasskeyProviderOptions {
  /**
   * Relying Party ID. Must match the domain configured for cross-platform
   * credential sharing.
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

  /**
   * When true, `deriveSeeds` automatically creates a new passkey if
   * none exists for this RP ID, then retries the assertion. When false
   * (default), throws an error instead, letting the caller control
   * registration separately via `createPasskey()`.
   * @default false
   */
  autoRegister?: boolean;

  /**
   * Restrict assertion (sign-in) to one of these credential IDs. The
   * platform refuses any other credential for this RP. When null or
   * empty, the platform picks any credential matching the RP. Critical
   * for deterministic seed derivation when multiple credentials might
   * exist for the same RP.
   */
  allowCredentialIds?: Uint8Array[];
}

/**
 * Error thrown by [PasskeyProvider] when a passkey operation fails.
 * Provides a structured `code` for programmatic handling.
 *
 * `code` values: `userCancelled`, `prfNotSupported`, `noCredential`,
 * `configuration`, `credentialAlreadyExists`, `unknown`.
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
 * import { Passkey } from '@breeztech/breez-sdk-spark-react-native'
 * import { PasskeyProvider } from '@breeztech/breez-sdk-spark-react-native/passkey-prf-provider'
 *
 * const prfProvider = new PasskeyProvider()
 * const passkey = new Passkey(prfProvider, undefined)
 * const wallet = await passkey.getWallet('personal')
 * ```
 */
export class PasskeyProvider {
  private rpId: string;
  private rpName: string;
  private userName: string;
  private userDisplayName: string;
  private autoRegister: boolean;
  private allowCredentialIds: Uint8Array[];

  constructor(options?: PasskeyProviderOptions) {
    this.rpId = options?.rpId ?? 'keys.breez.technology';
    this.rpName = options?.rpName ?? 'Breez SDK';
    this.userName = options?.userName ?? this.rpName;
    this.userDisplayName = options?.userDisplayName ?? this.userName;
    // Default to false (matching every other binding's behaviour
    // post-5f.1). When unset, deriveSeeds throws CredentialNotFound
    // on missing creds; the host then drives registration explicitly
    // via createPasskey.
    this.autoRegister = options?.autoRegister ?? false;
    this.allowCredentialIds = options?.allowCredentialIds ?? [];
  }

  /**
   * Derive multiple 32-byte seeds from passkey PRF with the given salts in
   * as few OS ceremonies as the platform supports (dual-salt assertion
   * when available). The `salts.length === 1` case short-circuits to a
   * single-salt assertion under the hood (one prompt). Used by the
   * SDK's `setup_wallet` orchestration to collapse master + label
   * derivation into one prompt.
   *
   * @param salts - Plain UTF-8 salt strings; the native side encodes each as
   *   bytes for the PRF eval inputs.
   */
  async deriveSeeds(salts: string[]): Promise<Uint8Array[]> {
    if (!BreezSdkSparkPasskey) {
      throw passkeyModuleUnavailableError('deriveSeeds');
    }

    const allowBase64 = this.allowCredentialIds.map(id => uint8ArrayToBase64(id));

    try {
      const base64Results: string[] = await BreezSdkSparkPasskey.deriveSeeds(
        salts,
        this.rpId,
        this.rpName,
        this.userName,
        this.userDisplayName,
        this.autoRegister,
        allowBase64
      );
      if (!Array.isArray(base64Results)) {
        throw new PasskeyPrfException('unknown', 'deriveSeeds returned a non-array');
      }
      return base64Results.map(b64 => base64ToUint8Array(b64));
    } catch (err) {
      if (err instanceof PasskeyPrfException) {
        throw err;
      }
      throw mapNativeError(err);
    }
  }

  /**
   * Create a new passkey with PRF support.
   *
   * Only registers the credential, no seed derivation. Triggers exactly
   * 1 platform prompt. Use this to separate credential creation from
   * derivation in multi-step onboarding flows.
   *
   * Per-call overrides on `request` (excludeCredentialIds, userId,
   * userName, userDisplayName) fall back to the constructor values when
   * omitted.
   *
   * @returns Credential ID plus AAGUID and backup-eligibility parsed from
   *   the attestation object. AAGUID and `backupEligible` are null when
   *   the attestation can't be parsed.
   * @throws If the user cancels or PRF is not supported by the authenticator.
   */
  async createPasskey(request?: CreatePasskeyRequest): Promise<RegisteredCredential> {
    if (!BreezSdkSparkPasskey) {
      throw passkeyModuleUnavailableError('createPasskey');
    }

    const req = request ?? {};
    const excludeBase64 = (req.excludeCredentialIds ?? []).map(id => uint8ArrayToBase64(id));
    const userIdBase64 = req.userId ? uint8ArrayToBase64(req.userId) : null;

    try {
      const result: {
        credentialId: string;
        aaguid: string | null;
        backupEligible: boolean | null;
      } = await BreezSdkSparkPasskey.createPasskey(
        this.rpId,
        this.rpName,
        req.userName ?? this.userName,
        req.userDisplayName ?? this.userDisplayName,
        excludeBase64,
        userIdBase64
      );

      return {
        credentialId: base64ToUint8Array(result.credentialId),
        aaguid: result.aaguid ? base64ToUint8Array(result.aaguid) : null,
        backupEligible: result.backupEligible,
      };
    } catch (err) {
      throw mapNativeError(err);
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

