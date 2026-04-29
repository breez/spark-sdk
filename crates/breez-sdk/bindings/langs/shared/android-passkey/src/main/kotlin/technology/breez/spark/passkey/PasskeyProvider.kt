package technology.breez.spark.passkey

import android.app.Activity
import breez_sdk_spark.DomainAssociation
import breez_sdk_spark.PasskeyPrfException
import breez_sdk_spark.PrfProvider
import technology.breez.spark.passkey.core.CredentialManagerPrfCore
import technology.breez.spark.passkey.core.CredentialManagerPrfCoreException
import technology.breez.spark.passkey.core.DomainAssociationResult

/**
 * Built-in [PrfProvider] that uses the AndroidX Credential Manager +
 * WebAuthn PRF extension to derive deterministic 32-byte seeds from platform
 * passkeys. A thin wrapper around [CredentialManagerPrfCore] that adapts the
 * core's exceptions into the UniFFI-generated [PasskeyPrfException] variants.
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
 * val passkey = Passkey(prfProvider, relayConfig = null)
 * val wallet = passkey.getWallet("personal")
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
 * @param autoRegister When `true` (default), [derivePrfSeed] automatically
 *   creates a new passkey if none exists for this RP ID, then retries the
 *   assertion. When `false`, [derivePrfSeed] throws
 *   [PasskeyPrfException.CredentialNotFound] instead, letting the caller
 *   control registration separately via [createPasskey].
 */
public class PasskeyProvider(
    private val activityProvider: () -> Activity,
    private val rpId: String = CredentialManagerPrfCore.DEFAULT_RP_ID,
    private val rpName: String = CredentialManagerPrfCore.DEFAULT_RP_NAME,
    userName: String? = null,
    userDisplayName: String? = null,
    private val autoRegister: Boolean = true,
) : PrfProvider {

    private val userName: String = userName ?: rpName
    private val userDisplayName: String = userDisplayName ?: (userName ?: rpName)

    override suspend fun derivePrfSeed(salt: String): ByteArray {
        try {
            return CredentialManagerPrfCore.deriveSeedOrRegister(
                activity = activityProvider(),
                salt = salt,
                rpId = rpId,
                rpName = rpName,
                userName = userName,
                userDisplayName = userDisplayName,
                autoRegister = autoRegister,
            )
        } catch (e: CredentialManagerPrfCoreException) {
            throw e.toPasskeyPrfException()
        }
    }

    override suspend fun isPrfAvailable(): Boolean =
        CredentialManagerPrfCore.isPrfAvailable()

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
     * Register a new passkey without deriving a seed.
     *
     * Triggers exactly one platform prompt. Use this to separate credential
     * creation from derivation in multi-step onboarding flows.
     *
     * @param excludeCredentialIds Optional list of credential IDs to exclude.
     *   Pass previously created credential IDs to prevent the authenticator
     *   from creating a duplicate on the same device.
     * @return The credential ID of the newly created passkey.
     */
    public suspend fun createPasskey(excludeCredentialIds: List<ByteArray> = emptyList()): ByteArray {
        try {
            return CredentialManagerPrfCore.createCredential(
                activity = activityProvider(),
                rpId = rpId,
                rpName = rpName,
                userName = userName,
                userDisplayName = userDisplayName,
                excludeCredentialIds = excludeCredentialIds,
            )
        } catch (e: CredentialManagerPrfCoreException) {
            throw e.toPasskeyPrfException()
        }
    }

    private fun CredentialManagerPrfCoreException.toPasskeyPrfException(): PasskeyPrfException =
        when (kind) {
            CredentialManagerPrfCore.Kind.PrfNotSupported ->
                PasskeyPrfException.PrfNotSupported()
            CredentialManagerPrfCore.Kind.UserCancelled ->
                PasskeyPrfException.UserCancelled()
            CredentialManagerPrfCore.Kind.CredentialNotFound ->
                PasskeyPrfException.CredentialNotFound()
            CredentialManagerPrfCore.Kind.AuthenticationFailed ->
                PasskeyPrfException.AuthenticationFailed(message ?: "")
            CredentialManagerPrfCore.Kind.PrfEvaluationFailed ->
                PasskeyPrfException.PrfEvaluationFailed(message ?: "")
            CredentialManagerPrfCore.Kind.Configuration ->
                PasskeyPrfException.Configuration(message ?: "")
            CredentialManagerPrfCore.Kind.Generic ->
                PasskeyPrfException.Generic(message ?: "")
        }
}
