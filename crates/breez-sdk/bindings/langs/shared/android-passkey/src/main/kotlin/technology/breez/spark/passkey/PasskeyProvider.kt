package technology.breez.spark.passkey

import android.app.Activity
import android.util.Log
import breez_sdk_spark.DeriveSeedsRequest
import breez_sdk_spark.DomainAssociation
import breez_sdk_spark.PrfProvider
import breez_sdk_spark.PrfProviderException
import breez_sdk_spark.RegisteredCredential
import technology.breez.spark.passkey.core.CredentialManagerPrfCore
import technology.breez.spark.passkey.core.CredentialManagerPrfCoreException
import technology.breez.spark.passkey.core.CredentialRegistry
import technology.breez.spark.passkey.core.DomainAssociationResult
import technology.breez.spark.passkey.core.RegistryOperation

/**
 * Built-in [PrfProvider] that uses the AndroidX Credential Manager +
 * WebAuthn PRF extension to derive deterministic 32-byte seeds from platform
 * passkeys. A thin wrapper around [CredentialManagerPrfCore] that adapts the
 * core's exceptions into the UniFFI-generated [PrfProviderException] variants.
 *
 * ## Requirements
 *
 * - Android 9+ (API 28) with Google Play Services, or Android 14+ (API 34)
 *   with any compatible Credential Manager provider.
 * - A valid `/.well-known/assetlinks.json` for the RP domain.
 * - A physical device: emulators cannot complete the WebAuthn registration
 *   handshake.
 *
 * @param activityProvider Called lazily on every request to obtain the
 *   current top Activity. Using a lambda (rather than a direct Activity
 *   reference) avoids holding a stale instance across configuration
 *   changes. Credential Manager is lifecycle-sensitive, so the lambda
 *   MUST return the foreground, RESUMED, non-finishing Activity at call
 *   time (it shows system UI on top of it) and MUST NOT return a
 *   destroyed or cached background instance. Returning the wrong
 *   Activity surfaces as opaque Credential Manager failures.
 * @param rpId Relying Party ID. Must match the domain configured for
 *   cross-platform credential sharing.
 * @param rpName Maps to the WebAuthn `rp.name`. Deprecated in
 *   WebAuthn L3 but still required by current Credential Manager
 *   prompts. Surfaces in some credential-management UIs (Google
 *   Password Manager, 1Password); platform UIs increasingly ignore
 *   it. Only used at credential registration; changing it does not
 *   affect existing credentials.
 * @param userName Maps to the WebAuthn `user.name`. Treated as the
 *   user's unique identifier for the credential and shown in the
 *   account picker during sign-in. Pass a stable per-user value if
 *   each registration should surface as a distinct entry. Defaults
 *   to [rpName]. Only used at registration; changing it does not
 *   affect existing credentials.
 * @param userDisplayName Maps to the WebAuthn `user.displayName`.
 *   The user-friendly label the OS / browser MAY (but is not
 *   required to) show in the picker; behavior varies by Credential
 *   Manager backend. Defaults to [userName] (or [rpName] if
 *   [userName] is null). Only used at registration; changing it
 *   does not affect existing credentials.
 * @param credentialRegistry Opt-in app-side store of known credential
 *   IDs. When supplied, the SDK auto-merges stored IDs into
 *   `allowCredentials` / `excludeCredentials` and writes new IDs
 *   back after success.
 * @param onRegistryError Best-effort callback for registry failures;
 *   never blocks the ceremony.
 */
public class PasskeyProvider(
    private val activityProvider: () -> Activity,
    private val rpId: String,
    private val rpName: String,
    userName: String? = null,
    userDisplayName: String? = null,
    private val credentialRegistry: CredentialRegistry? = null,
    private val onRegistryError: ((RegistryOperation, Throwable) -> Unit)? = null,
) : PrfProvider {

    public companion object {
        /**
         * Constant identifying Breez's shared `keys.breez.technology` RP.
         * Pass as `rpId` when opting into the Breez-managed Relying Party
         * (only valid for apps registered with Breez). Apps with their
         * own RP domain pass their own string.
         */
        public const val BREEZ_RP_ID: String = CredentialManagerPrfCore.DEFAULT_RP_ID

        private const val TAG = "PasskeyProvider"
    }

    private val resolvedUserName: String = userName ?: rpName
    private val resolvedUserDisplayName: String = userDisplayName ?: resolvedUserName

    /** The configured PRF engine; holds rp identity + registry. */
    private val core = CredentialManagerPrfCore(
        rpId = rpId,
        rpName = rpName,
        userName = resolvedUserName,
        userDisplayName = resolvedUserDisplayName,
        credentialRegistry = credentialRegistry,
        onRegistryError = onRegistryError,
        activityProvider = activityProvider,
    )

    /**
     * Bulk PRF derivation backed by [CredentialManagerPrfCore.deriveSeeds].
     * Uses the WebAuthn PRF dual-salt fast path where the authenticator
     * honors `prf.eval.second`, falling back to single-salt otherwise.
     * Output ordering matches input ordering. Returns the seeds plus the
     * credential ID observed in the same assertion (null when none was
     * captured, e.g. empty `salts`).
     *
     * Passes `autoRegister = false`: this provider never implicitly
     * creates a credential during derivation. Sign-up and sign-in are
     * explicit (the host calls [createPasskey] for registration), so a
     * missing credential surfaces as `CredentialNotFound` rather than
     * silently minting a new passkey. (The core defaults `autoRegister`
     * to true for direct callers; the provider opts out.)
     */
    override suspend fun deriveSeeds(request: DeriveSeedsRequest): DeriveSeedsOutput =
        try {
            val derivation = core.deriveSeeds(
                salts = request.salts,
                autoRegister = false,
                allowCredentials = request.allowCredentials,
                preferImmediatelyAvailableCredentials =
                    request.preferImmediatelyAvailableCredentials ?: true,
            )
            // The core observes the asserted credential ID inline and
            // returns it alongside the seeds.
            DeriveSeedsOutput(derivation.seeds, derivation.credentialId)
        } catch (e: CredentialManagerPrfCoreException) {
            throw e.toPrfProviderException()
        }

    override suspend fun isSupported(): Boolean =
        CredentialManagerPrfCore.isSupported()

    override suspend fun checkDomainAssociation(): DomainAssociation {
        val result = core.checkDomainAssociation()
        return when (result) {
            is DomainAssociationResult.Associated ->
                DomainAssociation.Associated

            // Soft-fail on Android: degrade NotAssociated → Skipped.
            //
            // Rationale: Android's CredentialManager performs its own DAL
            // verification internally, using Google Play Services' cache
            // of the assetlinks statements. That cache is typically
            // fresher than the public Digital Asset Links API our Core
            // queries. If our public-API probe reports "no match" while
            // GMS's cache would actually accept the credential, blocking
            // the user on our check is a strict regression vs. the
            // native path.
            //
            // Android's native error surface is also granular enough
            // (NoCredentialException, GetCredentialProviderConfiguration
            // Exception, etc.) that the subsequent CredentialManager call
            // produces a recognizable error when the credential truly
            // can't be used. iOS has the opposite property : 
            // ASAuthorizationError collapses AASA failures into
            // `CredentialNotFound`, so iOS keeps NotAssociated as a
            // hard-block (that's the whole point of the pre-check there).
            //
            // Logging at WARN so maintainers can still see the DAL
            // mismatch in logcat (or in-app log export) without users
            // being blocked.
            is DomainAssociationResult.NotAssociated -> {
                android.util.Log.w(
                    "CredentialManagerPrf",
                    "Digital Asset Links reports mismatch; degrading to Skipped. " +
                        "source=${result.source} reason=${result.reason}"
                )
                DomainAssociation.Skipped(
                    reason = "[soft-fail on Android] ${result.reason}"
                )
            }

            is DomainAssociationResult.Skipped ->
                DomainAssociation.Skipped(reason = result.reason)
        }
    }

    /**
     * Register a new passkey with PRF support. One ceremony, no seed
     * derivation.
     *
     * `user.id` is never host-supplied: the core mints a fresh random
     * 16-byte handle per call and surfaces it via
     * [RegisteredCredential.userId]. Branding fields (`userName`,
     * `userDisplayName`) live on this provider's constructor.
     *
     * When the provider was constructed with a `credentialRegistry`,
     * the registry's stored IDs are auto-merged into
     * `excludeCredentials` and the new credential ID is auto-added
     * on success.
     */
    override suspend fun createPasskey(excludeCredentials: List<ByteArray>): RegisteredCredential {
        try {
            val c = core.register(excludeCredentials)
            return RegisteredCredential(c.credentialId, c.userId, c.aaguid, c.backupEligible)
        } catch (e: CredentialManagerPrfCoreException) {
            throw e.toPrfProviderException()
        }
    }

    override suspend fun getKnownCredentialIds(): List<ByteArray> {
        val reg = credentialRegistry ?: return emptyList()
        return try {
            reg.read(rpId)
        } catch (t: Throwable) {
            Log.w(TAG, "CredentialRegistry.read failed", t)
            onRegistryError?.invoke(RegistryOperation.Read, t)
            emptyList()
        }
    }

    override suspend fun removeKnownCredentialId(id: ByteArray) {
        val reg = credentialRegistry ?: return
        try {
            reg.remove(rpId, id)
        } catch (t: Throwable) {
            Log.w(TAG, "CredentialRegistry.remove failed", t)
            onRegistryError?.invoke(RegistryOperation.Remove, t)
        }
    }

    override suspend fun clearKnownCredentialIds() {
        val reg = credentialRegistry ?: return
        try {
            reg.clear(rpId)
        } catch (t: Throwable) {
            Log.w(TAG, "CredentialRegistry.clear failed", t)
            onRegistryError?.invoke(RegistryOperation.Clear, t)
        }
    }

    private fun CredentialManagerPrfCoreException.toPrfProviderException(): PrfProviderException =
        when (kind) {
            CredentialManagerPrfCore.Kind.PrfNotSupported ->
                PrfProviderException.PrfNotSupported()
            CredentialManagerPrfCore.Kind.UserCancelled ->
                PrfProviderException.UserCancelled()
            CredentialManagerPrfCore.Kind.UserTimedOut ->
                PrfProviderException.UserTimedOut()
            CredentialManagerPrfCore.Kind.CredentialNotFound ->
                PrfProviderException.CredentialNotFound(message ?: "Credential not found")
            CredentialManagerPrfCore.Kind.AuthenticationFailed ->
                PrfProviderException.AuthenticationFailed(message ?: "")
            CredentialManagerPrfCore.Kind.PrfEvaluationFailed ->
                PrfProviderException.PrfEvaluationFailed(message ?: "")
            CredentialManagerPrfCore.Kind.Configuration ->
                PrfProviderException.Configuration(message ?: "")
            CredentialManagerPrfCore.Kind.CredentialAlreadyExists ->
                PrfProviderException.CredentialAlreadyExists(message ?: "")
            CredentialManagerPrfCore.Kind.Generic ->
                PrfProviderException.Generic(message ?: "")
        }
}
