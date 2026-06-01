import { NativeModules, Platform } from 'react-native';
import {
  PasskeyClient as SdkPasskeyClient,
  type PasskeyConfig,
  type PrfProvider,
} from './generated/breez_sdk_spark';

const { BreezSdkSparkPasskey } = NativeModules;

/**
 * Diagnostic error for when the native passkey module isn't reachable. The
 * common iOS cause is running below iOS 18, where the `@available(iOS 18.0, *)`
 * Swift class cannot load; on Android a missing module means broken linking.
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
 * A passkey credential from a register or sign-in ceremony. `credentialId`
 * is always set; the attestation fields are populated on registration and
 * absent on sign-in (an assertion carries no attestation). Persist
 * `credentialId` to drive `excludeCredentials` / `allowCredentials` on later
 * calls. Treat `aaguid` as an unverified display hint, never a trust decision.
 * `userId` is the user handle minted by the native plugin (never host-supplied).
 */
export interface PasskeyCredential {
  credentialId: Uint8Array;
  userId: Uint8Array | null;
  aaguid: Uint8Array | null;
  backupEligible: boolean | null;
}

/**
 * Result of {@link PasskeyProvider.checkDomainAssociation}. Switch on `kind`
 * to handle each outcome.
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
   * Relying Party ID: the domain configured for credential sharing.
   * Changing it after users register passkeys makes their existing
   * credentials undiscoverable. Pass {@link PasskeyProvider.BREEZ_RP_ID} for
   * the Breez-managed RP (only valid for Breez-registered apps).
   */
  rpId: string;

  /**
   * Display name shown in the OS passkey picker and credential-manager
   * UIs. Only used at registration; changing it does not affect existing
   * credentials.
   */
  rpName: string;

  /**
   * Per-credential identifier shown in the account picker during sign-in.
   * Pass a stable per-user value to surface each registration distinctly
   * (Apple's Passwords app dedupes by `(rpId, userName)`). Defaults to
   * `rpName`. Only used at registration.
   */
  userName?: string;

  /**
   * User-friendly label some platforms show in the picker. Defaults to
   * `userName`. Only used at registration.
   */
  userDisplayName?: string;
}

/**
 * Error thrown when a passkey operation fails, with a structured `code` for
 * programmatic handling: `userCancelled`, `userTimedOut`, `prfNotSupported`,
 * `noCredential`, `configuration`, `credentialAlreadyExists`, `unknown`.
 * `userTimedOut` is the OS biometric inactivity timeout (distinct from the
 * user dismissing the prompt), so hosts may safely auto-retry it.
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
 * Built-in React Native passkey PRF provider, backed by AuthenticationServices
 * on iOS and Credential Manager on Android. The default {@link PrfProvider};
 * inject a configured instance through {@link PasskeyClientBuilder}. Requires
 * iOS 18+ or Android 14+ (API 34) plus the Associated Domains entitlement
 * (iOS) or assetlinks.json (Android) for the RP domain.
 */
export class PasskeyProvider {
  /**
   * Breez's shared `keys.breez.technology` RP. Pass as `rpId` to opt in
   * (only valid for apps registered with Breez); apps with their own RP
   * domain pass their own string.
   */
  static readonly BREEZ_RP_ID: string = 'keys.breez.technology';

  /** Default `rpName` for the zero-config client when none is supplied. */
  static readonly DEFAULT_RP_NAME: string = 'Breez';

  private rpId: string;
  private rpName: string;
  private userName: string;
  private userDisplayName: string;

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
  }

  /**
   * Derive one 32-byte seed per salt from passkey PRF, in as few OS prompts
   * as the platform supports. `allowCredentials` restricts the assertion to
   * specific credential IDs (mainly for reauthentication) when non-empty;
   * `preferImmediatelyAvailableCredentials` overrides the platform default
   * when set. Returns the seeds plus the asserted credential ID.
   */
  async deriveSeeds(request: {
    salts: string[];
    allowCredentials?: Uint8Array[];
    preferImmediatelyAvailableCredentials?: boolean | null;
  }): Promise<{ seeds: Uint8Array[]; credentialId?: Uint8Array }> {
    if (!BreezSdkSparkPasskey) {
      throw passkeyModuleUnavailableError('deriveSeeds');
    }

    const allowBase64 = (request.allowCredentials ?? []).map(id =>
      uint8ArrayToBase64(id)
    );
    const preferImmediate = request.preferImmediatelyAvailableCredentials ?? null;

    let result: { seeds: string[]; credentialId?: string | null };
    try {
      result = await BreezSdkSparkPasskey.deriveSeeds(
        request.salts,
        this.rpId,
        this.rpName,
        this.userName,
        this.userDisplayName,
        false,
        allowBase64,
        preferImmediate
      );
      if (!result || !Array.isArray(result.seeds)) {
        throw new PasskeyPrfException('unknown', 'deriveSeeds returned an unexpected shape');
      }
    } catch (err) {
      if (err instanceof PasskeyPrfException) {
        throw err;
      }
      throw mapNativeError(err);
    }

    // The native module returns the asserted credential ID alongside the
    // seeds; surface it so the SDK can pin the next derive to this exact
    // credential.
    return {
      seeds: result.seeds.map(b64 => base64ToUint8Array(b64)),
      credentialId: result.credentialId
        ? base64ToUint8Array(result.credentialId)
        : undefined,
    };
  }

  /**
   * Register a new PRF-capable passkey (one prompt, no seed derivation): use
   * it to split credential creation from derivation in multi-step onboarding.
   * `excludeCredentials` blocks re-registering a device that already holds a
   * credential, surfaced as a `credentialAlreadyExists` failure. The returned
   * user handle is minted fresh per call (never host-supplied).
   */
  async createPasskey(excludeCredentials: Uint8Array[] = []): Promise<PasskeyCredential> {
    if (!BreezSdkSparkPasskey) {
      throw passkeyModuleUnavailableError('createPasskey');
    }

    const excludeBase64 = excludeCredentials.map(id => uint8ArrayToBase64(id));

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

      return {
        credentialId: base64ToUint8Array(result.credentialId),
        userId: base64ToUint8Array(result.userId),
        aaguid: result.aaguid ? base64ToUint8Array(result.aaguid) : null,
        backupEligible: result.backupEligible,
      };
    } catch (err) {
      throw mapNativeError(err);
    }
  }

  /** Whether this device supports passkeys with the PRF extension. */
  async isSupported(): Promise<boolean> {
    if (!BreezSdkSparkPasskey) {
      return false;
    }

    return await BreezSdkSparkPasskey.isSupported();
  }

  /**
   * Verify the app is associated with the configured `rpId` for WebAuthn.
   * Android always returns `Skipped` rather than `NotAssociated`: Credential
   * Manager runs its own check internally against fresher data.
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

/** Decode a base64 string to Uint8Array. */
function base64ToUint8Array(base64: string): Uint8Array {
  const binaryString = atob(base64);
  const bytes = new Uint8Array(binaryString.length);
  for (let i = 0; i < binaryString.length; i++) {
    bytes[i] = binaryString.charCodeAt(i);
  }
  return bytes;
}

/** Encode a Uint8Array to base64 string. */
function uint8ArrayToBase64(bytes: Uint8Array): string {
  let binary = '';
  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }
  return btoa(binary);
}

/**
 * Builds a `PasskeyClient` backed by a caller-supplied provider. Use this
 * when you need a configured {@link PasskeyProvider} (custom `rpId` /
 * `rpName`) or a custom PRF backend; omit the provider for the zero-config
 * Breez-RP default.
 */
export class PasskeyClientBuilder {
  private provider?: PrfProvider;

  /**
   * @param breezApiKey Breez relay key for authenticated (NIP-42) label
   *   storage. Omit for public relays only.
   * @param config Passkey client config. `rpId` / `rpName` configure the
   *   default provider (ignored when a provider is injected via
   *   {@link withPrfProvider}, which owns its RP); `defaultLabel` is the
   *   label-store default.
   */
  constructor(
    private readonly breezApiKey?: string,
    private readonly config?: PasskeyConfig
  ) {}

  /**
   * Inject the provider the client derives seeds through: the built-in
   * {@link PasskeyProvider} or any custom `PrfProvider` implementation.
   * Supersedes the config's `rpId` / `rpName` (the injected provider owns
   * its RP).
   */
  withPrfProvider(provider: PrfProvider): this {
    this.provider = provider;
    return this;
  }

  /**
   * Construct the client. Falls back to a default {@link PasskeyProvider}
   * on the config's `rpId` / `rpName` (default: the Breez RP) when no
   * provider was injected.
   */
  build(): SdkPasskeyClient {
    // The hand-written PasskeyProvider conforms structurally to the
    // generated PrfProvider foreign interface.
    const provider: PrfProvider =
      this.provider ??
      (new PasskeyProvider({
        rpId: this.config?.rpId ?? PasskeyProvider.BREEZ_RP_ID,
        rpName: this.config?.rpName ?? PasskeyProvider.DEFAULT_RP_NAME,
      }) as unknown as PrfProvider);
    return new SdkPasskeyClient(provider, this.breezApiKey, this.config);
  }
}

/** @internal Builds the zero-config client; exposed via {@link PasskeyClient}. */
function buildPasskeyClient(
  breezApiKey?: string,
  config?: PasskeyConfig
): SdkPasskeyClient {
  return new PasskeyClientBuilder(breezApiKey, config).build();
}

/**
 * Zero-config passkey client on the Breez shared RP (`keys.breez.technology`),
 * so a Breez-registered app needs only its relay key; set `rpId` / `rpName` on
 * the config to use your own RP. For a custom PRF backend, build the provider
 * and inject it via {@link PasskeyClientBuilder}.
 */
export const PasskeyClient: {
  new (breezApiKey?: string, config?: PasskeyConfig): SdkPasskeyClient;
} = buildPasskeyClient as unknown as {
  new (breezApiKey?: string, config?: PasskeyConfig): SdkPasskeyClient;
};

