package technology.breez.spark.passkey.core

import android.app.Activity
import android.content.Context
import android.content.pm.PackageManager
import android.content.pm.Signature
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
import org.json.JSONArray
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL
import java.net.URLEncoder
import java.security.MessageDigest
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
                @Suppress("UNUSED_VARIABLE")
                val ignored = registerCredential(activity, rpId, rpName, userName, userDisplayName)
                getAssertionWithPrf(activity, salt, rpId)
            }
        } catch (e: CredentialManagerPrfCoreException) {
            throw e
        } catch (e: Exception) {
            throw e.toCoreException()
        }
    }

    /**
     * Verify the app's package identity is listed by Google's Digital Asset
     * Links API for the given [rpId], with `get_login_creds` (WebAuthn /
     * passkey) permission.
     *
     * # Why this check exists
     *
     * Android's Credential Manager delegates domain verification to Google
     * Play Services, which caches assetlinks statements server-side. When
     * the `/.well-known/assetlinks.json` file on the RP domain doesn't list
     * your package (either because it was never added, or because Google's
     * cache is stale), subsequent WebAuthn calls fail with opaque
     * `GetCredentialException` / `CreateCredentialException` variants —
     * typically mapped to `CredentialNotFound` or a generic "cannot be
     * validated" error. Those are indistinguishable from "no credential
     * found" at the error layer.
     *
     * This check hits Google's public Digital Asset Links API directly:
     *
     *   `GET https://digitalassetlinks.googleapis.com/v1/statements:list`
     *      `?source.web.site=https://<rpId>`
     *      `&relation=delegate_permission/common.get_login_creds`
     *
     * and looks for an `android_app` statement matching this app's
     * package name **and** signing-certificate SHA-256 fingerprint. Both
     * must match — a package-only match would accept a MITM'd package
     * signed by a different key.
     *
     * # Return semantics
     *
     * - Match found → `DomainAssociationResult.Associated`
     * - Endpoint reachable, response parseable, no matching statement →
     *   `DomainAssociationResult.NotAssociated`
     * - Network error / timeout / non-200 / unparseable response →
     *   `DomainAssociationResult.Skipped` (caller proceeds with WebAuthn)
     *
     * @param context Any Context; used only to read
     *   [PackageManager.getPackageInfo] metadata on the running app.
     * @param rpId Relying Party ID (the domain hosting
     *   `.well-known/assetlinks.json`).
     * @param connectTimeoutMs HTTP connect timeout. Default 3000ms.
     * @param readTimeoutMs HTTP read timeout. Default 3000ms.
     */
    public suspend fun checkDomainAssociation(
        context: Context,
        rpId: String,
        connectTimeoutMs: Int = 3000,
        readTimeoutMs: Int = 3000,
    ): DomainAssociationResult = withContext(Dispatchers.IO) {
        val packageName = context.packageName
        val signingCertSha256 = try {
            computeSigningCertSha256(context, packageName)
        } catch (e: Exception) {
            return@withContext DomainAssociationResult.Skipped(
                "Could not read app signing certificate: ${e.message}"
            )
        } ?: return@withContext DomainAssociationResult.Skipped(
            "App has no signing certificate (unsigned debug build?)"
        )

        val encodedRpId = URLEncoder.encode(rpId, "UTF-8")
        val url = URL(
            "https://digitalassetlinks.googleapis.com/v1/statements:list" +
                "?source.web.site=https://$encodedRpId" +
                "&relation=delegate_permission/common.get_login_creds"
        )

        val responseJson = try {
            val connection = url.openConnection() as HttpURLConnection
            connection.connectTimeout = connectTimeoutMs
            connection.readTimeout = readTimeoutMs
            connection.requestMethod = "GET"
            try {
                if (connection.responseCode != 200) {
                    return@withContext DomainAssociationResult.Skipped(
                        "Digital Asset Links API returned HTTP ${connection.responseCode}"
                    )
                }
                connection.inputStream.bufferedReader().use { it.readText() }
            } finally {
                connection.disconnect()
            }
        } catch (e: Exception) {
            return@withContext DomainAssociationResult.Skipped(
                "Digital Asset Links API fetch failed: ${e.message}"
            )
        }

        val statements = try {
            JSONObject(responseJson).optJSONArray("statements") ?: JSONArray()
        } catch (e: Exception) {
            return@withContext DomainAssociationResult.Skipped(
                "Digital Asset Links API returned unparseable JSON: ${e.message}"
            )
        }

        // Response-format note: the `.well-known/assetlinks.json` FILE
        // format uses snake_case (`android_app`, `package_name`,
        // `sha256_cert_fingerprints` as an array). The Digital Asset
        // Links API response format is DIFFERENT — it's proto3 JSON
        // with camelCase field names and de-nests each fingerprint into
        // its own statement. A matching API statement looks like:
        //
        //   { "target": { "androidApp": {
        //       "packageName": "technology.breez.glow",
        //       "certificate": { "sha256Fingerprint": "AA:BB:..." }
        //     } } }
        //
        // Arrays in the file become multiple statements in the API. The
        // initial implementation used the file-format keys against the
        // API response and never found a match, blocking every Android
        // user. Verified live against
        // digitalassetlinks.googleapis.com/v1/statements:list.
        val listedFingerprints = mutableListOf<String>()
        for (i in 0 until statements.length()) {
            val stmt = statements.optJSONObject(i) ?: continue
            val target = stmt.optJSONObject("target") ?: continue
            val androidApp = target.optJSONObject("androidApp") ?: continue
            if (androidApp.optString("packageName") != packageName) continue
            val fingerprint = androidApp.optJSONObject("certificate")
                ?.optString("sha256Fingerprint") ?: continue
            if (fingerprint.isEmpty()) continue
            listedFingerprints.add(fingerprint)
            if (fingerprint.equals(signingCertSha256, ignoreCase = true)) {
                return@withContext DomainAssociationResult.Associated
            }
        }

        DomainAssociationResult.NotAssociated(
            source = "Google Digital Asset Links API",
            reason = "Package $packageName " +
                (if (listedFingerprints.isEmpty())
                    "has no android_app statement for https://$rpId " +
                        "(relation: delegate_permission/common.get_login_creds)."
                else
                    "is listed for https://$rpId but none of the statement " +
                        "fingerprints match this app's signing certificate " +
                        "$signingCertSha256. Listed: [${listedFingerprints.joinToString()}]."
                )
        )
    }

    /**
     * Compute the SHA-256 fingerprint of the app's signing certificate in
     * colon-separated uppercase hex (the format Google Digital Asset Links
     * uses). Returns null if the app has no signing certificate.
     *
     * Uses `PackageManager.GET_SIGNING_CERTIFICATES` on API 28+ (the
     * `CredentialManagerPrfCore` contract requires API 28+ anyway, so this
     * is always available in the contexts where this function runs).
     */
    @Suppress("DEPRECATION")
    private fun computeSigningCertSha256(context: Context, packageName: String): String? {
        val signatures: Array<Signature>? = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            val packageInfo = context.packageManager.getPackageInfo(
                packageName,
                PackageManager.GET_SIGNING_CERTIFICATES,
            )
            // Prefer apkContentsSigners (current signer), ignore past signers:
            // assetlinks.json matches only against the current signing cert.
            val signingInfo = packageInfo.signingInfo ?: return null
            if (signingInfo.hasMultipleSigners()) {
                signingInfo.apkContentsSigners
            } else {
                signingInfo.signingCertificateHistory
            }
        } else {
            // Pre-API 28 path — not reachable since CredentialManagerPrfCore
            // gates on API 28, but defensively included.
            val packageInfo = context.packageManager.getPackageInfo(
                packageName,
                PackageManager.GET_SIGNATURES,
            )
            packageInfo.signatures
        }

        val signature = signatures?.firstOrNull() ?: return null
        val digest = MessageDigest.getInstance("SHA-256").digest(signature.toByteArray())
        return digest.joinToString(":") { "%02X".format(it) }
    }

    /**
     * Register a new passkey without deriving a seed.
     *
     * Use this to separate credential creation from derivation in
     * multi-step onboarding flows. Triggers exactly one platform prompt.
     *
     * @param excludeCredentialIds Optional list of credential IDs to exclude.
     *   Pass previously created credential IDs to prevent the authenticator
     *   from creating a duplicate on the same device.
     * @return The credential ID of the newly created passkey.
     */
    public suspend fun createCredential(
        activity: Activity,
        rpId: String,
        rpName: String,
        userName: String,
        userDisplayName: String,
        excludeCredentialIds: List<ByteArray> = emptyList(),
    ): ByteArray = withContext(Dispatchers.Main) {
        try {
            registerCredential(activity, rpId, rpName, userName, userDisplayName, excludeCredentialIds)
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

        // Build request via JSONObject so integrator-provided strings (rpId)
        // are escaped correctly. Raw template interpolation would break on
        // rpId values containing quotes / backslashes / control chars and
        // surface as opaque JSONException deep in Credential Manager.
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
        excludeCredentialIds: List<ByteArray> = emptyList(),
    ): ByteArray {
        val credentialManager = CredentialManager.create(activity)
        val challenge = randomBase64Url(32)
        val userId = randomBase64Url(16)

        // Build request via JSONObject so integrator-provided strings
        // (rpId, rpName, userName, userDisplayName) are escaped correctly.
        // A perfectly reasonable display name like `Bob "B" Smith` or any
        // value containing a backslash / newline / apostrophe would break
        // raw template interpolation and surface as a confusing
        // JSONException deep in Credential Manager.
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
            if (excludeCredentialIds.isNotEmpty()) {
                put("excludeCredentials", JSONArray().apply {
                    for (credId in excludeCredentialIds) {
                        put(JSONObject().apply {
                            put("type", "public-key")
                            put("id", Base64.encodeToString(
                                credId,
                                Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING,
                            ))
                        })
                    }
                })
            }
            put("authenticatorSelection", JSONObject().apply {
                put("residentKey", "required")
                put("requireResidentKey", true)
                put("userVerification", "required")
            })
            put("extensions", JSONObject().apply {
                put("prf", JSONObject())
            })
        }.toString()

        val response = credentialManager.createCredential(
            activity,
            CreatePublicKeyCredentialRequest(requestJson),
        )

        val registrationJson = response.data.getString(
            "androidx.credentials.BUNDLE_KEY_REGISTRATION_RESPONSE_JSON"
        ) ?: throw CredentialManagerPrfCoreException(
            Kind.AuthenticationFailed,
            "No registration response",
        )
        val responseJson = JSONObject(registrationJson)
        val rawId = responseJson.optString("rawId", "")
        if (rawId.isEmpty()) {
            throw CredentialManagerPrfCoreException(
                Kind.AuthenticationFailed,
                "No credential ID in registration response",
            )
        }
        return Base64.decode(rawId, Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING)
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

/**
 * Result of a domain-association check. Mirrors the Rust
 * `DomainAssociation` enum one-to-one so the provider wrapper can map
 * directly without lossy conversions.
 */
public sealed class DomainAssociationResult {
    public object Associated : DomainAssociationResult()
    public data class NotAssociated(
        public val source: String,
        public val reason: String,
    ) : DomainAssociationResult()
    public data class Skipped(public val reason: String) : DomainAssociationResult()
}
