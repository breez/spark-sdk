/**
 * Capacitor native-bridge contract for the Breez SDK passkey provider.
 *
 * This is a TypeScript-only sub-export with no runtime. Capacitor
 * plugin authors should import the types and use them when wiring up
 * `registerPlugin<PasskeyPrfPlugin>('PasskeyPrf')`, so the JS-side
 * `definitions.ts` and the native iOS Swift / Android Kotlin shapes
 * stay in lockstep with the SDK.
 *
 * Usage in a Capacitor plugin's `definitions.ts`:
 *
 * ```ts
 * import type {
 *   PasskeyPrfPlugin,
 *   DomainAssociation,
 * } from '@breeztech/breez-sdk-spark/passkey-capacitor-bridge';
 *
 * export type { PasskeyPrfPlugin, DomainAssociation };
 * ```
 *
 * Then in the plugin entry:
 *
 * ```ts
 * import { registerPlugin } from '@capacitor/core';
 * import type { PasskeyPrfPlugin } from './definitions';
 *
 * export const PasskeyPrf =
 *   registerPlugin<PasskeyPrfPlugin>('PasskeyPrf');
 * ```
 *
 * The shape mirrors the canonical iOS `PasskeyAssertionCore.swift` and
 * Android `CredentialManagerPrfCore.kt` plugin surface bundled with
 * the SDK. Keep this contract narrow on purpose: it is the boundary
 * between the JS layer (which works with base64 strings, since
 * Capacitor's bridge serialises everything as JSON) and the native
 * layer (which uses raw bytes internally).
 */

/**
 * Result of a domain-association verification check against the
 * platform's out-of-band domain verification source (iOS AASA CDN,
 * Android Digital Asset Links). Mirrors the Rust `DomainAssociation`
 * enum so cross-platform callers handle it uniformly.
 *
 * - `Associated`: the platform confirmed the app is bound to the RP.
 *   Safe to proceed with WebAuthn calls.
 * - `NotAssociated`: the platform reports a configuration mismatch.
 *   Surface a dedicated error state. WebAuthn calls will fail for
 *   configuration reasons, not a UX-recoverable state.
 * - `Skipped`: verification could not be performed (offline, endpoint
 *   timeout, test context). Proceed with WebAuthn as normal. This is
 *   not a negative signal.
 */
export type DomainAssociation =
    | { kind: 'Associated' }
    | { kind: 'NotAssociated'; source: string; reason: string }
    | { kind: 'Skipped'; reason: string };

/**
 * The contract every Capacitor passkey-PRF plugin implements. The JS
 * side type-checks against this interface; the iOS Swift / Android
 * Kotlin native side must produce values that JSON-deserialise into
 * each return shape.
 *
 * All `Uint8Array`-shaped values are exchanged as base64-url-safe
 * strings without padding, since Capacitor cannot transport binary
 * data through its bridge directly. Encode/decode at the boundary.
 */
export interface PasskeyPrfPlugin {
    /**
     * Check whether native passkey PRF is available on this device.
     *
     * iOS: requires iOS 18.0+ (PRF entered the platform authenticator
     * in iOS 18). Android: requires API 28+ and a credential provider
     * that implements the PRF extension.
     *
     * Should never throw. Plugin authors may surface initialisation
     * failures via the boolean.
     */
    isSupported(): Promise<{ available: boolean }>;

    /**
     * Register a new passkey with PRF support. Triggers exactly one
     * biometric / passkey prompt.
     *
     * @returns the credential ID plus optional authenticator metadata.
     *   `aaguid` (16-byte provider identifier) and `backupEligible`
     *   (BE flag) are `null` when the platform doesn't surface enough
     *   authenticator data to extract them. AAGUID is unverified
     *   attestation: use as a display hint only, never for trust
     *   decisions.
     */
    createPasskey(options: {
        rpId?: string;
        rpName?: string;
        userName?: string;
        userDisplayName?: string;
        excludeCredentialIds?: string[];
    }): Promise<{
        credentialId: string;
        aaguid: string | null;
        backupEligible: boolean | null;
    }>;

    /**
     * Derive a 32-byte seed from passkey PRF for the given salt. If
     * `autoRegister` is true and no credential is available, the
     * plugin auto-registers one and then derives. Triggers one or
     * two biometric prompts depending on the path.
     *
     * @param allowCredentialIds optional base64-encoded credential IDs
     *   to constrain the assertion. When non-empty, the OS picker
     *   shows only these credentials (single-row auto-pick on iOS via
     *   `preferImmediatelyAvailableCredentials`). When empty / omitted,
     *   sign-in is fully discoverable.
     *
     * @returns the base64-encoded 32-byte seed plus the base64-encoded
     *   credential ID that was actually used. `credentialId` may be
     *   `null` when the platform did not surface it (rare; e.g. the
     *   call resolved through an auto-register path that completed
     *   before the assertion delegate fired).
     */
    deriveSeed(options: {
        rpId?: string;
        salt: string;
        autoRegister?: boolean;
        allowCredentialIds?: string[];
    }): Promise<{ seed: string; credentialId: string | null }>;

    /**
     * Bulk derive: collapse multiple PRF salts into as few biometric
     * ceremonies as the authenticator supports (1 ceremony per
     * dual-salt pair on iOS / Android, sequential elsewhere). Output
     * ordering matches input.
     *
     * The bulk call uses ONE credential for all salts (single
     * assertion), so the returned `credentialId` covers every entry
     * in `seeds`.
     */
    deriveSeeds(options: {
        rpId?: string;
        salts: string[];
        autoRegister?: boolean;
        allowCredentialIds?: string[];
    }): Promise<{ seeds: string[]; credentialId: string | null }>;

    /**
     * Verify the app's identity against the platform's out-of-band
     * domain verification source (iOS AASA CDN, Android Digital
     * Asset Links).
     *
     * Intended to be called once per session, before the first
     * WebAuthn ceremony. Gate onboarding / discovery UX on the
     * result. Never throws: verification failures surface as
     * `Skipped`, misconfigurations as `NotAssociated`.
     */
    checkDomainAssociation(options?: {
        rpId?: string;
    }): Promise<DomainAssociation>;

    /**
     * Read the persisted list of base64-encoded credential IDs for
     * `rpId`. Backed by the platform's synced keychain (iCloud
     * Keychain on iOS, Block Store on Android), so the list survives
     * app uninstall + reinstall. Used by hosts to populate
     * `excludeCredentialIds` on `createPasskey` without depending on
     * `localStorage` (which is wiped on uninstall).
     *
     * Returns an empty list when the store is missing, invalid, or
     * the RP has never registered a credential on this device.
     */
    getKnownCredentialIds(options?: {
        rpId?: string;
    }): Promise<{ credentialIds: string[] }>;

    /**
     * Drop a single credential ID from the persisted list for `rpId`.
     * Used by the switch-failure recovery path so a credential the
     * user has manually deleted from Settings stops appearing in the
     * management list while the rest of the user's known credentials
     * remain tracked.
     *
     * No-op when the credential is not in the store.
     */
    removeKnownCredentialId(options: {
        credentialId: string;
        rpId?: string;
    }): Promise<void>;

    /**
     * Clear the persisted credential-ID list for `rpId`. Called by
     * the deletion-recovery flow when a sign-in attempt returns
     * `CREDENTIAL_NOT_FOUND` for a device that has previously
     * registered: the user has manually deleted the passkey, so the
     * stale list is no longer meaningful.
     */
    clearKnownCredentialIds(options?: {
        rpId?: string;
    }): Promise<void>;
}
