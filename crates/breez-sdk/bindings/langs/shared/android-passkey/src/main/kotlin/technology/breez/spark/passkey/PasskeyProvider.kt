package technology.breez.spark.passkey

import android.app.Activity
import breez_sdk_spark.CreatePasskeyRequest
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
 * On first use, if no credential exists for the Relying Party, a new passkey
 * is automatically created (registered), then the assertion is retried.
 *
 * ## Requirements
 *
 * - Android 9+ (API 28) with Google Play Services, or Android 14+ (API 34)
 *   with any compatible Credential Manager provider.
 * - A valid `/.well-known/assetlinks.json` for the RP domain.
 * - A physical device: emulators cannot complete the WebAuthn registration
 *   handshake.
 *
 * ## Example
 *
 * ```kotlin
 * val prfProvider = PasskeyProvider(
 *     activityProvider = { MainActivity.currentInstance!! },
 * )
 * val passkey = PasskeyClient(prfProvider, relayConfig = null)
 * val response = passkey.signIn(SignInRequest(label = "personal"))
 * ```
 *
 * @param activityProvider Called lazily on every PRF / registration request
 *   to obtain the current top Activity. Using a lambda (rather than a direct
 *   Activity reference) avoids holding a stale instance across configuration
 *   changes.
 * @param rpId Relying Party ID. Must match the domain configured for
 *   cross-platform credential sharing. Changing this after users have
 *   registered passkeys will make their existing credentials undiscoverable.
 * @param rpName Display name for the RP, shown during credential registration.
 *   Only used when creating new passkeys.
 * @param userName User name stored with the credential. Defaults to [rpName].
 *   Only used during registration.
 * @param userDisplayName User display name shown in the passkey picker.
 *   Defaults to [userName] (or [rpName] if [userName] is null). Only used
 *   during registration.
 * @param autoRegister When `true`, [deriveSeed] automatically creates
 *   a new passkey if none exists, then retries the assertion. When
 *   `false` (default), throws [PrfProviderException.CredentialNotFound]
 *   and the caller drives registration via [createPasskey].
 * @param allowCredentialIds When non-empty, restricts assertion (sign-in)
 *   to one of the listed credential IDs. The platform refuses any other
 *   credential for this RP. Use this to bind sign-in to a specific
 *   passkey the caller has registered, instead of letting the platform
 *   pick any sibling credential that happens to share the RP. Critical
 *   for deterministic seed derivation when multiple credentials might
 *   exist for the same RP. When empty (default), the platform picks any
 *   credential matching the RP.
 */
public class PasskeyProvider(
    private val activityProvider: () -> Activity,
    private val rpId: String = CredentialManagerPrfCore.DEFAULT_RP_ID,
    private val rpName: String = CredentialManagerPrfCore.DEFAULT_RP_NAME,
    userName: String? = null,
    userDisplayName: String? = null,
    private val autoRegister: Boolean = false,
    private val allowCredentialIds: List<ByteArray> = emptyList(),
    private val credentialRegistry: CredentialRegistry? = null,
    private val onRegistryError: ((RegistryOperation, Throwable) -> Unit)? = null,
) : PrfProvider {

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
    public fun takeLastObservedCredentialId(): ByteArray? {
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
            // Per-call overrides win over per-instance defaults; an
            // empty per-call list defers to the constructor's
            // `allowCredentialIds`.
            val perCallAllow = request.allowCredentialIds
            val effectiveAllow = if (perCallAllow.isEmpty()) allowCredentialIds else perCallAllow
            val options = DeriveSeedsOptions(
                allowCredentialIds = effectiveAllow,
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
                autoRegister = autoRegister,
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
            // can't be used. iOS has the opposite property —
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
     * derivation. Per-call overrides on `request` (userId, userName,
     * userDisplayName) fall back to constructor values when omitted.
     *
     * When the provider was constructed with a `credentialRegistry`,
     * the registry's stored IDs are auto-merged into
     * `request.excludeCredentialIds` and the new credential ID is
     * auto-added on success.
     */
    override suspend fun createPasskey(request: CreatePasskeyRequest): RegisteredCredential {
        try {
            val core = CredentialManagerPrfCore.createCredential(
                activity = activityProvider(),
                rpId = rpId,
                rpName = rpName,
                userName = request.userName ?: userName,
                userDisplayName = request.userDisplayName ?: userDisplayName,
                excludeCredentialIds = request.excludeCredentialIds,
                userIdOverride = request.userId,
                options = CreatePasskeyOptions(
                    credentialRegistry = credentialRegistry,
                    onRegistryError = onRegistryError,
                ),
            )
            return RegisteredCredential(core.credentialId, core.aaguid, core.backupEligible)
        } catch (e: CredentialManagerPrfCoreException) {
            throw e.toPrfProviderException()
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
                PrfProviderException.CredentialNotFound()
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
