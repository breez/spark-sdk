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
import androidx.credentials.exceptions.domerrors.InvalidStateError
import androidx.credentials.exceptions.publickeycredential.CreatePublicKeyCredentialDomException
import technology.breez.spark.passkey.KnownCredentialsStore
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
 * Wrappers (the UniFFI `PasskeyProvider`, the Flutter MethodChannel
 * plugin, the React Native native module) delegate to this object and
 * only provide framework-specific glue on top: error mapping, activity
 * retrieval, and call-site boilerplate.
 *
 * Throws [CredentialManagerPrfCoreException] for every well-known failure
 * mode so wrappers can switch on [Kind] without peeking at WebAuthn or
 * Credential Manager internals.
 */

/**
 * Authenticator data captured at registration. [aaguid] is the 16-byte
 * Authenticator Attestation GUID (provider identifier); [backupEligible]
 * is the BE flag indicating whether the credential can sync across
 * devices. Both are null when the attestation can't be parsed. AAGUID is
 * unverified attestation: display hint only, never a trust decision.
 */
public data class RegisteredCredential(
    public val credentialId: ByteArray,
    public val aaguid: ByteArray?,
    public val backupEligible: Boolean?,
)

/**
 * Per-call shaping options for [CredentialManagerPrfCore.deriveSeedsOrRegister].
 * Lets the upstream callers (UniFFI Kotlin `PasskeyProvider`, Flutter
 * plugin, React Native module) override the per-instance defaults on a
 * per-ceremony basis without reconstructing the core.
 */
public data class DeriveSeedsOptions(
    /**
     * Per-call assertion allow-list. When non-empty, this list overrides
     * any caller-supplied default for the duration of the ceremony.
     * Empty defers to the legacy positional `allowCredentialIds`
     * parameter (for back-compat).
     */
    public val allowCredentialIds: List<ByteArray> = emptyList(),
    /**
     * Per-call control over [GetCredentialRequest.Builder.setPreferImmediatelyAvailableCredentials].
     * `null` keeps the historical default (`true`); `false` opts back
     * into the cross-device hybrid sheet.
     */
    public val preferImmediatelyAvailableCredentials: Boolean? = null,
)

public object CredentialManagerPrfCore {

    /** Default Relying Party ID for cross-platform credential sharing. */
    public const val DEFAULT_RP_ID: String = "keys.breez.technology"

    /** Default Relying Party display name shown during passkey registration. */
    public const val DEFAULT_RP_NAME: String = "Breez SDK"

    /** Lazily initialised; first-use entropy gathering can dominate the cold path. */
    private val secureRandom: SecureRandom by lazy { SecureRandom() }

    /**
     * Cached `CredentialManager`. The factory is cheap but each call
     * still allocates; held as a singleton against the application
     * context (lifecycle-safe across activity rotation).
     */
    @Volatile
    private var cachedCredentialManager: CredentialManager? = null

    private fun credentialManager(activity: Activity): CredentialManager =
        cachedCredentialManager ?: synchronized(this) {
            cachedCredentialManager ?: CredentialManager.create(activity.applicationContext).also {
                cachedCredentialManager = it
            }
        }

    /**
     * Returns `true` if this device's OS version could support passkey PRF.
     *
     * PRF extension support requires API 28+ (Android 9+) via Google Play
     * Services. The Jetpack Credential Manager library handles backward
     * compatibility automatically from there. This check does NOT verify
     * that a credential provider is actually installed or that biometrics
     * are enrolled, only the platform version.
     */
    public fun isSupported(): Boolean =
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.P

    /**
     * Authenticate the user via passkey PRF and return the 32-byte seed.
     *
     * If no credential exists for [rpId], auto-register a new one and
     * retry the assertion. Switches to `Dispatchers.Main` internally so
     * callers may invoke from any coroutine context.
     *
     * @param allowCredentialIds When non-empty, restricts assertion to one
     *   of the listed credential IDs. The Credential Manager refuses any
     *   other credential for this RP. Use this to bind sign-in to a
     *   specific passkey the caller has registered, instead of letting
     *   the platform pick any sibling credential that happens to share
     *   the RP. Critical for deterministic seed derivation when multiple
     *   credentials might exist for the same RP.
     * @param onAssertionCredentialId Optional callback invoked with the
     *   credential ID of every successful assertion. Hosts can use this
     *   to record which credential was used and populate
     *   [allowCredentialIds] / `excludeCredentialIds` on subsequent
     *   requests, e.g. migrating users whose passkey predates the host's
     *   own credential-ID tracking. Best-effort: failures inside the
     *   callback do not block the seed return.
     *
     * @throws CredentialManagerPrfCoreException for every handled error;
     *   wrappers should catch and remap by [CredentialManagerPrfCoreException.kind].
     */
    private suspend fun deriveSeedOrRegister(
        activity: Activity,
        salt: String,
        rpId: String,
        rpName: String,
        userName: String,
        userDisplayName: String,
        autoRegister: Boolean = true,
        allowCredentialIds: List<ByteArray> = emptyList(),
        preferImmediatelyAvailableCredentials: Boolean = true,
        onAssertionCredentialId: ((ByteArray) -> Unit)? = null,
    ): ByteArray = withContext(Dispatchers.Main) {
        val startedAtMs = System.currentTimeMillis()
        try {
            try {
                getAssertionWithPrf(
                    activity, salt, rpId, allowCredentialIds,
                    preferImmediatelyAvailableCredentials, onAssertionCredentialId,
                )
            } catch (e: NoCredentialException) {
                if (!autoRegister) {
                    throw CredentialManagerPrfCoreException(Kind.CredentialNotFound, e.message)
                }
                @Suppress("UNUSED_VARIABLE")
                val ignored = registerCredential(activity, rpId, rpName, userName, userDisplayName)
                // Retry with the same allowCredentialIds. If the caller
                // supplied a non-empty list and the existing credential was
                // genuinely missing (e.g. user deleted it from Settings),
                // the just-registered ID won't be in the list and the
                // retry will fail with CredentialNotFound. Hosts handle
                // that as a deletion-recovery signal: clear the registry
                // and route to onboarding. Mirrors iOS behavior.
                getAssertionWithPrf(
                    activity, salt, rpId, allowCredentialIds,
                    preferImmediatelyAvailableCredentials, onAssertionCredentialId,
                )
            }
        } catch (e: CredentialManagerPrfCoreException) {
            throw e
        } catch (e: Exception) {
            throw e.toCoreException(System.currentTimeMillis() - startedAtMs)
        }
    }

    /**
     * Bulk-derive multiple PRF outputs in as few authenticator
     * ceremonies as the platform supports.
     *
     * Uses the WebAuthn PRF dual-salt fast path where available: a
     * single assertion returns two PRF outputs via `prf.eval.first` and
     * `prf.eval.second`. Authenticators that silently drop the second
     * salt cause the cached verdict to flip to `false`, after which
     * subsequent calls fall back to sequential single-salt assertions
     * (matching the prior loop behavior).
     *
     * Salt count semantics:
     * - 0 salts: empty result, no prompt.
     * - 1 salt: equivalent to [deriveSeedOrRegister] (1 prompt).
     * - 2 salts: 1 prompt on supported authenticators, 2 on others.
     * - 3+ salts: pairs are batched. Trailing odd salt uses single-salt.
     *
     * Output ordering matches input ordering.
     *
     * @param allowCredentialIds applied to every assertion in the batch.
     * @param onAssertionCredentialId invoked once per assertion that
     *   returns a credential ID. With dual-salt support this fires
     *   once for the pair; without, it fires per individual assertion.
     */
    public suspend fun deriveSeedsOrRegister(
        activity: Activity,
        salts: List<String>,
        rpId: String,
        rpName: String,
        userName: String,
        userDisplayName: String,
        autoRegister: Boolean = true,
        allowCredentialIds: List<ByteArray> = emptyList(),
        onAssertionCredentialId: ((ByteArray) -> Unit)? = null,
        options: DeriveSeedsOptions = DeriveSeedsOptions(),
    ): List<ByteArray> = withContext(Dispatchers.Main) {
        // Per-call options win over the legacy positional
        // `allowCredentialIds` when non-empty. The positional parameter
        // remains for back-compat with older call sites.
        val effectiveAllow = if (options.allowCredentialIds.isNotEmpty()) {
            options.allowCredentialIds
        } else {
            allowCredentialIds
        }
        val preferImmediate = options.preferImmediatelyAvailableCredentials ?: true
        if (salts.isEmpty()) {
            return@withContext emptyList()
        }
        if (salts.size == 1) {
            return@withContext listOf(
                deriveSeedOrRegister(
                    activity = activity,
                    salt = salts[0],
                    rpId = rpId,
                    rpName = rpName,
                    userName = userName,
                    userDisplayName = userDisplayName,
                    autoRegister = autoRegister,
                    allowCredentialIds = effectiveAllow,
                    preferImmediatelyAvailableCredentials = preferImmediate,
                    onAssertionCredentialId = onAssertionCredentialId,
                )
            )
        }

        val output = ArrayList<ByteArray>(salts.size)
        var idx = 0
        while (idx < salts.size) {
            if (idx + 1 < salts.size) {
                // Always attempt dual-salt; if the authenticator drops
                // `results.second` we recover via single-salt below at
                // zero extra prompt cost.
                try {
                    val pair = getDualSaltAssertionWithPrfOrRegister(
                        activity = activity,
                        salt1 = salts[idx],
                        salt2 = salts[idx + 1],
                        rpId = rpId,
                        rpName = rpName,
                        userName = userName,
                        userDisplayName = userDisplayName,
                        autoRegister = autoRegister,
                        allowCredentialIds = effectiveAllow,
                        preferImmediatelyAvailableCredentials = preferImmediate,
                        onAssertionCredentialId = onAssertionCredentialId,
                    )
                    output.add(pair.first)
                    if (pair.second != null) {
                        output.add(pair.second!!)
                        idx += 2
                        continue
                    }
                    // Got `results.first` but not `results.second`.
                    // Fall through to a single-salt assertion for
                    // salts[idx + 1]. The verdict stays `null` so the
                    // next pair re-probes (cheap; same prompt count
                    // either way).
                    output.add(
                        deriveSeedOrRegister(
                            activity = activity,
                            salt = salts[idx + 1],
                            rpId = rpId,
                            rpName = rpName,
                            userName = userName,
                            userDisplayName = userDisplayName,
                            autoRegister = autoRegister,
                            allowCredentialIds = effectiveAllow,
                            preferImmediatelyAvailableCredentials = preferImmediate,
                            onAssertionCredentialId = onAssertionCredentialId,
                        )
                    )
                    idx += 2
                    continue
                } catch (e: CredentialManagerPrfCoreException) {
                    throw e
                } catch (e: Exception) {
                    throw e.toCoreException(null)
                }
            }

            // Single-salt fallback: cached verdict says dual unsupported,
            // or this is the trailing odd salt of a 3+ batch.
            output.add(
                deriveSeedOrRegister(
                    activity = activity,
                    salt = salts[idx],
                    rpId = rpId,
                    rpName = rpName,
                    userName = userName,
                    userDisplayName = userDisplayName,
                    autoRegister = autoRegister,
                    allowCredentialIds = effectiveAllow,
                    preferImmediatelyAvailableCredentials = preferImmediate,
                    onAssertionCredentialId = onAssertionCredentialId,
                )
            )
            idx += 1
        }
        output
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
     * @return Credential ID plus AAGUID and backup-eligibility parsed from
     *   the attestation object. AAGUID and backupEligible are null when
     *   the attestation can't be parsed.
     */
    public suspend fun createCredential(
        activity: Activity,
        rpId: String,
        rpName: String,
        userName: String,
        userDisplayName: String,
        excludeCredentialIds: List<ByteArray> = emptyList(),
        userIdOverride: ByteArray? = null,
    ): RegisteredCredential = withContext(Dispatchers.Main) {
        val startedAtMs = System.currentTimeMillis()
        try {
            registerCredential(
                activity, rpId, rpName, userName, userDisplayName,
                excludeCredentialIds, userIdOverride,
            )
        } catch (e: CredentialManagerPrfCoreException) {
            throw e
        } catch (e: Exception) {
            throw e.toCoreException(System.currentTimeMillis() - startedAtMs)
        }
    }

    // ------------------------------------------------------------------
    // Private
    // ------------------------------------------------------------------

    private suspend fun getAssertionWithPrf(
        activity: Activity,
        salt: String,
        rpId: String,
        allowCredentialIds: List<ByteArray> = emptyList(),
        preferImmediatelyAvailableCredentials: Boolean = true,
        onAssertionCredentialId: ((ByteArray) -> Unit)? = null,
    ): ByteArray {
        val prfEval = JSONObject().apply {
            put("first", encodeBase64Url(salt.toByteArray(Charsets.UTF_8)))
        }
        val results = runAssertion(
            activity, rpId, allowCredentialIds,
            preferImmediatelyAvailableCredentials, prfEval, onAssertionCredentialId,
        )
        val first = results.optString("first")
        if (first.isNullOrEmpty()) {
            throw CredentialManagerPrfCoreException(Kind.PrfEvaluationFailed, "empty result")
        }
        return decodeBase64Url(first)
    }

    /**
     * Dual-salt assertion with auto-register on missing credential.
     * Returns `(first, second?)`; `second` is null when the
     * authenticator dropped `saltInput2`.
     */
    private suspend fun getDualSaltAssertionWithPrfOrRegister(
        activity: Activity,
        salt1: String,
        salt2: String,
        rpId: String,
        rpName: String,
        userName: String,
        userDisplayName: String,
        autoRegister: Boolean,
        allowCredentialIds: List<ByteArray>,
        preferImmediatelyAvailableCredentials: Boolean = true,
        onAssertionCredentialId: ((ByteArray) -> Unit)?,
    ): Pair<ByteArray, ByteArray?> {
        val startedAtMs = System.currentTimeMillis()
        return try {
            getDualSaltAssertionWithPrf(
                activity, salt1, salt2, rpId, allowCredentialIds,
                preferImmediatelyAvailableCredentials, onAssertionCredentialId,
            )
        } catch (e: NoCredentialException) {
            if (!autoRegister) {
                throw CredentialManagerPrfCoreException(Kind.CredentialNotFound, e.message)
            }
            @Suppress("UNUSED_VARIABLE")
            val ignored = registerCredential(
                activity, rpId, rpName, userName, userDisplayName,
            )
            getDualSaltAssertionWithPrf(
                activity, salt1, salt2, rpId, allowCredentialIds,
                preferImmediatelyAvailableCredentials, onAssertionCredentialId,
            )
        } catch (e: CredentialManagerPrfCoreException) {
            throw e
        } catch (e: Exception) {
            throw e.toCoreException(System.currentTimeMillis() - startedAtMs)
        }
    }

    private suspend fun getDualSaltAssertionWithPrf(
        activity: Activity,
        salt1: String,
        salt2: String,
        rpId: String,
        allowCredentialIds: List<ByteArray>,
        preferImmediatelyAvailableCredentials: Boolean = true,
        onAssertionCredentialId: ((ByteArray) -> Unit)?,
    ): Pair<ByteArray, ByteArray?> {
        val prfEval = JSONObject().apply {
            put("first", encodeBase64Url(salt1.toByteArray(Charsets.UTF_8)))
            put("second", encodeBase64Url(salt2.toByteArray(Charsets.UTF_8)))
        }
        val results = runAssertion(
            activity, rpId, allowCredentialIds,
            preferImmediatelyAvailableCredentials, prfEval, onAssertionCredentialId,
        )
        val first = results.optString("first")
        if (first.isNullOrEmpty()) {
            throw CredentialManagerPrfCoreException(Kind.PrfEvaluationFailed, "empty first result")
        }
        val firstBytes = decodeBase64Url(first)

        // saltInput2 may be silently dropped by older Credential Manager
        // implementations or third-party providers; surface as null so
        // the caller falls back to a single-salt assertion for it.
        val second = results.optString("second", "")
        if (second.isEmpty()) return Pair(firstBytes, null)
        val secondBytes = try {
            decodeBase64Url(second)
        } catch (_: Exception) {
            return Pair(firstBytes, null)
        }
        return Pair(firstBytes, secondBytes)
    }

    /**
     * Build the WebAuthn assertion JSON, run the ceremony with
     * cross-device hybrid suppressed, fire `onAssertionCredentialId`
     * on success, and return the parsed `prf.results` JSONObject.
     * `prfEval` is the caller's `{ first, second? }` shape.
     */
    private suspend fun runAssertion(
        activity: Activity,
        rpId: String,
        allowCredentialIds: List<ByteArray>,
        preferImmediatelyAvailableCredentials: Boolean,
        prfEval: JSONObject,
        onAssertionCredentialId: ((ByteArray) -> Unit)?,
    ): JSONObject {
        // JSONObject so integrator-supplied rpId is escaped correctly;
        // raw template interpolation would break on quotes / backslashes
        // and surface as opaque JSONException deep in Credential Manager.
        val requestJson = JSONObject().apply {
            put("challenge", randomBase64Url(32))
            put("rpId", rpId)
            put("allowCredentials", JSONArray().apply {
                for (credId in allowCredentialIds) {
                    put(JSONObject().apply {
                        put("type", "public-key")
                        put("id", encodeBase64Url(credId))
                    })
                }
            })
            put("userVerification", "required")
            put("extensions", JSONObject().apply {
                put("prf", JSONObject().apply {
                    put("eval", prfEval)
                })
            })
        }.toString()

        val option = GetPublicKeyCredentialOption(requestJson)
        // Default `true` suppresses the cross-device QR sheet so a
        // missing local credential surfaces as NoCredentialException
        // instead of a hybrid flow the wallet user will never use.
        // Per-call `false` opts back into the picker for hosts that
        // want to support cross-device sign-in.
        val request = GetCredentialRequest.Builder()
            .addCredentialOption(option)
            .setPreferImmediatelyAvailableCredentials(preferImmediatelyAvailableCredentials)
            .build()
        val response = credentialManager(activity).getCredential(activity, request)

        val authResponseJson = response.credential.data.getString(
            "androidx.credentials.BUNDLE_KEY_AUTHENTICATION_RESPONSE_JSON",
        ) ?: throw CredentialManagerPrfCoreException(
            Kind.AuthenticationFailed, "No credential response",
        )
        val responseJson = JSONObject(authResponseJson)

        // Capture the asserted credential ID for two purposes:
        //
        //   1. Auto-add to KnownCredentialsStore (always). Idempotent
        //      add migrates users whose passkey predates our tracking:
        //      first sign-in seeds the store, so the next createPasskey
        //      correctly hits the platform-level "already exists" guard
        //      via excludeCredentials. Without this, the store stays
        //      empty until a fresh registration runs, and a returning
        //      user with a pre-tracking credential could accidentally
        //      register a duplicate.
        //
        //   2. Forward to the host's optional onAssertionCredentialId
        //      callback for any host-side bookkeeping (per-cred
        //      metadata, last-seen timestamps, etc.).
        //
        // Both are best-effort: a malformed rawId or a throwing callback
        // must not block the seed return.
        val rawIdEncoded = responseJson.optString("rawId", "")
        if (rawIdEncoded.isNotEmpty()) {
            try {
                KnownCredentialsStore.add(activity.applicationContext, rawIdEncoded, rpId)
            } catch (_: Exception) {}
            if (onAssertionCredentialId != null) {
                try {
                    onAssertionCredentialId(decodeBase64Url(rawIdEncoded))
                } catch (_: Exception) {}
            }
        }

        val extensions = responseJson.optJSONObject("clientExtensionResults")
            ?: throw CredentialManagerPrfCoreException(Kind.PrfNotSupported)
        val prf = extensions.optJSONObject("prf")
            ?: throw CredentialManagerPrfCoreException(Kind.PrfNotSupported)
        return prf.optJSONObject("results")
            ?: throw CredentialManagerPrfCoreException(Kind.PrfEvaluationFailed, "no results")
    }

    private fun encodeBase64Url(bytes: ByteArray): String =
        Base64.encodeToString(bytes, Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING)

    private fun decodeBase64Url(s: String): ByteArray =
        Base64.decode(s, Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING)

    private suspend fun registerCredential(
        activity: Activity,
        rpId: String,
        rpName: String,
        userName: String,
        userDisplayName: String,
        excludeCredentialIds: List<ByteArray> = emptyList(),
        userIdOverride: ByteArray? = null,
    ): RegisteredCredential {
        val credentialManager = credentialManager(activity)
        val challenge = randomBase64Url(32)
        val userId = userIdOverride
            ?.let { Base64.encodeToString(it, Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING) }
            ?: randomBase64Url(16)

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
        val credentialId = Base64.decode(rawId, Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING)
        var aaguid: ByteArray? = null
        var backupEligible: Boolean? = null
        val attestationB64 = responseJson.optJSONObject("response")?.optString("attestationObject", "") ?: ""
        if (attestationB64.isNotEmpty()) {
            val attestation = Base64.decode(
                attestationB64,
                Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING,
            )
            extractRegistrationMetadata(attestation)?.let { meta ->
                aaguid = meta.first
                backupEligible = meta.second
            }
        }
        return RegisteredCredential(credentialId, aaguid, backupEligible)
    }

    /**
     * Extract AAGUID + BE flag from the attestation object's authenticator
     * data via byte-pattern search for the "authData" CBOR key. Returns
     * null when the pattern isn't found or the byte string is too short.
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

    private fun randomBase64Url(byteCount: Int): String {
        val bytes = ByteArray(byteCount)
        secureRandom.nextBytes(bytes)
        return encodeBase64Url(bytes)
    }

    /**
     * Map an arbitrary exception thrown by Credential Manager into the
     * typed core exception. `elapsedMs` is the wall-clock time between
     * the start of the ceremony and this error firing; when supplied,
     * a "cancellation" that took longer than ~55s is reclassified as
     * [Kind.UserTimedOut] (the OS biometric inactivity timeout) rather
     * than [Kind.UserCancelled] (the user actively dismissed the
     * prompt). Pass `null` when timing is unknown to preserve the
     * historical mapping (`UserCancelled`).
     */
    private fun Exception.toCoreException(
        elapsedMs: Long? = null,
    ): CredentialManagerPrfCoreException = when (this) {
        is GetCredentialCancellationException,
        is CreateCredentialCancellationException ->
            CredentialManagerPrfCoreException(classifyCancellation(elapsedMs))

        is NoCredentialException ->
            CredentialManagerPrfCoreException(Kind.CredentialNotFound)

        // Surface the platform's duplicate-prevention check as a typed
        // kind so callers can route the user to the sign-in path instead
        // of treating it as a generic registration failure. Credential
        // Manager wraps WebAuthn DOM errors in CreatePublicKeyCredential
        // DomException with the spec-level error in `domError`. Must be
        // matched before the generic CreateCredentialException case
        // since it's a subclass.
        is CreatePublicKeyCredentialDomException ->
            if (domError is InvalidStateError) {
                CredentialManagerPrfCoreException(
                    Kind.CredentialAlreadyExists,
                    message ?: "Credential already registered for this RP",
                )
            } else {
                CredentialManagerPrfCoreException(
                    Kind.AuthenticationFailed,
                    "${type}: ${message ?: toString()}",
                )
            }

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

    /**
     * Discriminate between a user-dismissed prompt and the OS biometric
     * inactivity timeout. AndroidX surfaces both as the same
     * `*CancellationException`. The wall-clock between ceremony start
     * and the throw is the only in-process signal available:
     * Credential Manager tears the prompt down at the platform's
     * biometric inactivity timeout (~55s+), so anything at or beyond
     * that is reclassified as [Kind.UserTimedOut].
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
         * The OS biometric prompt timed out without user interaction
         * (~55s+ inactivity). Distinct from [UserCancelled], which means
         * the user actively dismissed the prompt; hosts may auto-retry
         * or surface a re-prompt UI without treating this as user
         * intent to abandon.
         */
        UserTimedOut,
        /** No credential exists for the RP and auto-registration was not attempted. */
        CredentialNotFound,
        /** Credential Manager reported an authentication / registration failure. */
        AuthenticationFailed,
        /** PRF evaluation produced an empty or malformed response. */
        PrfEvaluationFailed,
        /** Platform or app configuration error (e.g. missing assetlinks.json, misconfigured RP ID). */
        Configuration,
        /**
         * Credential registration was refused because a credential matching
         * one of the IDs in `excludeCredentialIds` is already on the
         * authenticator. Surfaces the platform's duplicate-prevention check
         * as a typed kind so callers can route the user to the sign-in
         * path instead of treating it as a generic registration failure.
         */
        CredentialAlreadyExists,
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
