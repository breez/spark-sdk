package technology.breez.spark

import android.app.Activity
import android.util.Base64
import io.flutter.embedding.engine.plugins.FlutterPlugin
import io.flutter.embedding.engine.plugins.activity.ActivityAware
import io.flutter.embedding.engine.plugins.activity.ActivityPluginBinding
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel
import io.flutter.plugin.common.MethodChannel.MethodCallHandler
import io.flutter.plugin.common.MethodChannel.Result
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import technology.breez.spark.passkey.core.CredentialManagerPrfCore
import technology.breez.spark.passkey.core.CredentialManagerPrfCoreException

/**
 * Flutter plugin for passkey PRF operations on Android.
 *
 * Thin MethodChannel wrapper around [CredentialManagerPrfCore]. All of the
 * WebAuthn JSON, Credential Manager, and PRF-extraction plumbing lives in
 * the core helper; this file only translates Flutter's method arguments
 * and maps [CredentialManagerPrfCoreException] into MethodChannel error
 * codes understood by the Dart side.
 *
 * Do not hand-edit [CredentialManagerPrfCore.kt] here — it is a generated
 * mirror of the canonical copy under
 * `crates/breez-sdk/bindings/langs/shared/android-passkey/`. Run
 * `cargo xtask sync-passkey-core` after editing the canonical file.
 */
class BreezSdkSparkPasskeyPlugin : FlutterPlugin, MethodCallHandler, ActivityAware {

    private lateinit var channel: MethodChannel
    private var activity: Activity? = null

    /**
     * Plugin-scoped coroutine scope. Cancelled in [onDetachedFromEngine] so
     * any in-flight passkey ceremony does not outlive the plugin and leak
     * the captured Activity. SupervisorJob keeps siblings alive if one
     * branch fails, matching the per-call try/catch pattern below.
     */
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)

    override fun onAttachedToEngine(binding: FlutterPlugin.FlutterPluginBinding) {
        channel = MethodChannel(binding.binaryMessenger, "breez_sdk_spark_passkey")
        channel.setMethodCallHandler(this)
    }

    override fun onDetachedFromEngine(binding: FlutterPlugin.FlutterPluginBinding) {
        channel.setMethodCallHandler(null)
        scope.cancel()
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
            "derivePrfSeed" -> handleDerivePrfSeed(call, result)
            "createPasskey" -> handleCreatePasskey(call, result)
            "isPrfAvailable" -> result.success(CredentialManagerPrfCore.isPrfAvailable())
            else -> result.notImplemented()
        }
    }

    private fun handleDerivePrfSeed(call: MethodCall, result: Result) {
        val salt = call.argument<String>("salt")
        val rpId = call.argument<String>("rpId")
        val rpName = call.argument<String>("rpName")
        val userName = call.argument<String>("userName")
        val userDisplayName = call.argument<String>("userDisplayName")

        if (salt == null || rpId == null || rpName == null || userName == null || userDisplayName == null) {
            result.error("ERR_PASSKEY", "Invalid arguments", null)
            return
        }
        val currentActivity = activity ?: run {
            result.error("ERR_PASSKEY", "No activity available", null)
            return
        }

        scope.launch {
            try {
                val prfOutput = CredentialManagerPrfCore.deriveSeedOrRegister(
                    activity = currentActivity,
                    salt = salt,
                    rpId = rpId,
                    rpName = rpName,
                    userName = userName,
                    userDisplayName = userDisplayName,
                )
                result.success(Base64.encodeToString(prfOutput, Base64.NO_WRAP))
            } catch (e: CredentialManagerPrfCoreException) {
                result.error(e.errorCode, e.message ?: e.defaultMessage, null)
            } catch (e: Exception) {
                result.error("ERR_PASSKEY", e.message ?: e.toString(), null)
            }
        }
    }

    private fun handleCreatePasskey(call: MethodCall, result: Result) {
        val rpId = call.argument<String>("rpId")
        val rpName = call.argument<String>("rpName")
        val userName = call.argument<String>("userName")
        val userDisplayName = call.argument<String>("userDisplayName")

        if (rpId == null || rpName == null || userName == null || userDisplayName == null) {
            result.error("ERR_PASSKEY", "Invalid arguments", null)
            return
        }
        val currentActivity = activity ?: run {
            result.error("ERR_PASSKEY", "No activity available", null)
            return
        }

        scope.launch {
            try {
                CredentialManagerPrfCore.createCredential(
                    activity = currentActivity,
                    rpId = rpId,
                    rpName = rpName,
                    userName = userName,
                    userDisplayName = userDisplayName,
                )
                result.success(null)
            } catch (e: CredentialManagerPrfCoreException) {
                result.error(e.errorCode, e.message ?: e.defaultMessage, null)
            } catch (e: Exception) {
                result.error("ERR_PASSKEY", e.message ?: e.toString(), null)
            }
        }
    }

    private val CredentialManagerPrfCoreException.errorCode: String
        get() = when (kind) {
            CredentialManagerPrfCore.Kind.UserCancelled -> "ERR_USER_CANCELLED"
            CredentialManagerPrfCore.Kind.CredentialNotFound -> "ERR_NO_CREDENTIAL"
            else -> "ERR_PASSKEY"
        }

    private val CredentialManagerPrfCoreException.defaultMessage: String
        get() = when (kind) {
            CredentialManagerPrfCore.Kind.UserCancelled -> "User cancelled the passkey operation"
            CredentialManagerPrfCore.Kind.CredentialNotFound -> "No passkey credential found for this domain"
            CredentialManagerPrfCore.Kind.PrfNotSupported -> "PRF not supported by authenticator"
            CredentialManagerPrfCore.Kind.AuthenticationFailed -> "Passkey authentication failed"
            CredentialManagerPrfCore.Kind.PrfEvaluationFailed -> "PRF evaluation failed"
            CredentialManagerPrfCore.Kind.Generic -> "Passkey operation failed"
        }
}
