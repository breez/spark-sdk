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
   * When true (default), `derivePrfSeed` automatically creates a new passkey
   * if none exists for this RP ID, then retries the assertion. When false,
   * throws an error instead, letting the caller control registration
   * separately via `createPasskey()`.
   * @default true
   */
  autoRegister?: boolean;
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

  constructor(options?: PasskeyProviderOptions) {
    this.rpId = options?.rpId ?? 'keys.breez.technology';
    this.rpName = options?.rpName ?? 'Breez SDK';
    this.userName = options?.userName ?? this.rpName;
    this.userDisplayName = options?.userDisplayName ?? this.userName;
    this.autoRegister = options?.autoRegister !== false;
  }

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
  async derivePrfSeed(salt: string): Promise<Uint8Array> {
    if (!BreezSdkSparkPasskey) {
      throw passkeyModuleUnavailableError('derivePrfSeed');
    }

    const base64Result: string = await BreezSdkSparkPasskey.derivePrfSeed(
      salt,
      this.rpId,
      this.rpName,
      this.userName,
      this.userDisplayName,
      this.autoRegister
    );

    return base64ToUint8Array(base64Result);
  }

  /**
   * Create a new passkey with PRF support.
   *
   * Only registers the credential — no seed derivation. Triggers exactly
   * 1 platform prompt. Use this to separate credential creation from
   * derivation in multi-step onboarding flows.
   *
   * @param excludeCredentialIds - Optional list of credential IDs to exclude.
   *   Pass previously created credential IDs to prevent the authenticator
   *   from creating a duplicate on the same device.
   * @returns The credential ID of the newly created passkey.
   * @throws If the user cancels or PRF is not supported by the authenticator.
   */
  async createPasskey(excludeCredentialIds?: Uint8Array[]): Promise<Uint8Array> {
    if (!BreezSdkSparkPasskey) {
      throw passkeyModuleUnavailableError('createPasskey');
    }

    const excludeBase64 = (excludeCredentialIds ?? []).map(id => uint8ArrayToBase64(id));

    const base64Result: string = await BreezSdkSparkPasskey.createPasskey(
      this.rpId,
      this.rpName,
      this.userName,
      this.userDisplayName,
      excludeBase64
    );

    return base64ToUint8Array(base64Result);
  }

  /**
   * Check if a PRF-capable passkey is available on this device.
   *
   * @returns true if the platform supports passkeys with PRF extension.
   */
  async isPrfAvailable(): Promise<boolean> {
    if (!BreezSdkSparkPasskey) {
      return false;
    }

    return await BreezSdkSparkPasskey.isPrfAvailable();
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
  for (let i = 0; i < bytes.length; i++) {
    binary += String.fromCharCode(bytes[i]);
  }
  return btoa(binary);
}

/**
 * @deprecated Use PasskeyProviderOptions instead. This alias will be removed in a future release.
 */
export type PasskeyPrfProviderOptions = PasskeyProviderOptions;

/**
 * @deprecated Use PasskeyProvider instead. This alias will be removed in a future release.
 */
export { PasskeyProvider as PasskeyPrfProvider };
