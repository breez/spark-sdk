package com.breeztech.breezsdkspark

import android.util.Base64
import com.facebook.react.bridge.Arguments
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
import technology.breez.spark.passkey.core.PostCreateGraceTracker

/**
 * React Native native module for passkey PRF operations on Android.
 *
 * Thin React-bridge wrapper around [CredentialManagerPrfCore]. All of the
 * WebAuthn JSON, Credential Manager, and PRF-extraction plumbing lives in
 * the core helper; this file only translates React Native arguments and
 * maps [CredentialManagerPrfCoreException] into Promise rejection codes
 * understood by the JS side.
 *
 * Do not hand-edit [CredentialManagerPrfCore.kt] here: it is a generated
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

    /**
     * Shared across each per-call [CredentialManagerPrfCore] instance so the
     * post-create grace armed by one ceremony is consumed by the next.
     * Without sharing, every fresh-per-call core would have its own empty
     * tracker and the cross-call grace would never fire.
     */
    private val graceTracker = PostCreateGraceTracker()

    override fun getName(): String = NAME

    override fun onCatalystInstanceDestroy() {
        scope.cancel()
        super.onCatalystInstanceDestroy()
    }

    /**
     * Derive multiple 32-byte PRF seeds in a single ceremony when supported
     * (dual-salt assertion). Falls back to per-salt single-salt assertion
     * if the authenticator drops the second salt. The `salts.size == 1`
     * case short-circuits to a single-salt assertion (one prompt).
     *
     * @param promise Resolves with a list of base64-encoded 32-byte PRF outputs.
     */
    @ReactMethod
    fun deriveSeeds(
        saltsArg: com.facebook.react.bridge.ReadableArray,
        rpId: String,
        rpName: String,
        userName: String,
        userDisplayName: String,
        autoRegister: Boolean,
        allowCredentialsArg: com.facebook.react.bridge.ReadableArray,
        preferImmediatelyAvailableCredentials: Boolean?,
        promise: Promise,
    ) {
        val activity = currentActivity
        if (activity == null) {
            promise.reject("ERR_NO_ACTIVITY", "No current activity available")
            return
        }

        val salts = mutableListOf<String>()
        for (i in 0 until saltsArg.size()) {
            val s = saltsArg.getString(i)
            if (s == null) {
                promise.reject("ERR_PASSKEY", "Invalid salt at index $i")
                return
            }
            salts.add(s)
        }

        val allowIds = mutableListOf<ByteArray>()
        for (i in 0 until allowCredentialsArg.size()) {
            val b64 = allowCredentialsArg.getString(i) ?: continue
            allowIds.add(Base64.decode(b64, Base64.NO_WRAP))
        }

        scope.launch {
            try {
                val derivation = CredentialManagerPrfCore(
                    rpId = rpId,
                    rpName = rpName,
                    userName = userName,
                    userDisplayName = userDisplayName,
                    activityProvider = { activity },
                    graceTracker = graceTracker,
                ).deriveSeeds(
                    salts = salts,
                    autoRegister = autoRegister,
                    allowCredentials = allowIds,
                    preferImmediatelyAvailableCredentials = preferImmediatelyAvailableCredentials ?: true,
                )
                // Encode each seed as base64 so the React bridge can carry
                // them as an array of strings, plus the asserted credential
                // ID. JS side base64-decodes back to Uint8Array.
                val seedsArr = Arguments.createArray()
                for (seed in derivation.seeds) {
                    seedsArr.pushString(Base64.encodeToString(seed, Base64.NO_WRAP))
                }
                val result = Arguments.createMap()
                result.putArray("seeds", seedsArr)
                val credentialId = derivation.credentialId
                if (credentialId != null) {
                    result.putString("credentialId", Base64.encodeToString(credentialId, Base64.NO_WRAP))
                } else {
                    result.putNull("credentialId")
                }
                promise.resolve(result)
            } catch (e: CredentialManagerPrfCoreException) {
                promise.reject(e.errorCode, e.message ?: e.defaultMessage)
            } catch (e: Exception) {
                promise.reject("ERR_PASSKEY", e.message ?: e.toString())
            }
        }
    }

    /**
     * Domain association check. Mirrors Flutter Android: degrades
     * `NotAssociated` results from the public Digital Asset Links API
     * to `Skipped`, since CredentialManager runs its own check
     * internally with a fresher GMS-cached statement set.
     */
    @ReactMethod
    fun checkDomainAssociation(rpId: String, promise: Promise) {
        val activity = currentActivity
        if (activity == null) {
            promise.reject("ERR_NO_ACTIVITY", "No current activity available")
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
                    activityProvider = { activity },
                ).checkDomainAssociation()
                val map = Arguments.createMap()
                when (outcome) {
                    is technology.breez.spark.passkey.core.DomainAssociationResult.Associated -> {
                        map.putString("kind", "Associated")
                    }
                    is technology.breez.spark.passkey.core.DomainAssociationResult.NotAssociated -> {
                        map.putString("kind", "Skipped")
                        map.putString("reason", "[soft-fail on Android] ${outcome.reason}")
                    }
                    is technology.breez.spark.passkey.core.DomainAssociationResult.Skipped -> {
                        map.putString("kind", "Skipped")
                        map.putString("reason", outcome.reason)
                    }
                }
                promise.resolve(map)
            } catch (e: Exception) {
                val map = Arguments.createMap()
                map.putString("kind", "Skipped")
                map.putString("reason", "Domain association probe failed: ${e.message ?: e.toString()}")
                promise.resolve(map)
            }
        }
    }

    /**
     * Create a new passkey with PRF support.
     *
     * Only registers the credential, no seed derivation. Triggers exactly
     * one platform prompt. Use for multi-step onboarding flows.
     */
    @ReactMethod
    fun createPasskey(
        rpId: String,
        rpName: String,
        userName: String,
        userDisplayName: String,
        excludeCredentialsBase64: com.facebook.react.bridge.ReadableArray,
        promise: Promise,
    ) {
        val activity = currentActivity
        if (activity == null) {
            promise.reject("ERR_NO_ACTIVITY", "No current activity available")
            return
        }

        val excludeIds = mutableListOf<ByteArray>()
        for (i in 0 until excludeCredentialsBase64.size()) {
            val b64 = excludeCredentialsBase64.getString(i)
            excludeIds.add(Base64.decode(b64, Base64.NO_WRAP))
        }

        scope.launch {
            try {
                val credential = CredentialManagerPrfCore(
                    rpId = rpId,
                    rpName = rpName,
                    userName = userName,
                    userDisplayName = userDisplayName,
                    activityProvider = { activity },
                    graceTracker = graceTracker,
                ).register(excludeIds)
                // The core's `register` arms the shared grace tracker so the
                // next `deriveSeeds` call holds out the credential's
                // PRF-readiness window without the wrapper having to.
                val map = Arguments.createMap()
                map.putString("credentialId", Base64.encodeToString(credential.credentialId, Base64.NO_WRAP))
                map.putString("userId", Base64.encodeToString(credential.userId, Base64.NO_WRAP))
                if (credential.aaguid != null) {
                    map.putString("aaguid", Base64.encodeToString(credential.aaguid, Base64.NO_WRAP))
                } else {
                    map.putNull("aaguid")
                }
                if (credential.backupEligible != null) {
                    map.putBoolean("backupEligible", credential.backupEligible!!)
                } else {
                    map.putNull("backupEligible")
                }
                promise.resolve(map)
            } catch (e: CredentialManagerPrfCoreException) {
                promise.reject(e.errorCode, e.message ?: e.defaultMessage)
            } catch (e: Exception) {
                promise.reject("ERR_PASSKEY", e.message ?: e.toString())
            }
        }
    }

    /** Check if PRF-capable passkeys are available on this device. */
    @ReactMethod
    fun isSupported(promise: Promise) {
        promise.resolve(CredentialManagerPrfCore.isSupported())
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

    companion object {
        const val NAME = "BreezSdkSparkPasskey"
    }
}
