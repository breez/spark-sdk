package technology.breez.spark.passkey.core

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
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.security.SecureRandom

// =====================================================================
// !!! SOURCE-OF-TRUTH NOTICE !!!
//
// The canonical copy of this file lives at:
//   crates/breez-sdk/bindings/langs/shared/android-passkey/src/main/kotlin/
//     technology/breez/spark/passkey/core/CredentialManagerPrfCore.kt
//
// It is shared into four Android artifacts via two mechanisms:
//   1. gradle `srcDirs` — bindings-android + breez-sdk-spark-kmp
//   2. `cargo xtask sync-passkey-core` — packages/flutter + packages/react-native
//
// Never hand-edit the Flutter or React Native copies. Edit the canonical
// file, then run `cargo xtask sync-passkey-core` and commit the diff.
// CI will fail if a copy drifts from the canonical.
// =====================================================================

/**
 * Framework-agnostic helper that wraps the AndroidX Credential Manager +
 * WebAuthn PRF extension machinery for passkey-based seed derivation.
 *
 * Wrappers (the UniFFI `PasskeyPrfProvider`, the Flutter MethodChannel
 * plugin, the React Native native module) delegate to this object and
 * only provide framework-specific glue on top: error mapping, activity
 * retrieval, and call-site boilerplate.
 *
 * Throws [CredentialManagerPrfCoreException] for every well-known failure
 * mode so wrappers can switch on [Kind] without peeking at WebAuthn or
 * Credential Manager internals.
 */
public object CredentialManagerPrfCore {

    /** Default Relying Party ID for cross-platform credential sharing. */
    public const val DEFAULT_RP_ID: String = "keys.breez.technology"

    /** Default Relying Party display name shown during passkey registration. */
    public const val DEFAULT_RP_NAME: String = "Breez SDK"

    /**
     * Returns `true` if this device's OS version could support passkey PRF.
     *
     * PRF extension support requires API 28+ (Android 9+) via Google Play
     * Services. The Jetpack Credential Manager library handles backward
     * compatibility automatically from there. This check does NOT verify
     * that a credential provider is actually installed or that biometrics
     * are enrolled — only the platform version.
     */
    public fun isPrfAvailable(): Boolean =
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.P

    /**
     * Authenticate the user via passkey PRF and return the 32-byte seed.
     *
     * If no credential exists for [rpId], auto-register a new one and
     * retry the assertion. Switches to `Dispatchers.Main` internally so
     * callers may invoke from any coroutine context.
     *
     * @throws CredentialManagerPrfCoreException for every handled error;
     *   wrappers should catch and remap by [CredentialManagerPrfCoreException.kind].
     */
    public suspend fun deriveSeedOrRegister(
        activity: Activity,
        salt: String,
        rpId: String,
        rpName: String,
        userName: String,
        userDisplayName: String,
    ): ByteArray = withContext(Dispatchers.Main) {
        try {
            try {
                getAssertionWithPrf(activity, salt, rpId)
            } catch (e: NoCredentialException) {
                registerCredential(activity, rpId, rpName, userName, userDisplayName)
                getAssertionWithPrf(activity, salt, rpId)
            }
        } catch (e: CredentialManagerPrfCoreException) {
            throw e
        } catch (e: Exception) {
            throw e.toCoreException()
        }
    }

    /**
     * Register a new passkey without deriving a seed.
     *
     * Use this to separate credential creation from derivation in
     * multi-step onboarding flows. Triggers exactly one platform prompt.
     */
    public suspend fun createCredential(
        activity: Activity,
        rpId: String,
        rpName: String,
        userName: String,
        userDisplayName: String,
    ): Unit = withContext(Dispatchers.Main) {
        try {
            registerCredential(activity, rpId, rpName, userName, userDisplayName)
        } catch (e: CredentialManagerPrfCoreException) {
            throw e
        } catch (e: Exception) {
            throw e.toCoreException()
        }
    }

    // ------------------------------------------------------------------
    // Private
    // ------------------------------------------------------------------

    private suspend fun getAssertionWithPrf(
        activity: Activity,
        salt: String,
        rpId: String,
    ): ByteArray {
        val credentialManager = CredentialManager.create(activity)

        val saltBase64 = Base64.encodeToString(
            salt.toByteArray(Charsets.UTF_8),
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

        val authResponseJson = response.credential.data.getString(
            "androidx.credentials.BUNDLE_KEY_AUTHENTICATION_RESPONSE_JSON",
        ) ?: throw CredentialManagerPrfCoreException(
            Kind.AuthenticationFailed,
            "No credential response",
        )

        val responseJson = JSONObject(authResponseJson)
        val extensions = responseJson.optJSONObject("clientExtensionResults")
            ?: throw CredentialManagerPrfCoreException(Kind.PrfNotSupported)
        val prf = extensions.optJSONObject("prf")
            ?: throw CredentialManagerPrfCoreException(Kind.PrfNotSupported)
        val results = prf.optJSONObject("results")
            ?: throw CredentialManagerPrfCoreException(
                Kind.PrfEvaluationFailed,
                "no results",
            )
        val first = results.optString("first")
        if (first.isNullOrEmpty()) {
            throw CredentialManagerPrfCoreException(
                Kind.PrfEvaluationFailed,
                "empty result",
            )
        }
        return Base64.decode(first, Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING)
    }

    private suspend fun registerCredential(
        activity: Activity,
        rpId: String,
        rpName: String,
        userName: String,
        userDisplayName: String,
    ) {
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

        credentialManager.createCredential(
            activity,
            CreatePublicKeyCredentialRequest(requestJson),
        )
    }

    private fun randomBase64Url(byteCount: Int): String {
        val bytes = ByteArray(byteCount)
        SecureRandom().nextBytes(bytes)
        return Base64.encodeToString(
            bytes,
            Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING,
        )
    }

    private fun Exception.toCoreException(): CredentialManagerPrfCoreException = when (this) {
        is GetCredentialCancellationException,
        is CreateCredentialCancellationException ->
            CredentialManagerPrfCoreException(Kind.UserCancelled)

        is NoCredentialException ->
            CredentialManagerPrfCoreException(Kind.CredentialNotFound)

        is GetCredentialException ->
            CredentialManagerPrfCoreException(
                Kind.AuthenticationFailed,
                "${type}: ${message ?: toString()}",
            )

        is CreateCredentialException ->
            CredentialManagerPrfCoreException(
                Kind.AuthenticationFailed,
                "${type}: ${message ?: toString()}",
            )

        else -> {
            val raw = message ?: toString()
            // Actionable hints for common misconfigurations. Mirror what the
            // original Flutter plugin surfaced so existing users see the same
            // guidance after the refactor.
            val hint = when {
                raw.contains("cannot be validated", ignoreCase = true) ->
                    "Domain verification failed. Passkeys require a physical device with " +
                        "Google Play Services and a valid /.well-known/assetlinks.json " +
                        "for the RP domain. Emulators are not supported."
                raw.contains("not supported", ignoreCase = true) ->
                    "Passkeys require Android 9+ with Google Play Services, or Android " +
                        "14+ with a compatible Credential Manager provider."
                else -> raw
            }
            CredentialManagerPrfCoreException(Kind.Generic, hint)
        }
    }

    /** Discriminator for [CredentialManagerPrfCoreException]. */
    public enum class Kind {
        /** The authenticator does not support the WebAuthn PRF extension. */
        PrfNotSupported,
        /** The user dismissed the passkey prompt or cancelled the operation. */
        UserCancelled,
        /** No credential exists for the RP and auto-registration was not attempted. */
        CredentialNotFound,
        /** Credential Manager reported an authentication / registration failure. */
        AuthenticationFailed,
        /** PRF evaluation produced an empty or malformed response. */
        PrfEvaluationFailed,
        /** Any other unexpected error — message contains the details. */
        Generic,
    }
}

/**
 * Typed exception thrown by [CredentialManagerPrfCore]. Wrappers should
 * switch on [kind] to map to their own framework-specific error type.
 */
public class CredentialManagerPrfCoreException(
    public val kind: CredentialManagerPrfCore.Kind,
    message: String? = null,
) : Exception(message)
