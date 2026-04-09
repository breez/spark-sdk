package technology.breez.spark.passkey

import android.app.Activity
import android.os.Build
import android.util.Base64
import androidx.credentials.CreatePublicKeyCredentialRequest
import androidx.credentials.CredentialManager
import androidx.credentials.GetCredentialRequest
import androidx.credentials.GetPublicKeyCredentialOption
import androidx.credentials.exceptions.CreateCredentialCancellationException
import androidx.credentials.exceptions.CreateCredentialException
import androidx.credentials.exceptions.GetCredentialCancellationException
import androidx.credentials.exceptions.GetCredentialException
import androidx.credentials.exceptions.NoCredentialException
import breez_sdk_spark.PasskeyPrfException
import breez_sdk_spark.PasskeyPrfProvider
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.security.SecureRandom

/**
 * Built-in [PasskeyPrfProvider] that uses the AndroidX Credential Manager and
 * the WebAuthn PRF extension to derive deterministic 32-byte seeds from
 * platform passkeys.
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
 * ## Thread safety
 *
 * All Credential Manager calls must run on the main thread; this class
 * switches to `Dispatchers.Main` internally, so callers may invoke it from
 * any coroutine context.
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
 * @param activityProvider Called lazily on every PRF / registration request to
 *   obtain the current top Activity. Using a lambda (rather than a direct
 *   Activity reference) avoids holding a stale instance across configuration
 *   changes.
 * @param rpId Relying Party ID. Must match the domain configured for
 *   cross-platform credential sharing. Changing this after users have
 *   registered passkeys will make their existing credentials undiscoverable.
 * @param rpName Display name for the RP, shown during credential registration.
 *   Only used when creating new passkeys; changing it does not affect existing
 *   credentials.
 * @param userName User name stored with the credential. Defaults to [rpName].
 *   Only used during registration.
 * @param userDisplayName User display name shown in the passkey picker.
 *   Defaults to [userName] (or [rpName] if [userName] is null). Only used
 *   during registration.
 */
public class CredentialManagerPrfProvider(
    private val activityProvider: () -> Activity,
    private val rpId: String = DEFAULT_RP_ID,
    private val rpName: String = DEFAULT_RP_NAME,
    userName: String? = null,
    userDisplayName: String? = null,
) : PasskeyPrfProvider {

    private val userName: String = userName ?: rpName
    private val userDisplayName: String = userDisplayName ?: (userName ?: rpName)

    override suspend fun derivePrfSeed(salt: String): ByteArray = withContext(Dispatchers.Main) {
        val activity = activityProvider()
        try {
            try {
                getAssertionWithPrf(activity, salt)
            } catch (e: NoCredentialException) {
                // No credential found — register a new one and retry.
                registerCredential(activity)
                getAssertionWithPrf(activity, salt)
            }
        } catch (e: PasskeyPrfException) {
            throw e
        } catch (e: Exception) {
            throw e.toPasskeyPrfException()
        }
    }

    override suspend fun isPrfAvailable(): Boolean =
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.P

    /**
     * Register a new passkey without deriving a seed.
     *
     * Triggers exactly one platform prompt. Use this to separate credential
     * creation from derivation in multi-step onboarding flows.
     *
     * @throws PasskeyPrfException if the user cancels or the authenticator
     *   does not support the PRF extension.
     */
    public suspend fun createPasskey(): Unit = withContext(Dispatchers.Main) {
        try {
            registerCredential(activityProvider())
        } catch (e: PasskeyPrfException) {
            throw e
        } catch (e: Exception) {
            throw e.toPasskeyPrfException()
        }
    }

    // ------------------------------------------------------------------
    // Private
    // ------------------------------------------------------------------

    private suspend fun getAssertionWithPrf(activity: Activity, salt: String): ByteArray {
        val credentialManager = CredentialManager.create(activity)

        val saltBytes = salt.toByteArray(Charsets.UTF_8)
        val saltBase64 = Base64.encodeToString(
            saltBytes,
            Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING,
        )
        val challenge = randomBase64Url(32)

        val requestJson = """
        {
            "challenge": "$challenge",
            "rpId": "$rpId",
            "allowCredentials": [],
            "userVerification": "required",
            "extensions": {
                "prf": {
                    "eval": {
                        "first": "$saltBase64"
                    }
                }
            }
        }
        """.trimIndent()

        val option = GetPublicKeyCredentialOption(requestJson)
        val request = GetCredentialRequest(listOf(option))
        val response = credentialManager.getCredential(activity, request)

        val credential = response.credential
        val authResponseJson = credential.data.getString(
            "androidx.credentials.BUNDLE_KEY_AUTHENTICATION_RESPONSE_JSON",
        ) ?: throw PasskeyPrfException.AuthenticationFailed("No credential response")

        val responseJson = JSONObject(authResponseJson)
        val extensions = responseJson.optJSONObject("clientExtensionResults")
            ?: throw PasskeyPrfException.PrfNotSupported()
        val prf = extensions.optJSONObject("prf")
            ?: throw PasskeyPrfException.PrfNotSupported()
        val results = prf.optJSONObject("results")
            ?: throw PasskeyPrfException.PrfEvaluationFailed("no results")
        val first = results.optString("first")
        if (first.isNullOrEmpty()) {
            throw PasskeyPrfException.PrfEvaluationFailed("empty result")
        }

        return Base64.decode(first, Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING)
    }

    private suspend fun registerCredential(activity: Activity) {
        val credentialManager = CredentialManager.create(activity)

        val challenge = randomBase64Url(32)
        val userId = randomBase64Url(16)

        val requestJson = """
        {
            "challenge": "$challenge",
            "rp": {
                "id": "$rpId",
                "name": "$rpName"
            },
            "user": {
                "id": "$userId",
                "name": "$userName",
                "displayName": "$userDisplayName"
            },
            "pubKeyCredParams": [
                {"type": "public-key", "alg": -7},
                {"type": "public-key", "alg": -257}
            ],
            "authenticatorSelection": {
                "residentKey": "required",
                "requireResidentKey": true,
                "userVerification": "required"
            },
            "extensions": {
                "prf": {}
            }
        }
        """.trimIndent()

        val request = CreatePublicKeyCredentialRequest(requestJson)
        credentialManager.createCredential(activity, request)
    }

    private fun randomBase64Url(byteCount: Int): String {
        val bytes = ByteArray(byteCount)
        SecureRandom().nextBytes(bytes)
        return Base64.encodeToString(
            bytes,
            Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING,
        )
    }

    private fun Exception.toPasskeyPrfException(): PasskeyPrfException = when (this) {
        is GetCredentialCancellationException,
        is CreateCredentialCancellationException -> PasskeyPrfException.UserCancelled()

        is NoCredentialException -> PasskeyPrfException.CredentialNotFound()

        is GetCredentialException ->
            PasskeyPrfException.AuthenticationFailed("${type}: ${message ?: toString()}")

        is CreateCredentialException ->
            PasskeyPrfException.AuthenticationFailed("${type}: ${message ?: toString()}")

        else -> {
            val raw = message ?: toString()
            // Surface actionable hints for common issues (mirrors the Flutter
            // and React Native Android plugin behavior).
            val hint = when {
                raw.contains("cannot be validated", ignoreCase = true) ->
                    "Domain verification failed. Passkeys require a physical device with " +
                        "Google Play Services and a valid /.well-known/assetlinks.json for the " +
                        "RP domain. Emulators are not supported."
                raw.contains("not supported", ignoreCase = true) ->
                    "Passkeys require Android 9+ with Google Play Services, or Android 14+ " +
                        "with a compatible Credential Manager provider."
                else -> raw
            }
            PasskeyPrfException.Generic(hint)
        }
    }

    public companion object {
        /**
         * Default Relying Party ID used for cross-platform credential sharing
         * across Breez SDK clients.
         */
        public const val DEFAULT_RP_ID: String = "keys.breez.technology"

        /**
         * Default Relying Party display name shown during passkey registration.
         */
        public const val DEFAULT_RP_NAME: String = "Breez SDK"
    }
}
