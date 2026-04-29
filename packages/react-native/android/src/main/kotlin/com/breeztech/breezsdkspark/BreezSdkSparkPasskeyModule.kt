package com.breeztech.breezsdkspark

import android.util.Base64
import com.facebook.react.bridge.Promise
import com.facebook.react.bridge.ReactApplicationContext
import com.facebook.react.bridge.ReactContextBaseJavaModule
import com.facebook.react.bridge.ReactMethod
import com.facebook.react.module.annotations.ReactModule
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import technology.breez.spark.passkey.core.CredentialManagerPrfCore
import technology.breez.spark.passkey.core.CredentialManagerPrfCoreException

/**
 * React Native native module for passkey PRF operations on Android.
 *
 * Thin React-bridge wrapper around [CredentialManagerPrfCore]. All of the
 * WebAuthn JSON, Credential Manager, and PRF-extraction plumbing lives in
 * the core helper; this file only translates React Native arguments and
 * maps [CredentialManagerPrfCoreException] into Promise rejection codes
 * understood by the JS side.
 *
 * Do not hand-edit [CredentialManagerPrfCore.kt] here — it is a generated
 * mirror of the canonical copy under
 * `crates/breez-sdk/bindings/langs/shared/android-passkey/`. Run
 * `cargo xtask sync-passkey-core` after editing the canonical file.
 */
@ReactModule(name = BreezSdkSparkPasskeyModule.NAME)
class BreezSdkSparkPasskeyModule(
    private val reactContext: ReactApplicationContext,
) : ReactContextBaseJavaModule(reactContext) {

    /**
     * Module-scoped coroutine scope. Cancelled in [onCatalystInstanceDestroy]
     * so any in-flight passkey ceremony does not outlive the React context
     * and leak the captured Activity. SupervisorJob keeps siblings alive if
     * one branch fails, matching the per-call try/catch pattern below.
     */
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)

    override fun getName(): String = NAME

    override fun onCatalystInstanceDestroy() {
        scope.cancel()
        super.onCatalystInstanceDestroy()
    }

    /**
     * Derive a 32-byte PRF seed from a passkey assertion.
     *
     * @param promise Resolves with a base64-encoded 32-byte PRF output.
     */
    @ReactMethod
    fun derivePrfSeed(
        salt: String,
        rpId: String,
        rpName: String,
        userName: String,
        userDisplayName: String,
        promise: Promise,
    ) {
        val activity = currentActivity
        if (activity == null) {
            promise.reject("ERR_NO_ACTIVITY", "No current activity available")
            return
        }

        scope.launch {
            try {
                val prfOutput = CredentialManagerPrfCore.deriveSeedOrRegister(
                    activity = activity,
                    salt = salt,
                    rpId = rpId,
                    rpName = rpName,
                    userName = userName,
                    userDisplayName = userDisplayName,
                )
                promise.resolve(Base64.encodeToString(prfOutput, Base64.NO_WRAP))
            } catch (e: CredentialManagerPrfCoreException) {
                promise.reject(e.errorCode, e.message ?: e.defaultMessage)
            } catch (e: Exception) {
                promise.reject("ERR_PASSKEY", e.message ?: e.toString())
            }
        }
    }

    /**
     * Create a new passkey with PRF support.
     *
     * Only registers the credential — no seed derivation. Triggers exactly
     * one platform prompt. Use for multi-step onboarding flows.
     */
    @ReactMethod
    fun createPasskey(
        rpId: String,
        rpName: String,
        userName: String,
        userDisplayName: String,
        promise: Promise,
    ) {
        val activity = currentActivity
        if (activity == null) {
            promise.reject("ERR_NO_ACTIVITY", "No current activity available")
            return
        }

        scope.launch {
            try {
                CredentialManagerPrfCore.createCredential(
                    activity = activity,
                    rpId = rpId,
                    rpName = rpName,
                    userName = userName,
                    userDisplayName = userDisplayName,
                )
                promise.resolve(null)
            } catch (e: CredentialManagerPrfCoreException) {
                promise.reject(e.errorCode, e.message ?: e.defaultMessage)
            } catch (e: Exception) {
                promise.reject("ERR_PASSKEY", e.message ?: e.toString())
            }
        }
    }

    /** Check if PRF-capable passkeys are available on this device. */
    @ReactMethod
    fun isPrfAvailable(promise: Promise) {
        promise.resolve(CredentialManagerPrfCore.isPrfAvailable())
    }

    private val CredentialManagerPrfCoreException.errorCode: String
        get() = when (kind) {
            CredentialManagerPrfCore.Kind.UserCancelled -> "ERR_USER_CANCELLED"
            CredentialManagerPrfCore.Kind.CredentialNotFound -> "ERR_NO_CREDENTIAL"
            CredentialManagerPrfCore.Kind.PrfNotSupported -> "ERR_PRF_NOT_SUPPORTED"
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

    companion object {
        const val NAME = "BreezSdkSparkPasskey"
    }
}
