package com.breeztech.breezsdkspark

import android.os.Build
import android.util.Base64
import com.facebook.react.bridge.Promise
import com.facebook.react.bridge.ReactApplicationContext
import com.facebook.react.bridge.ReactContextBaseJavaModule
import com.facebook.react.bridge.ReactMethod
import com.facebook.react.module.annotations.ReactModule
import androidx.credentials.CredentialManager
import androidx.credentials.GetCredentialRequest
import androidx.credentials.GetPublicKeyCredentialOption
import androidx.credentials.CreatePublicKeyCredentialRequest
import androidx.credentials.exceptions.GetCredentialCancellationException
import androidx.credentials.exceptions.NoCredentialException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import org.json.JSONArray
import org.json.JSONObject
import java.security.SecureRandom

/**
 * React Native native module for passkey PRF operations on Android.
 *
 * Uses Android's Credential Manager API with WebAuthn and the PRF extension.
 * Auto-registers a new credential on first use if none exists.
 */
@ReactModule(name = BreezSdkSparkPasskeyModule.NAME)
class BreezSdkSparkPasskeyModule(
    private val reactContext: ReactApplicationContext
) : ReactContextBaseJavaModule(reactContext) {

    override fun getName(): String = NAME

    private val credentialManager: CredentialManager by lazy {
        CredentialManager.create(reactContext)
    }

    /**
     * Derive a 32-byte PRF seed from a passkey assertion.
     *
     * @param salt The salt string for PRF evaluation.
     * @param rpId The Relying Party ID.
     * @param rpName The RP display name (used during registration).
     * @param userName User name for credential registration.
     * @param userDisplayName User display name for credential registration.
     * @param promise Resolves with a base64-encoded 32-byte PRF output.
     */
    @ReactMethod
    fun derivePrfSeed(
        salt: String, rpId: String, rpName: String,
        userName: String, userDisplayName: String,
        promise: Promise
    ) {
        val activity = currentActivity
        if (activity == null) {
            promise.reject("ERR_NO_ACTIVITY", "No current activity available")
            return
        }

        CoroutineScope(Dispatchers.Main).launch {
            try {
                val saltBytes = salt.toByteArray(Charsets.UTF_8)
                val saltBase64 = base64UrlEncode(saltBytes)

                val result = try {
                    getAssertionWithPrf(saltBase64, rpId, activity)
                } catch (e: NoCredentialFound) {
                    registerCredential(rpId, rpName, userName, userDisplayName, activity)
                    getAssertionWithPrf(saltBase64, rpId, activity)
                }

                promise.resolve(Base64.encodeToString(result, Base64.NO_WRAP))
            } catch (e: UserCancelledException) {
                promise.reject("ERR_USER_CANCELLED", "User cancelled authentication")
            } catch (e: PrfNotSupportedException) {
                promise.reject("ERR_PRF_NOT_SUPPORTED", "PRF not supported by authenticator")
            } catch (e: Exception) {
                promise.reject("ERR_PASSKEY", e.message ?: "Passkey operation failed")
            }
        }
    }

    /**
     * Create a new passkey with PRF support.
     *
     * Only registers the credential — no seed derivation. Triggers exactly
     * 1 platform prompt. Use for multi-step onboarding flows.
     *
     * @param rpId The Relying Party ID.
     * @param rpName The RP display name.
     * @param userName User name for credential registration.
     * @param userDisplayName User display name for credential registration.
     * @param promise Resolves with null on success.
     */
    @ReactMethod
    fun createPasskey(
        rpId: String, rpName: String,
        userName: String, userDisplayName: String,
        promise: Promise
    ) {
        val activity = currentActivity
        if (activity == null) {
            promise.reject("ERR_NO_ACTIVITY", "No current activity available")
            return
        }

        CoroutineScope(Dispatchers.Main).launch {
            try {
                registerCredential(rpId, rpName, userName, userDisplayName, activity)
                promise.resolve(null)
            } catch (e: UserCancelledException) {
                promise.reject("ERR_USER_CANCELLED", "User cancelled registration")
            } catch (e: Exception) {
                promise.reject("ERR_PASSKEY", e.message ?: "Passkey registration failed")
            }
        }
    }

    /**
     * Check if PRF-capable passkeys are available on this device.
     *
     * Passkeys with PRF are supported on Android 9+ (API 28) via Google Play
     * Services. The Jetpack Credential Manager library handles backward
     * compatibility automatically.
     *
     * @param promise Resolves with a boolean.
     */
    @ReactMethod
    fun isPrfAvailable(promise: Promise) {
        promise.resolve(Build.VERSION.SDK_INT >= Build.VERSION_CODES.P)
    }

    private suspend fun getAssertionWithPrf(
        saltBase64: String,
        rpId: String,
        activity: android.app.Activity
    ): ByteArray {
        val challenge = base64UrlEncode(randomBytes(32))

        val requestJson = JSONObject().apply {
            put("challenge", challenge)
            put("rpId", rpId)
            put("allowCredentials", JSONArray())
            put("userVerification", "required")
            put("extensions", JSONObject().apply {
                put("prf", JSONObject().apply {
                    put("eval", JSONObject().apply {
                        put("first", saltBase64)
                    })
                })
            })
        }.toString()

        val credentialOption = GetPublicKeyCredentialOption(requestJson)
        val request = GetCredentialRequest.Builder()
            .addCredentialOption(credentialOption)
            .build()

        val response = try {
            credentialManager.getCredential(activity, request)
        } catch (e: NoCredentialException) {
            throw NoCredentialFound()
        } catch (e: GetCredentialCancellationException) {
            throw UserCancelledException()
        }

        val responseJson = response.credential.data
            .getString("androidx.credentials.BUNDLE_KEY_AUTHENTICATION_RESPONSE_JSON")
            ?: throw PrfNotSupportedException()

        return extractPrfFromResponse(responseJson)
    }

    private fun extractPrfFromResponse(responseJson: String): ByteArray {
        val json = JSONObject(responseJson)
        val extensions = json.optJSONObject("clientExtensionResults")
            ?: throw PrfNotSupportedException()
        val prf = extensions.optJSONObject("prf")
            ?: throw PrfNotSupportedException()
        val results = prf.optJSONObject("results")
            ?: throw PrfNotSupportedException()
        val firstBase64 = results.optString("first", "")
        if (firstBase64.isEmpty()) throw PrfNotSupportedException()
        return base64UrlDecode(firstBase64)
    }

    private suspend fun registerCredential(
        rpId: String, rpName: String,
        userName: String, userDisplayName: String,
        activity: android.app.Activity
    ) {
        val challenge = base64UrlEncode(randomBytes(32))
        val userId = base64UrlEncode(randomBytes(16))

        val requestJson = JSONObject().apply {
            put("challenge", challenge)
            put("rp", JSONObject().apply {
                put("id", rpId)
                put("name", rpName)
            })
            put("user", JSONObject().apply {
                put("id", userId)
                put("name", userName)
                put("displayName", userDisplayName)
            })
            put("pubKeyCredParams", JSONArray().apply {
                put(JSONObject().apply { put("type", "public-key"); put("alg", -7) })
                put(JSONObject().apply { put("type", "public-key"); put("alg", -257) })
            })
            put("authenticatorSelection", JSONObject().apply {
                put("residentKey", "required")
                put("requireResidentKey", true)
                put("userVerification", "required")
            })
            put("extensions", JSONObject().apply {
                put("prf", JSONObject())
            })
        }.toString()

        try {
            credentialManager.createCredential(activity, CreatePublicKeyCredentialRequest(requestJson))
        } catch (e: androidx.credentials.exceptions.CreateCredentialCancellationException) {
            throw UserCancelledException()
        }
    }

    private class NoCredentialFound : Exception()
    private class UserCancelledException : Exception()
    private class PrfNotSupportedException : Exception()

    companion object {
        const val NAME = "BreezSdkSparkPasskey"

        private val secureRandom = SecureRandom()

        private fun randomBytes(count: Int): ByteArray {
            val bytes = ByteArray(count)
            secureRandom.nextBytes(bytes)
            return bytes
        }

        private fun base64UrlEncode(data: ByteArray): String {
            return Base64.encodeToString(data, Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP)
        }

        private fun base64UrlDecode(data: String): ByteArray {
            return Base64.decode(data, Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP)
        }
    }
}
