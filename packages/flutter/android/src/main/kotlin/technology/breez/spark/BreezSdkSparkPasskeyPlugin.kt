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
import technology.breez.spark.passkey.core.PostCreateGraceTracker

/**
 * Flutter plugin for passkey PRF operations on Android.
 *
 * Thin MethodChannel wrapper around [CredentialManagerPrfCore]. All of the
 * WebAuthn JSON, Credential Manager, and PRF-extraction plumbing lives in
 * the core helper; this file only translates Flutter's method arguments
 * and maps [CredentialManagerPrfCoreException] into MethodChannel error
 * codes understood by the Dart side.
 *
 * Do not hand-edit [CredentialManagerPrfCore.kt] here: it is a generated
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

    /**
     * Shared across each per-call [CredentialManagerPrfCore] instance so the
     * post-create grace armed by one ceremony is consumed by the next.
     * Without sharing, every fresh-per-call core would have its own empty
     * tracker and the cross-call grace would never fire.
     */
    private val graceTracker = PostCreateGraceTracker()

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
            "deriveSeeds" -> handleDeriveSeeds(call, result)
            "createPasskey" -> handleCreatePasskey(call, result)
            "isSupported" -> result.success(CredentialManagerPrfCore.isSupported())
            "checkDomainAssociation" -> handleCheckDomainAssociation(call, result)
            else -> result.notImplemented()
        }
    }

    private fun handleDeriveSeeds(call: MethodCall, result: Result) {
        @Suppress("UNCHECKED_CAST")
        val salts = call.argument<List<String>>("salts")
        val rpId = call.argument<String>("rpId")
        val rpName = call.argument<String>("rpName")
        val userName = call.argument<String>("userName")
        val userDisplayName = call.argument<String>("userDisplayName")
        val autoRegister = call.argument<Boolean>("autoRegister") ?: false

        if (salts == null || rpId == null || rpName == null || userName == null || userDisplayName == null) {
            result.error("ERR_PASSKEY", "Invalid arguments", null)
            return
        }
        val currentActivity = activity ?: run {
            result.error("ERR_PASSKEY", "No activity available", null)
            return
        }

        // Caller-supplied allow-list. The host passes any known
        // credential IDs from the Dart side before the MethodChannel
        // call. The native plugin never reads or writes credential IDs
        // itself.
        val allowIds: List<ByteArray> =
            (call.argument<List<String>>("allowCredentials") ?: emptyList()).map {
                Base64.decode(it, Base64.NO_WRAP)
            }
        val preferImmediate = call.argument<Boolean>("preferImmediatelyAvailableCredentials")

        scope.launch {
            try {
                val derivation = CredentialManagerPrfCore(
                    rpId = rpId,
                    rpName = rpName,
                    userName = userName,
                    userDisplayName = userDisplayName,
                    activityProvider = { currentActivity },
                    graceTracker = graceTracker,
                ).deriveSeeds(
                    salts = salts,
                    autoRegister = autoRegister,
                    allowCredentials = allowIds,
                    preferImmediatelyAvailableCredentials = preferImmediate ?: true,
                )
                // Encode each seed as base64 so MethodChannel can carry it.
                // The asserted credential ID rides alongside (null when no
                // assertion ran) so the Dart side surfaces it via
                // DeriveSeedsOutput, matching the other bindings.
                result.success(mapOf(
                    "seeds" to derivation.seeds.map { Base64.encodeToString(it, Base64.NO_WRAP) },
                    "credentialId" to derivation.credentialId?.let { Base64.encodeToString(it, Base64.NO_WRAP) },
                ))
            } catch (e: CredentialManagerPrfCoreException) {
                result.error(e.errorCode, e.message ?: e.defaultMessage, null)
            } catch (e: Exception) {
                result.error("ERR_PASSKEY", e.message ?: e.toString(), null)
            }
        }
    }

    private fun handleCheckDomainAssociation(call: MethodCall, result: Result) {
        val rpId = call.argument<String>("rpId")
        if (rpId == null) {
            result.error("ERR_PASSKEY", "Invalid arguments", null)
            return
        }
        val currentActivity = activity ?: run {
            result.error("ERR_PASSKEY", "No activity available", null)
            return
        }
        scope.launch {
            try {
                // Branding fields are unused by the domain check; pass
                // rpId as a placeholder since this is a check-only core.
                val outcome = CredentialManagerPrfCore(
                    rpId = rpId,
                    rpName = rpId,
                    userName = rpId,
                    userDisplayName = rpId,
                    activityProvider = { currentActivity },
                ).checkDomainAssociation()
                // Soft-fail to Skipped on Android: see the upstream
                // PasskeyProvider for the rationale (CredentialManager
                // runs its own DAL check internally and a public-API
                // mismatch can be a stale-cache false negative).
                result.success(when (outcome) {
                    is technology.breez.spark.passkey.core.DomainAssociationResult.Associated ->
                        mapOf("kind" to "Associated")
                    is technology.breez.spark.passkey.core.DomainAssociationResult.NotAssociated ->
                        mapOf(
                            "kind" to "Skipped",
                            "reason" to "[soft-fail on Android] ${outcome.reason}",
                        )
                    is technology.breez.spark.passkey.core.DomainAssociationResult.Skipped ->
                        mapOf("kind" to "Skipped", "reason" to outcome.reason)
                })
            } catch (e: Exception) {
                result.success(mapOf(
                    "kind" to "Skipped",
                    "reason" to "Domain association probe failed: ${e.message ?: e.toString()}",
                ))
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

        val excludeIds: List<ByteArray> =
            (call.argument<List<String>>("excludeCredentials") ?: emptyList()).map {
                Base64.decode(it, Base64.NO_WRAP)
            }

        scope.launch {
            try {
                val credential = CredentialManagerPrfCore(
                    rpId = rpId,
                    rpName = rpName,
                    userName = userName,
                    userDisplayName = userDisplayName,
                    activityProvider = { currentActivity },
                    graceTracker = graceTracker,
                ).register(excludeIds)
                // The core's `register` arms the shared grace tracker so the
                // next `deriveSeeds` call holds out the credential's
                // PRF-readiness window without the wrapper having to.
                result.success(mapOf(
                    "credentialId" to Base64.encodeToString(credential.credentialId, Base64.NO_WRAP),
                    "userId" to Base64.encodeToString(credential.userId, Base64.NO_WRAP),
                    "aaguid" to credential.aaguid?.let { Base64.encodeToString(it, Base64.NO_WRAP) },
                    "backupEligible" to credential.backupEligible,
                ))
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
            CredentialManagerPrfCore.Kind.UserTimedOut -> "ERR_USER_TIMED_OUT"
            CredentialManagerPrfCore.Kind.CredentialNotFound -> "ERR_NO_CREDENTIAL"
            CredentialManagerPrfCore.Kind.PrfNotSupported -> "ERR_PRF_NOT_SUPPORTED"
            CredentialManagerPrfCore.Kind.Configuration -> "ERR_CONFIGURATION"
            CredentialManagerPrfCore.Kind.CredentialAlreadyExists -> "ERR_CREDENTIAL_ALREADY_EXISTS"
            else -> "ERR_PASSKEY"
        }

    private val CredentialManagerPrfCoreException.defaultMessage: String
        get() = when (kind) {
            CredentialManagerPrfCore.Kind.UserCancelled -> "User cancelled the passkey operation"
            CredentialManagerPrfCore.Kind.UserTimedOut -> "Authenticator timed out"
            CredentialManagerPrfCore.Kind.CredentialNotFound -> "No passkey credential found for this domain"
            CredentialManagerPrfCore.Kind.PrfNotSupported -> "PRF not supported by authenticator"
            CredentialManagerPrfCore.Kind.AuthenticationFailed -> "Passkey authentication failed"
            CredentialManagerPrfCore.Kind.PrfEvaluationFailed -> "PRF evaluation failed"
            CredentialManagerPrfCore.Kind.Configuration -> "Platform or app configuration error"
            CredentialManagerPrfCore.Kind.CredentialAlreadyExists -> "A passkey for this app already exists on this device"
            CredentialManagerPrfCore.Kind.Generic -> "Passkey operation failed"
        }
}
