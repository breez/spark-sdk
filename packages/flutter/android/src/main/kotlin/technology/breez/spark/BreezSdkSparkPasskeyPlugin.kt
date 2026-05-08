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
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import technology.breez.spark.passkey.KnownCredentialsStore
import technology.breez.spark.passkey.core.CredentialManagerPrfCore
import technology.breez.spark.passkey.core.CredentialManagerPrfCoreException

/**
 * After a successful `createPasskey` the platform takes a moment to
 * make the new credential PRF-ready. On Apple Passwords this surfaces
 * as the dual-salt assertion dropping `prf.second`; on GPM the cred
 * may briefly be invisible to the picker. Holding the next derive
 * call for up to [POST_CREATE_GRACE_TOTAL_MS] lets the OS finish
 * indexing. Mirrors the Capacitor plugin's `PostCreateGraceTracker`.
 */
private class PostCreateGraceTracker {
    private val mutex = Mutex()
    @Volatile private var deadlineMs: Long = 0L

    suspend fun arm(durationMs: Long) {
        mutex.withLock {
            deadlineMs = System.currentTimeMillis() + durationMs
        }
    }

    suspend fun consume() {
        val waitMs = mutex.withLock {
            val now = System.currentTimeMillis()
            val remaining = deadlineMs - now
            deadlineMs = 0L
            if (remaining > 0L) remaining else 0L
        }
        if (waitMs > 0L) {
            delay(waitMs)
        }
    }
}

private const val POST_CREATE_GRACE_TOTAL_MS: Long = 800L

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

        // Caller-supplied allow-list (e.g. host-tracked credential IDs)
        // takes precedence over the implicit KnownCredentialsStore set
        // when present.
        val callerAllow: List<ByteArray> =
            (call.argument<List<String>>("allowCredentialIds") ?: emptyList()).map {
                Base64.decode(it, Base64.NO_WRAP)
            }
        val allowIds = if (callerAllow.isNotEmpty()) {
            callerAllow
        } else {
            readKnownCredentialIds(currentActivity.applicationContext, rpId)
        }

        scope.launch {
            try {
                graceTracker.consume()
                val seeds = CredentialManagerPrfCore.deriveSeedsOrRegister(
                    activity = currentActivity,
                    salts = salts,
                    rpId = rpId,
                    rpName = rpName,
                    userName = userName,
                    userDisplayName = userDisplayName,
                    autoRegister = autoRegister,
                    allowCredentialIds = allowIds,
                )
                // Encode each seed as base64 so MethodChannel can carry
                // it as a List<String>. Dart side base64-decodes back
                // to Uint8List.
                result.success(seeds.map { Base64.encodeToString(it, Base64.NO_WRAP) })
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
                val outcome = CredentialManagerPrfCore.checkDomainAssociation(
                    context = currentActivity.applicationContext,
                    rpId = rpId,
                )
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

        val callerExcludes: List<ByteArray> =
            (call.argument<List<String>>("excludeCredentialIds") ?: emptyList()).map {
                Base64.decode(it, Base64.NO_WRAP)
            }
        val userIdOverride: ByteArray? =
            call.argument<String>("userId")?.let { Base64.decode(it, Base64.NO_WRAP) }
        val context = currentActivity.applicationContext

        scope.launch {
            try {
                // Auto-merge previously-registered credential IDs so the
                // platform refuses duplicates even after a reinstall.
                val merged = mergeKnownCredentials(context, rpId, callerExcludes)
                val credential = CredentialManagerPrfCore.createCredential(
                    activity = currentActivity,
                    rpId = rpId,
                    rpName = rpName,
                    userName = userName,
                    userDisplayName = userDisplayName,
                    excludeCredentialIds = merged,
                    userIdOverride = userIdOverride,
                )
                KnownCredentialsStore.add(
                    context,
                    Base64.encodeToString(
                        credential.credentialId,
                        Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING,
                    ),
                    rpId,
                )
                // Hold the next derive call for up to 800ms so the
                // immediate post-register assertion doesn't race the
                // credential's PRF-readiness window.
                graceTracker.arm(POST_CREATE_GRACE_TOTAL_MS)
                result.success(mapOf(
                    "credentialId" to Base64.encodeToString(credential.credentialId, Base64.NO_WRAP),
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

    private suspend fun mergeKnownCredentials(
        context: android.content.Context,
        rpId: String,
        caller: List<ByteArray>,
    ): List<ByteArray> {
        val known = readKnownCredentialIds(context, rpId)
        if (known.isEmpty()) return caller
        val seen = caller.map { it.toList() }.toMutableSet()
        val out = caller.toMutableList()
        for (id in known) {
            if (seen.add(id.toList())) {
                out.add(id)
            }
        }
        return out
    }

    /// Read all known credential IDs for [rpId] from the iCloud-Keychain
    /// equivalent (encrypted SharedPreferences + Block Store). Used by
    /// the derive paths to populate `allowCredentials` so the OS auto-
    /// routes to the registering provider after a fresh `createPasskey`,
    /// skipping the "select your passkey" picker. Without this the user
    /// sees an extra prompt between create and the post-register PRF
    /// assertion. Mirrors the Capacitor plugin's allowCredentialIds path.
    private suspend fun readKnownCredentialIds(
        context: android.content.Context,
        rpId: String,
    ): List<ByteArray> {
        return KnownCredentialsStore.read(context, rpId).map {
            Base64.decode(it, Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING)
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
