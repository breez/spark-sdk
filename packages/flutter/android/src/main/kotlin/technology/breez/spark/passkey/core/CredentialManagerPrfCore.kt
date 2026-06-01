package technology.breez.spark.passkey.core

import android.app.Activity
import android.content.Context
import android.content.pm.PackageManager
import android.content.pm.Signature
import android.os.Build
import android.util.Base64
import android.util.Log
import androidx.credentials.CreatePublicKeyCredentialRequest
import androidx.credentials.CredentialManager
import androidx.credentials.GetCredentialRequest
import androidx.credentials.GetPublicKeyCredentialOption
import androidx.credentials.exceptions.CreateCredentialCancellationException
import androidx.credentials.exceptions.CreateCredentialException
import androidx.credentials.exceptions.GetCredentialCancellationException
import androidx.credentials.exceptions.GetCredentialException
import androidx.credentials.exceptions.NoCredentialException
import androidx.credentials.exceptions.domerrors.InvalidStateError
import androidx.credentials.exceptions.publickeycredential.CreatePublicKeyCredentialDomException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
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
// Canonical copy:
//   crates/breez-sdk/bindings/langs/shared/android-passkey/src/main/kotlin/
//     technology/breez/spark/passkey/core/CredentialManagerPrfCore.kt
//
// Shared into four Android artifacts via gradle `srcDirs`
// (bindings-android + breez-sdk-spark-kmp) and `cargo xtask
// sync-passkey-core` (packages/flutter + packages/react-native). Never
// hand-edit a copy: edit this file, run the xtask, commit the diff. CI
// fails if a copy drifts.
// =====================================================================

/**
 * Framework-agnostic helper wrapping AndroidX Credential Manager + the
 * WebAuthn PRF extension for passkey-based seed derivation.
 *
 * Wrappers (UniFFI `PasskeyProvider`, Flutter MethodChannel, React Native
 * module) delegate here and add only framework glue: error mapping,
 * activity retrieval, call-site boilerplate. Throws
 * [CredentialManagerPrfCoreException] for every well-known failure so
 * wrappers can switch on [Kind] without touching Credential Manager
 * internals.
 */

/**
 * A passkey credential from a register or sign-in ceremony.
 * [credentialId] is always set. The remaining fields are populated on
 * registration and null on sign-in (an assertion carries no
 * attestation). [aaguid] is the 16-byte Authenticator Attestation GUID
 * (provider identifier), unverified attestation: a display hint only,
 * never a trust decision. [backupEligible] is the BE flag (can the
 * credential sync across devices). [userId] is the core-minted WebAuthn
 * user handle, never host-supplied. Persist [credentialId] to drive
 * `excludeCredentials` / `allowCredentials` on later calls.
 */
public data class PasskeyCredential(
    public val credentialId: ByteArray,
    public val userId: ByteArray?,
    public val aaguid: ByteArray?,
    public val backupEligible: Boolean?,
)

private const val CORE_TAG = "PasskeyPrfCore"

// =====================================================================
// Post-create grace
// =====================================================================

/**
 * A newly-registered passkey is briefly not ready for the immediate
 * post-create assertion: Credential Manager can drop `prf.second` from a
 * dual-salt assertion (forcing a second single-salt prompt) or surface
 * the credential as not yet discoverable in the picker. Holding the next
 * derive up to [DEFAULT_DURATION_MS] lets the OS finish indexing.
 *
 * Mirrors iOS `PostCreateGraceTracker`; an instance lives inside
 * [CredentialManagerPrfCore] so every consumer that holds onto a single
 * core (e.g. `PasskeyProvider` for UniFFI / KMM consumers) inherits the
 * grace without per-wrapper plumbing.
 */
public class PostCreateGraceTracker {
    private val mutex = Mutex()
    @Volatile private var deadlineMs: Long = 0L

    public suspend fun arm(durationMs: Long = DEFAULT_DURATION_MS) {
        mutex.withLock {
            deadlineMs = System.currentTimeMillis() + durationMs
        }
    }

    public suspend fun consume() {
        val waitMs = mutex.withLock {
            val remaining = deadlineMs - System.currentTimeMillis()
            deadlineMs = 0L
            if (remaining > 0L) remaining else 0L
        }
        if (waitMs > 0L) {
            delay(waitMs)
        }
    }

    public companion object {
        public const val DEFAULT_DURATION_MS: Long = 800L
    }
}

/**
 * Platform PRF engine: holds the relying-party identity for one
 * provider's lifetime, exposing `deriveSeeds`, `register`,
 * `checkDomainAssociation`, and `isSupported`. Per-call methods take
 * only per-ceremony arguments; everything else is fixed at
 * construction. Each consumer maps the typed
 * [CredentialManagerPrfCoreException] onto its own error surface.
 *
 * @param activityProvider Resolves the current top Activity lazily on
 *   each call, so a stale instance is never held across rotation.
 * @param graceTracker Post-create grace state. Defaults to a fresh
 *   tracker per core; consumers that pool cores across the same wrapper
 *   instance can share one tracker explicitly.
 * @param postCreateGraceMs How long [register] arms the grace for.
 *   Defaults to [PostCreateGraceTracker.DEFAULT_DURATION_MS].
 */
public class CredentialManagerPrfCore(
    private val rpId: String,
    private val rpName: String,
    private val userName: String,
    private val userDisplayName: String,
    private val activityProvider: () -> Activity,
    private val graceTracker: PostCreateGraceTracker = PostCreateGraceTracker(),
    private val postCreateGraceMs: Long = PostCreateGraceTracker.DEFAULT_DURATION_MS,
) {

    public companion object {
        /** Default Relying Party ID for cross-platform credential sharing. */
        public const val DEFAULT_RP_ID: String = "keys.breez.technology"

        /**
         * `true` if the OS version could support passkey PRF (API 28+).
         * Checks the platform version only, not whether a credential
         * provider is installed or biometrics are enrolled.
         */
        public fun isSupported(): Boolean =
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.P

        /** Lazily initialised; first-use entropy gathering can dominate the cold path. */
        private val secureRandom: SecureRandom by lazy { SecureRandom() }

        /**
         * Cached `CredentialManager`, held process-wide against the
         * application context (lifecycle-safe across rotation) so per-call
         * core instances don't each re-allocate it.
         */
        @Volatile
        private var cachedCredentialManager: CredentialManager? = null

        private fun credentialManager(activity: Activity): CredentialManager =
            cachedCredentialManager ?: synchronized(this) {
                cachedCredentialManager ?: CredentialManager.create(activity.applicationContext).also {
                    cachedCredentialManager = it
                }
            }

        private fun encodeBase64Url(bytes: ByteArray): String =
            Base64.encodeToString(bytes, Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING)

        private fun decodeBase64Url(s: String): ByteArray =
            Base64.decode(s, Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING)

        /**
         * Decode URL-safe base64 from a provider response, logging +
         * returning null on malformed input (a provider/protocol fault).
         */
        private fun decodeBase64UrlOrNull(s: String, what: String): ByteArray? =
            try {
                decodeBase64Url(s)
            } catch (e: IllegalArgumentException) {
                Log.w(CORE_TAG, "Malformed base64url in provider response ($what)", e)
                null
            }

        private fun randomBase64Url(byteCount: Int): String {
            val bytes = ByteArray(byteCount)
            secureRandom.nextBytes(bytes)
            return encodeBase64Url(bytes)
        }
    }

    /**
     * Derive one 32-byte PRF output per salt in as few authenticator
     * ceremonies as the platform supports: salts are walked in pairs
     * (one dual-salt assertion each via `prf.eval.first`/`.second`),
     * and an authenticator that drops `second` is recovered with a
     * single-salt re-assert. When no credential exists yet and
     * [autoRegister] is set, the first miss registers a passkey and
     * retries. Output ordering matches input ordering.
     *
     * Prompt count for the common 2-salt setup: 1 on a conformant
     * authenticator. Worst case is 3 (assert-miss, register, then a
     * dual-assert that drops `prf.second` plus a single-salt recover) on
     * a provider that both lacks a credential and drops `second`. Register
     * runs at most once per call, so prompts never grow unbounded.
     */
    public suspend fun deriveSeeds(
        salts: List<String>,
        autoRegister: Boolean = true,
        allowCredentials: List<ByteArray> = emptyList(),
        preferImmediatelyAvailableCredentials: Boolean = true,
    ): PrfDerivation = withContext(Dispatchers.Main) {
        // Wait out the post-create grace so an immediate derive doesn't
        // race the credential's PRF-readiness window (see grace tracker).
        graceTracker.consume()
        // Pinned to the first asserted credential after the first chunk so
        // every salt in this call derives from one passkey.
        var allow = allowCredentials
        if (salts.isEmpty()) return@withContext PrfDerivation(emptyList(), null)

        // One assertion for 1-2 salts, registering + retrying once on no
        // credential. Returns one output per salt the authenticator
        // evaluated (a dropped `second` yields one) plus the asserted
        // credential ID. After the first chunk the caller pins `allow` to
        // it, so every chunk resolves to the same credential.
        suspend fun assertChunk(chunk: List<String>): Pair<List<ByteArray>, ByteArray?> =
            try {
                assertPrf(chunk, allow, preferImmediatelyAvailableCredentials)
            } catch (e: NoCredentialException) {
                if (!autoRegister) {
                    throw CredentialManagerPrfCoreException(
                        Kind.CredentialNotFound,
                        e.message ?: "",
                    )
                }
                register()
                // Retry once. A second miss (e.g. user deleted the pinned
                // credential in Settings) escapes as CredentialNotFound for
                // hosts to treat as deletion recovery.
                assertPrf(chunk, allow, preferImmediatelyAvailableCredentials)
            }

        val startedAtMs = System.currentTimeMillis()
        try {
            val output = ArrayList<ByteArray>(salts.size)
            // Asserted credential ID, returned so the binding layer can
            // surface it on `SignInResponse.credential_id`.
            var observedCredentialId: ByteArray? = null
            var idx = 0
            while (idx < salts.size) {
                if (idx + 1 < salts.size) {
                    val (outputs, credId) = assertChunk(listOf(salts[idx], salts[idx + 1]))
                    observedCredentialId = credId
                    // Pin every later assertion in this call to the credential
                    // the first one resolved to, so all salts derive from one
                    // passkey even when a chunk splits (dropped `second`, 3+ salts).
                    credId?.let { allow = listOf(it) }
                    output.add(outputs[0])
                    if (outputs.size > 1) {
                        output.add(outputs[1])
                    } else {
                        // Authenticator dropped `second`: single-salt recover,
                        // pinned to the same credential as the first output.
                        val (recovered, _) = assertChunk(listOf(salts[idx + 1]))
                        output.add(recovered[0])
                    }
                    idx += 2
                } else {
                    val (single, credId) = assertChunk(listOf(salts[idx]))
                    observedCredentialId = credId
                    credId?.let { allow = listOf(it) }
                    output.add(single[0])
                    idx += 1
                }
            }
            PrfDerivation(output, observedCredentialId)
        } catch (e: CredentialManagerPrfCoreException) {
            throw e
        } catch (e: Exception) {
            throw e.toCoreException(System.currentTimeMillis() - startedAtMs)
        }
    }

    /**
     * Verify the app is listed by Google's Digital Asset Links API for
     * [rpId] with `get_login_creds` permission, queried up front because
     * Credential Manager otherwise surfaces a stale or missing
     * assetlinks.json as an opaque `CredentialNotFound` / "cannot be
     * validated" error indistinguishable from "no credential found":
     *
     *   `GET https://digitalassetlinks.googleapis.com/v1/statements:list`
     *      `?source.web.site=https://<rpId>`
     *      `&relation=delegate_permission/common.get_login_creds`
     *
     * Requires an `android_app` statement matching this app's package name
     * AND signing-cert SHA-256: a package-only match would accept a MITM'd
     * package signed by a different key.
     *
     * Returns `Associated` on a match, `NotAssociated` when the endpoint
     * is reachable but no statement matches, `Skipped` on any
     * network/timeout/non-200/parse failure (caller proceeds anyway).
     *
     * @param connectTimeoutMs HTTP connect timeout. Default 3000ms.
     * @param readTimeoutMs HTTP read timeout. Default 3000ms.
     */
    public suspend fun checkDomainAssociation(
        connectTimeoutMs: Int = 3000,
        readTimeoutMs: Int = 3000,
    ): DomainAssociationResult = withContext(Dispatchers.IO) {
        val context = activityProvider().applicationContext
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

        // The API response format differs from the assetlinks.json FILE:
        // it's proto3 JSON with camelCase keys and de-nests each
        // fingerprint into its own statement (file-format snake_case keys
        // never match). A matching statement looks like:
        //
        //   { "target": { "androidApp": {
        //       "packageName": "technology.breez.glow",
        //       "certificate": { "sha256Fingerprint": "AA:BB:..." }
        //     } } }
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
     * SHA-256 of the app's signing certificate in colon-separated
     * uppercase hex (the format Digital Asset Links uses). Null if the app
     * has no signing certificate. Uses `GET_SIGNING_CERTIFICATES`, always
     * available given the API 28+ contract.
     */
    @Suppress("DEPRECATION")
    private fun computeSigningCertSha256(context: Context, packageName: String): String? {
        val signatures: Array<Signature>? = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            val packageInfo = context.packageManager.getPackageInfo(
                packageName,
                PackageManager.GET_SIGNING_CERTIFICATES,
            )
            // Current signer only: assetlinks.json matches the current
            // signing cert, not past signers.
            val signingInfo = packageInfo.signingInfo ?: return null
            if (signingInfo.hasMultipleSigners()) {
                signingInfo.apkContentsSigners
            } else {
                signingInfo.signingCertificateHistory
            }
        } else {
            // Defensive only: the API 28+ contract makes this unreachable.
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

    // ------------------------------------------------------------------
    // Private
    // ------------------------------------------------------------------

    /**
     * Run one assertion ceremony for [salts] (1 or 2): build the WebAuthn
     * request (cross-device hybrid suppressed unless
     * [preferImmediatelyAvailableCredentials] is false), evaluate PRF, and
     * return one 32-byte output per salt the authenticator evaluated plus
     * the asserted credential ID. A dropped `results.second` yields a
     * single-element list.
     */
    private suspend fun assertPrf(
        salts: List<String>,
        allowCredentials: List<ByteArray>,
        preferImmediatelyAvailableCredentials: Boolean,
    ): Pair<List<ByteArray>, ByteArray?> {
        val activity = activityProvider()
        // JSONObject (not string interpolation) so integrator-supplied rpId
        // is escaped, not breaking on quotes/backslashes inside Credential
        // Manager.
        val requestJson = JSONObject().apply {
            put("challenge", randomBase64Url(32))
            put("rpId", rpId)
            put("allowCredentials", JSONArray().apply {
                for (credId in allowCredentials) {
                    put(JSONObject().apply {
                        put("type", "public-key")
                        put("id", encodeBase64Url(credId))
                    })
                }
            })
            put("userVerification", "required")
            put("extensions", JSONObject().apply {
                put("prf", JSONObject().apply {
                    put("eval", JSONObject().apply {
                        put("first", encodeBase64Url(salts[0].toByteArray(Charsets.UTF_8)))
                        if (salts.size > 1) {
                            put("second", encodeBase64Url(salts[1].toByteArray(Charsets.UTF_8)))
                        }
                    })
                })
            })
        }.toString()

        // `preferImmediatelyAvailableCredentials = true` suppresses the
        // cross-device QR sheet so a missing local credential surfaces
        // as NoCredentialException; `false` opts back into the picker.
        val request = GetCredentialRequest.Builder()
            .addCredentialOption(GetPublicKeyCredentialOption(requestJson))
            .setPreferImmediatelyAvailableCredentials(preferImmediatelyAvailableCredentials)
            .build()
        val response = credentialManager(activity).getCredential(activity, request)

        val authResponseJson = response.credential.data.getString(
            "androidx.credentials.BUNDLE_KEY_AUTHENTICATION_RESPONSE_JSON",
        ) ?: throw CredentialManagerPrfCoreException(
            Kind.AuthenticationFailed, "No credential response",
        )
        val responseJson = JSONObject(authResponseJson)

        // Asserted credential ID, returned inline for the binding layer.
        val credentialId = responseJson.optString("rawId", "").takeIf { it.isNotEmpty() }
            ?.let { decodeBase64UrlOrNull(it, "assertion rawId") }

        val extensions = responseJson.optJSONObject("clientExtensionResults")
            ?: throw CredentialManagerPrfCoreException(Kind.PrfNotSupported)
        val prf = extensions.optJSONObject("prf")
            ?: throw CredentialManagerPrfCoreException(Kind.PrfNotSupported)
        val results = prf.optJSONObject("results")
            ?: throw CredentialManagerPrfCoreException(Kind.PrfEvaluationFailed, "no results")

        val first = results.optString("first")
        if (first.isNullOrEmpty()) {
            throw CredentialManagerPrfCoreException(Kind.PrfEvaluationFailed, "empty result")
        }
        val out = ArrayList<ByteArray>(salts.size)
        out.add(decodeBase64Url(first))
        // Older Credential Manager implementations silently drop
        // saltInput2; omit it so the caller re-asserts single-salt.
        if (salts.size > 1) {
            results.optString("second", "").takeIf { it.isNotEmpty() }
                ?.let { decodeBase64UrlOrNull(it, "prf.results.second") }
                ?.let { out.add(it) }
        }
        return Pair(out, credentialId)
    }

    /**
     * Register a new passkey (one platform prompt, no seed derivation).
     * [excludeCredentials] is passed straight through so the platform
     * refuses to register a credential already on the device (raising
     * `CredentialAlreadyExists`, which callers route to sign-in). Returns
     * the credential ID plus AAGUID / backup-eligibility from the
     * attestation (null when unparseable).
     */
    public suspend fun register(
        excludeCredentials: List<ByteArray> = emptyList(),
    ): PasskeyCredential = withContext(Dispatchers.Main) {
        val startedAtMs = System.currentTimeMillis()
        try {
            val activity = activityProvider()
            // Mint a fresh random 16-byte user handle per call (never
            // host-supplied); the JSON base64url string and the raw
            // PasskeyCredential.userId bytes share this buffer.
            val userIdBytes = ByteArray(16).also { secureRandom.nextBytes(it) }

            // JSONObject (not string interpolation) so integrator strings
            // are escaped, not breaking on quotes/backslashes inside
            // Credential Manager.
            val requestJson = JSONObject().apply {
                put("challenge", randomBase64Url(32))
                put("rp", JSONObject().apply {
                    put("id", rpId)
                    put("name", rpName)
                })
                put("user", JSONObject().apply {
                    put("id", encodeBase64Url(userIdBytes))
                    put("name", userName)
                    put("displayName", userDisplayName)
                })
                put("pubKeyCredParams", JSONArray().apply {
                    put(JSONObject().apply { put("type", "public-key"); put("alg", -7) })
                    put(JSONObject().apply { put("type", "public-key"); put("alg", -257) })
                })
                if (excludeCredentials.isNotEmpty()) {
                    put("excludeCredentials", JSONArray().apply {
                        for (credId in excludeCredentials) {
                            put(JSONObject().apply {
                                put("type", "public-key")
                                put("id", encodeBase64Url(credId))
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

            val response = credentialManager(activity).createCredential(
                activity,
                CreatePublicKeyCredentialRequest(requestJson),
            )
            val registrationJson = response.data.getString(
                "androidx.credentials.BUNDLE_KEY_REGISTRATION_RESPONSE_JSON"
            ) ?: throw CredentialManagerPrfCoreException(
                Kind.AuthenticationFailed, "No registration response",
            )
            val responseJson = JSONObject(registrationJson)
            val rawId = responseJson.optString("rawId", "")
            if (rawId.isEmpty()) {
                throw CredentialManagerPrfCoreException(
                    Kind.AuthenticationFailed, "No credential ID in registration response",
                )
            }
            val credentialId = decodeBase64Url(rawId)
            var aaguid: ByteArray? = null
            var backupEligible: Boolean? = null
            val attestationB64 = responseJson.optJSONObject("response")?.optString("attestationObject", "") ?: ""
            if (attestationB64.isNotEmpty()) {
                extractRegistrationMetadata(decodeBase64Url(attestationB64))?.let { meta ->
                    aaguid = meta.first
                    backupEligible = meta.second
                }
            }
            // Arm the post-create grace so the immediate derive doesn't
            // race the credential's PRF-readiness window (see grace tracker).
            graceTracker.arm(postCreateGraceMs)
            PasskeyCredential(credentialId, userIdBytes, aaguid, backupEligible)
        } catch (e: CredentialManagerPrfCoreException) {
            throw e
        } catch (e: Exception) {
            throw e.toCoreException(System.currentTimeMillis() - startedAtMs)
        }
    }

    /**
     * Extract AAGUID + BE flag from the attestation object's authenticator
     * data via byte-pattern search for the "authData" CBOR key. Returns
     * null when not found or too short.
     *
     * authData layout when AT flag is set (always on a successful create):
     *   [32]      flags (UP=0, UV=2, BE=3, BS=4, AT=6)
     *   [37..53)  AAGUID (16 bytes)
     */
    private fun extractRegistrationMetadata(attestation: ByteArray): Pair<ByteArray, Boolean>? {
        // CBOR text key "authData": 0x68 = major type 3 (text) + length 8.
        val key = byteArrayOf(0x68, 0x61, 0x75, 0x74, 0x68, 0x44, 0x61, 0x74, 0x61)
        if (attestation.size < key.size) return null
        var keyEnd = -1
        for (i in 0..(attestation.size - key.size)) {
            var match = true
            for (j in key.indices) {
                if (attestation[i + j] != key[j]) { match = false; break }
            }
            if (match) { keyEnd = i + key.size; break }
        }
        if (keyEnd < 0 || keyEnd >= attestation.size) return null

        val header = attestation[keyEnd].toInt() and 0xff
        if (header shr 5 != 2) return null
        val minor = header and 0x1f
        val length: Int
        val dataStart: Int
        when {
            minor < 24 -> { length = minor; dataStart = keyEnd + 1 }
            minor == 24 -> {
                if (keyEnd + 1 >= attestation.size) return null
                length = attestation[keyEnd + 1].toInt() and 0xff
                dataStart = keyEnd + 2
            }
            minor == 25 -> {
                if (keyEnd + 2 >= attestation.size) return null
                length = ((attestation[keyEnd + 1].toInt() and 0xff) shl 8) or
                    (attestation[keyEnd + 2].toInt() and 0xff)
                dataStart = keyEnd + 3
            }
            minor == 26 -> {
                if (keyEnd + 4 >= attestation.size) return null
                length = ((attestation[keyEnd + 1].toInt() and 0xff) shl 24) or
                    ((attestation[keyEnd + 2].toInt() and 0xff) shl 16) or
                    ((attestation[keyEnd + 3].toInt() and 0xff) shl 8) or
                    (attestation[keyEnd + 4].toInt() and 0xff)
                dataStart = keyEnd + 5
            }
            else -> return null
        }
        if (dataStart + length > attestation.size || length < 53) return null
        val flags = attestation[dataStart + 32].toInt() and 0xff
        if (flags and 0x40 == 0) return null
        val backupEligible = flags and 0x08 != 0
        val aaguid = attestation.copyOfRange(dataStart + 37, dataStart + 53)
        return Pair(aaguid, backupEligible)
    }

    /**
     * Map a Credential Manager exception into the typed core exception.
     * `elapsedMs` is the ceremony's wall-clock duration: a cancellation
     * beyond ~55s reclassifies to [Kind.UserTimedOut] (biometric
     * inactivity timeout) instead of [Kind.UserCancelled]. Pass `null`
     * when timing is unknown to default to `UserCancelled`.
     */
    private fun Exception.toCoreException(
        elapsedMs: Long? = null,
    ): CredentialManagerPrfCoreException = when (this) {
        is GetCredentialCancellationException,
        is CreateCredentialCancellationException ->
            CredentialManagerPrfCoreException(classifyCancellation(elapsedMs), cause = this)

        is NoCredentialException ->
            CredentialManagerPrfCoreException(
                Kind.CredentialNotFound,
                message ?: "No matching credential on this device",
                this,
            )

        // Credential Manager wraps WebAuthn DOM errors here with the
        // spec-level error in `domError`; an InvalidStateError is the
        // duplicate-prevention check, routed to sign-in. Must precede the
        // generic CreateCredentialException case (this is a subclass).
        is CreatePublicKeyCredentialDomException ->
            if (domError is InvalidStateError) {
                CredentialManagerPrfCoreException(
                    Kind.CredentialAlreadyExists,
                    message ?: "Credential already registered for this RP",
                    this,
                )
            } else {
                CredentialManagerPrfCoreException(
                    Kind.AuthenticationFailed,
                    "${type}: ${message ?: toString()}",
                    this,
                )
            }

        is GetCredentialException ->
            CredentialManagerPrfCoreException(
                Kind.AuthenticationFailed,
                "${type}: ${message ?: toString()}",
                this,
            )

        is CreateCredentialException ->
            CredentialManagerPrfCoreException(
                Kind.AuthenticationFailed,
                "${type}: ${message ?: toString()}",
                this,
            )

        else -> {
            val raw = message ?: toString()
            // Actionable hints for common misconfigurations.
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
            CredentialManagerPrfCoreException(Kind.Generic, hint, this)
        }
    }

    /**
     * Tell a user-dismissed prompt from the biometric inactivity timeout:
     * AndroidX surfaces both as the same `*CancellationException`, so the
     * ceremony's elapsed time is the only signal. The prompt is torn down
     * at ~55s+, so anything beyond that is [Kind.UserTimedOut].
     */
    private fun classifyCancellation(elapsedMs: Long?): Kind {
        if (elapsedMs != null && elapsedMs >= 55_000L) {
            return Kind.UserTimedOut
        }
        return Kind.UserCancelled
    }

    /** Discriminator for [CredentialManagerPrfCoreException]. */
    public enum class Kind {
        /** The authenticator does not support the WebAuthn PRF extension. */
        PrfNotSupported,
        /** The user dismissed the passkey prompt or cancelled the operation. */
        UserCancelled,
        /**
         * The biometric prompt timed out without user interaction (~55s+).
         * Distinct from [UserCancelled] (active dismissal): hosts may
         * auto-retry or re-prompt rather than treating it as abandonment.
         */
        UserTimedOut,
        /** No credential exists for the RP and auto-registration was not attempted. */
        CredentialNotFound,
        /** Credential Manager reported an authentication / registration failure. */
        AuthenticationFailed,
        /** PRF evaluation produced an empty or malformed response. */
        PrfEvaluationFailed,
        /**
         * Platform or app configuration error (e.g. missing
         * assetlinks.json, misconfigured RP ID). Reserved for parity with
         * the cross-platform taxonomy; Credential Manager reports these as
         * `AuthenticationFailed`, so this kind is never emitted here.
         */
        Configuration,
        /**
         * Registration refused because a credential in `excludeCredentials`
         * is already on the authenticator. The duplicate-prevention check,
         * surfaced as a typed kind so callers route to sign-in.
         */
        CredentialAlreadyExists,
        /** Any other unexpected error: message contains the details. */
        Generic,
    }
}

/**
 * Result of [CredentialManagerPrfCore.deriveSeeds]: one 32-byte PRF
 * output per salt (input order) plus the asserted credential ID.
 * [credentialId] is `null` when no assertion ran (empty `salts`).
 */
public data class PrfDerivation(
    public val seeds: List<ByteArray>,
    public val credentialId: ByteArray?,
)

/**
 * Typed exception thrown by [CredentialManagerPrfCore]. Wrappers should
 * switch on [kind] to map to their own framework-specific error type.
 */
public class CredentialManagerPrfCoreException(
    public val kind: CredentialManagerPrfCore.Kind,
    message: String? = null,
    cause: Throwable? = null,
) : Exception(message, cause)

/**
 * Result of a domain-association check. Mirrors the Rust
 * `DomainAssociation` enum one-to-one so the wrapper maps it losslessly.
 */
public sealed class DomainAssociationResult {
    public object Associated : DomainAssociationResult()
    public data class NotAssociated(
        public val source: String,
        public val reason: String,
    ) : DomainAssociationResult()
    public data class Skipped(public val reason: String) : DomainAssociationResult()
}
