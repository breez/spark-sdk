package technology.breez.spark.passkey

import android.app.Activity
import breez_sdk_spark.PasskeyPrfException
import breez_sdk_spark.PasskeyPrfProvider
import technology.breez.spark.passkey.core.CredentialManagerPrfCore
import technology.breez.spark.passkey.core.CredentialManagerPrfCoreException

/**
 * Built-in [PasskeyPrfProvider] that uses the AndroidX Credential Manager +
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
 * val prfProvider = CredentialManagerPrfProvider(
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
 */
public class CredentialManagerPrfProvider(
    private val activityProvider: () -> Activity,
    private val rpId: String = CredentialManagerPrfCore.DEFAULT_RP_ID,
    private val rpName: String = CredentialManagerPrfCore.DEFAULT_RP_NAME,
    userName: String? = null,
    userDisplayName: String? = null,
) : PasskeyPrfProvider {

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
            )
        } catch (e: CredentialManagerPrfCoreException) {
            throw e.toPasskeyPrfException()
        }
    }

    override suspend fun isPrfAvailable(): Boolean =
        CredentialManagerPrfCore.isPrfAvailable()

    /**
     * Register a new passkey without deriving a seed.
     *
     * Triggers exactly one platform prompt. Use this to separate credential
     * creation from derivation in multi-step onboarding flows.
     */
    public suspend fun createPasskey() {
        try {
            CredentialManagerPrfCore.createCredential(
                activity = activityProvider(),
                rpId = rpId,
                rpName = rpName,
                userName = userName,
                userDisplayName = userDisplayName,
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
            CredentialManagerPrfCore.Kind.Generic ->
                PasskeyPrfException.Generic(message ?: "")
        }
}
