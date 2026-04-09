import { NativeModules, Platform } from 'react-native';

const { BreezSdkSparkPasskey } = NativeModules;

/**
 * Options for constructing a PasskeyPrfProvider.
 */
export interface PasskeyPrfProviderOptions {
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
}

/**
 * React Native passkey PRF provider using platform-native APIs.
 *
 * Implements the PasskeyPrfProvider interface using:
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
 * import { PasskeyPrfProvider, Passkey } from '@breeztech/breez-sdk-spark-react-native'
 *
 * const prfProvider = new PasskeyPrfProvider()
 * const passkey = new Passkey(prfProvider, undefined)
 * const wallet = await passkey.getWallet('personal')
 * ```
 */
export class PasskeyPrfProvider {
  private rpId: string;
  private rpName: string;
  private userName: string;
  private userDisplayName: string;

  constructor(options?: PasskeyPrfProviderOptions) {
    this.rpId = options?.rpId ?? 'keys.breez.technology';
    this.rpName = options?.rpName ?? 'Breez SDK';
    this.userName = options?.userName ?? this.rpName;
    this.userDisplayName = options?.userDisplayName ?? this.userName;
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
      throw new Error(
        'BreezSdkSparkPasskey native module not found. ' +
        'Ensure the react-native package is properly linked.'
      );
    }

    const base64Result: string = await BreezSdkSparkPasskey.derivePrfSeed(
      salt,
      this.rpId,
      this.rpName,
      this.userName,
      this.userDisplayName
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
   * @throws If the user cancels or PRF is not supported by the authenticator.
   */
  async createPasskey(): Promise<void> {
    if (!BreezSdkSparkPasskey) {
      throw new Error(
        'BreezSdkSparkPasskey native module not found. ' +
        'Ensure the react-native package is properly linked.'
      );
    }

    await BreezSdkSparkPasskey.createPasskey(
      this.rpId,
      this.rpName,
      this.userName,
      this.userDisplayName
    );
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
