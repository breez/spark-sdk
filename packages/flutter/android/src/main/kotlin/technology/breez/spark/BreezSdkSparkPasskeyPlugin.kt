package technology.breez.spark

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
import io.flutter.embedding.engine.plugins.FlutterPlugin
import io.flutter.embedding.engine.plugins.activity.ActivityAware
import io.flutter.embedding.engine.plugins.activity.ActivityPluginBinding
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel
import io.flutter.plugin.common.MethodChannel.MethodCallHandler
import io.flutter.plugin.common.MethodChannel.Result
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import org.json.JSONObject
import java.security.SecureRandom

/**
 * Flutter plugin for passkey PRF operations on Android.
 *
 * Uses `androidx.credentials.CredentialManager` with WebAuthn JSON requests.
 * Auto-registers a new credential on first use if none exists.
 *
 * Requires Android 14+ (API 34) for Credential Manager with PRF support.
 */
class BreezSdkSparkPasskeyPlugin : FlutterPlugin, MethodCallHandler, ActivityAware {

    private lateinit var channel: MethodChannel
    private var activity: Activity? = null

    override fun onAttachedToEngine(binding: FlutterPlugin.FlutterPluginBinding) {
        channel = MethodChannel(binding.binaryMessenger, "breez_sdk_spark_passkey")
        channel.setMethodCallHandler(this)
    }

    override fun onDetachedFromEngine(binding: FlutterPlugin.FlutterPluginBinding) {
        channel.setMethodCallHandler(null)
    }

    override fun onAttachedToActivity(binding: ActivityPluginBinding) {
        activity = binding.activity
    }

    override fun onDetachedFromActivityForConfigChanges() {
        activity = null
    }

    override fun onReattachedToActivityForConfigChanges(binding: ActivityPluginBinding) {
        activity = binding.activity
    }

    override fun onDetachedFromActivity() {
        activity = null
    }

    override fun onMethodCall(call: MethodCall, result: Result) {
        when (call.method) {
            "derivePrfSeed" -> {
                val salt = call.argument<String>("salt")
                val rpId = call.argument<String>("rpId")
                val rpName = call.argument<String>("rpName")
                val userName = call.argument<String>("userName")
                val userDisplayName = call.argument<String>("userDisplayName")

                if (salt == null || rpId == null || rpName == null || userName == null || userDisplayName == null) {
                    result.error("ERR_PASSKEY", "Invalid arguments", null)
                    return
                }

                val currentActivity = activity
                if (currentActivity == null) {
                    result.error("ERR_PASSKEY", "No activity available", null)
                    return
                }

                CoroutineScope(Dispatchers.Main).launch {
                    try {
                        val prfOutput = performDerivation(currentActivity, salt, rpId, rpName, userName, userDisplayName)
                        result.success(Base64.encodeToString(prfOutput, Base64.NO_WRAP))
                    } catch (e: Exception) {
                        handleError(e, result)
                    }
                }
            }

            "createPasskey" -> {
                val rpId = call.argument<String>("rpId")
                val rpName = call.argument<String>("rpName")
                val userName = call.argument<String>("userName")
                val userDisplayName = call.argument<String>("userDisplayName")

                if (rpId == null || rpName == null || userName == null || userDisplayName == null) {
                    result.error("ERR_PASSKEY", "Invalid arguments", null)
                    return
                }

                val currentActivity = activity
                if (currentActivity == null) {
                    result.error("ERR_PASSKEY", "No activity available", null)
                    return
                }

                CoroutineScope(Dispatchers.Main).launch {
                    try {
                        registerCredential(currentActivity, rpId, rpName, userName, userDisplayName)
                        result.success(null)
                    } catch (e: Exception) {
                        handleError(e, result)
                    }
                }
            }

            "isPrfAvailable" -> {
                // Passkeys with PRF are supported on Android 9+ (API 28) via
                // Google Play Services. The Jetpack Credential Manager library
                // handles backward compatibility automatically.
                result.success(Build.VERSION.SDK_INT >= Build.VERSION_CODES.P)
            }

            else -> result.notImplemented()
        }
    }

    // MARK: - Error Handling

    private fun handleError(e: Exception, result: Result) {
        when (e) {
            is GetCredentialCancellationException,
            is CreateCredentialCancellationException ->
                result.error("ERR_USER_CANCELLED", "User cancelled the passkey operation", null)

            is NoCredentialException ->
                result.error("ERR_NO_CREDENTIAL", "No passkey credential found for this domain", null)

            is GetCredentialException ->
                result.error("ERR_PASSKEY", "Passkey authentication failed: ${e.type} - ${e.message}", null)

            is CreateCredentialException ->
                result.error("ERR_PASSKEY", "Passkey registration failed: ${e.type} - ${e.message}", null)

            else -> {
                val message = e.message ?: e.toString()
                // Surface actionable hints for common issues
                val hint = when {
                    message.contains("cannot be validated", ignoreCase = true) ->
                        "Domain verification failed. Passkeys require a physical device with Google Play Services " +
                        "and a valid /.well-known/assetlinks.json for the RP domain. " +
                        "Emulators are not supported."
                    message.contains("not supported", ignoreCase = true) ->
                        "Passkeys require Android 14+ with Google Play Services."
                    else -> message
                }
                result.error("ERR_PASSKEY", hint, null)
            }
        }
    }

    // MARK: - Private

    private suspend fun performDerivation(
        activity: Activity,
        salt: String,
        rpId: String,
        rpName: String,
        userName: String,
        userDisplayName: String
    ): ByteArray {
        return try {
            getAssertionWithPrf(activity, salt, rpId)
        } catch (e: NoCredentialException) {
            registerCredential(activity, rpId, rpName, userName, userDisplayName)
            getAssertionWithPrf(activity, salt, rpId)
        }
    }

    private suspend fun getAssertionWithPrf(
        activity: Activity,
        salt: String,
        rpId: String
    ): ByteArray {
        val credentialManager = CredentialManager.create(activity)

        val saltBytes = salt.toByteArray(Charsets.UTF_8)
        val saltBase64 = Base64.encodeToString(saltBytes, Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING)
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
        val responseJson = JSONObject(credential.data.getString("androidx.credentials.BUNDLE_KEY_AUTHENTICATION_RESPONSE_JSON")
            ?: throw Exception("No credential response"))

        val extensions = responseJson.optJSONObject("clientExtensionResults")
            ?: throw Exception("PRF not supported: no extension results")
        val prf = extensions.optJSONObject("prf")
            ?: throw Exception("PRF not supported by authenticator")
        val results = prf.optJSONObject("results")
            ?: throw Exception("PRF evaluation failed: no results")
        val first = results.optString("first")
        if (first.isNullOrEmpty()) {
            throw Exception("PRF evaluation failed: empty result")
        }

        return Base64.decode(first, Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING)
    }

    private suspend fun registerCredential(
        activity: Activity,
        rpId: String,
        rpName: String,
        userName: String,
        userDisplayName: String
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

        val request = CreatePublicKeyCredentialRequest(requestJson)
        credentialManager.createCredential(activity, request)
    }

    private fun randomBase64Url(length: Int): String {
        val bytes = ByteArray(length)
        SecureRandom().nextBytes(bytes)
        return Base64.encodeToString(bytes, Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING)
    }
}
