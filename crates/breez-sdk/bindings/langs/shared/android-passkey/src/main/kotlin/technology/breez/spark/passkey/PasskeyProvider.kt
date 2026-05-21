package technology.breez.spark.passkey

import android.app.Activity
import breez_sdk_spark.DeriveSeedsRequest
import breez_sdk_spark.DomainAssociation
import breez_sdk_spark.PrfProvider
import breez_sdk_spark.PrfProviderException
import breez_sdk_spark.RegisteredCredential
import technology.breez.spark.passkey.core.CreatePasskeyOptions
import technology.breez.spark.passkey.core.CredentialManagerPrfCore
import technology.breez.spark.passkey.core.CredentialManagerPrfCoreException
import technology.breez.spark.passkey.core.CredentialRegistry
import technology.breez.spark.passkey.core.DeriveSeedsOptions
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
 *   changes.
 * @param rpId Relying Party ID. Must match the domain configured for
 *   cross-platform credential sharing.
 * @param rpName Display name shown to the user in the OS passkey
 *   picker and credential-management UIs (iCloud Keychain, Google
 *   Password Manager, 1Password, etc.) when choosing a credential.
 *   Only used at credential registration; changing it does not affect
 *   existing credentials.
 * @param userName User name stored with the credential. Defaults to [rpName].
 * @param userDisplayName User display name shown in the passkey picker.
 *   Defaults to [userName] (or [rpName] if [userName] is null).
 * @param credentialRegistry Opt-in app-side store of known credential
 *   IDs. When supplied, the SDK auto-merges stored IDs into
 *   `allowCredentialIds` / `excludeCredentialIds` and writes new IDs
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
    }

    private val userName: String = userName ?: rpName
    private val userDisplayName: String = userDisplayName ?: (userName ?: rpName)

    /** Slot used to surface the credential ID asserted in the most
     *  recent ceremony. Read once, cleared on read. Used by the
     *  binding-layer `SignInResponse.credential_id` plumbing.
     */
    @Volatile
    private var lastObservedCredentialId: ByteArray? = null

    /** Take ownership of the credential ID captured by the most
     *  recent assertion, clearing the slot. Returns `null` if no
     *  assertion has completed since the last call.
     */
    override suspend fun takeLastObservedCredentialId(): ByteArray? {
        val v = lastObservedCredentialId
        lastObservedCredentialId = null
        return v
    }

    /**
     * Bulk PRF derivation backed by [CredentialManagerPrfCore.deriveSeedsOrRegister].
     *
     * Uses the WebAuthn PRF dual-salt fast path on authenticators that
     * honor `prf.eval.second` (Google Password Manager on recent
     * versions). Falls back to sequential single-salt assertions on
     * authenticators that silently drop the second salt; the verdict
     * is cached per process so the first failed attempt is not paid
     * twice.
     *
     * Output ordering matches input ordering.
     */
    override suspend fun deriveSeeds(request: DeriveSeedsRequest): List<ByteArray> {
        try {
            val options = DeriveSeedsOptions(
                allowCredentialIds = request.allowCredentialIds,
                preferImmediatelyAvailableCredentials = request.preferImmediatelyAvailableCredentials,
                credentialRegistry = credentialRegistry,
                onRegistryError = onRegistryError,
            )
            return CredentialManagerPrfCore.deriveSeedsOrRegister(
                activity = activityProvider(),
                salts = request.salts,
                rpId = rpId,
                rpName = rpName,
                userName = userName,
                userDisplayName = userDisplayName,
                autoRegister = false,
                captureCredentialId = { id -> lastObservedCredentialId = id },
                options = options,
            )
        } catch (e: CredentialManagerPrfCoreException) {
            throw e.toPrfProviderException()
        }
    }

    override suspend fun isSupported(): Boolean =
        CredentialManagerPrfCore.isSupported()

    override suspend fun checkDomainAssociation(): DomainAssociation {
        val activity = activityProvider()
        val result = CredentialManagerPrfCore.checkDomainAssociation(
            context = activity.applicationContext,
            rpId = rpId,
        )
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
     * `excludeCredentialIds` and the new credential ID is auto-added
     * on success.
     */
    override suspend fun createPasskey(excludeCredentialIds: List<ByteArray>): RegisteredCredential {
        try {
            val core = CredentialManagerPrfCore.createCredential(
                activity = activityProvider(),
                rpId = rpId,
                rpName = rpName,
                userName = userName,
                userDisplayName = userDisplayName,
                excludeCredentialIds = excludeCredentialIds,
                options = CreatePasskeyOptions(
                    credentialRegistry = credentialRegistry,
                    onRegistryError = onRegistryError,
                ),
            )
            return RegisteredCredential(core.credentialId, core.userId, core.aaguid, core.backupEligible)
        } catch (e: CredentialManagerPrfCoreException) {
            throw e.toPrfProviderException()
        }
    }

    override suspend fun getKnownCredentialIds(): List<ByteArray> {
        val reg = credentialRegistry ?: return emptyList()
        return try {
            reg.read(rpId)
        } catch (t: Throwable) {
            onRegistryError?.invoke(RegistryOperation.Read, t)
            emptyList()
        }
    }

    override suspend fun removeKnownCredentialId(id: ByteArray) {
        val reg = credentialRegistry ?: return
        try {
            reg.remove(rpId, id)
        } catch (t: Throwable) {
            onRegistryError?.invoke(RegistryOperation.Remove, t)
        }
    }

    override suspend fun clearKnownCredentialIds() {
        val reg = credentialRegistry ?: return
        try {
            reg.clear(rpId)
        } catch (t: Throwable) {
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
