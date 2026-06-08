package technology.breez.spark.passkey

import android.app.Activity
import breez_sdk_spark.DeriveSeedsOutput
import breez_sdk_spark.DeriveSeedsRequest
import breez_sdk_spark.DomainAssociation
import breez_sdk_spark.PasskeyCredential
import breez_sdk_spark.PasskeyProviderOptions
import breez_sdk_spark.PrfProvider
import breez_sdk_spark.PrfProviderException
import technology.breez.spark.passkey.core.CredentialManagerPrfCore
import technology.breez.spark.passkey.core.CredentialManagerPrfCoreException
import technology.breez.spark.passkey.core.DomainAssociationResult

/**
 * Built-in [PrfProvider] that uses the AndroidX Credential Manager +
 * WebAuthn PRF extension to derive deterministic 32-byte seeds from platform
 * passkeys. A thin wrapper around [CredentialManagerPrfCore] that adapts the
 * core's exceptions into the UniFFI-generated [PrfProviderException] variants.
 *
 * ## Requirements
 *
 * - Android 9+ (API 28) with Google Play Services, or Android 14+ (API 34)
 *   with any compatible Credential Manager provider.
 * - A valid `/.well-known/assetlinks.json` for the RP domain.
 * - A physical device: emulators cannot complete the WebAuthn registration
 *   handshake.
 *
 * @param activityProvider Called lazily per request for the current top
 *   Activity. Credential Manager is lifecycle-sensitive: this MUST return
 *   the foreground, RESUMED, non-finishing Activity (a stale or background
 *   instance surfaces as opaque Credential Manager failures). A lambda
 *   avoids holding a stale reference across configuration changes.
 * @param options Relying Party and user identity (`rpId`, `rpName`,
 *   `userName`, `userDisplayName`). Unset `rpId` / `rpName` default to
 *   [BREEZ_RP_ID] / [DEFAULT_RP_NAME]; `userName` defaults to `rpName` and
 *   `userDisplayName` to `userName`. The same [PasskeyProviderOptions] is
 *   settable on `PasskeyConfig` for the zero-config client.
 */
public class PasskeyProvider(
    private val activityProvider: () -> Activity,
    options: PasskeyProviderOptions = PasskeyProviderOptions(),
) : PrfProvider {

    public companion object {
        /**
         * Constant identifying Breez's shared `keys.breez.technology` RP.
         * Pass as `rpId` when opting into the Breez-managed Relying Party
         * (only valid for apps registered with Breez). Apps with their
         * own RP domain pass their own string.
         */
        public const val BREEZ_RP_ID: String = CredentialManagerPrfCore.DEFAULT_RP_ID

        /**
         * Default Relying Party name used by the zero-config
         * [PasskeyClient] factory / [PasskeyClientBuilder] when no
         * `rpName` is supplied. Surfaces in some credential-manager UIs
         * (Google Password Manager).
         */
        public const val DEFAULT_RP_NAME: String = "Breez"
    }

    private val rpId: String = options.rpId ?: BREEZ_RP_ID
    private val rpName: String = options.rpName ?: DEFAULT_RP_NAME
    private val resolvedUserName: String = options.userName ?: rpName
    private val resolvedUserDisplayName: String = options.userDisplayName ?: resolvedUserName

    /** The configured PRF engine; holds the rp identity. */
    private val core = CredentialManagerPrfCore(
        rpId = rpId,
        rpName = rpName,
        userName = resolvedUserName,
        userDisplayName = resolvedUserDisplayName,
        activityProvider = activityProvider,
    )

    /**
     * Bulk PRF derivation backed by [CredentialManagerPrfCore.deriveSeeds].
     * Uses the WebAuthn PRF dual-salt fast path where the authenticator
     * honors `prf.eval.second`, else falls back to single-salt. Output order
     * matches input order; returns the seeds plus the credential ID observed
     * in the same assertion (null when none, e.g. empty `salts`).
     *
     * Never auto-creates a credential during derivation: a missing credential
     * surfaces as `CredentialNotFound`, not a new passkey.
     */
    override suspend fun deriveSeeds(request: DeriveSeedsRequest): DeriveSeedsOutput =
        try {
            val derivation = core.deriveSeeds(
                salts = request.salts,
                autoRegister = false,
                allowCredentials = request.allowCredentials,
                preferImmediatelyAvailableCredentials =
                    request.preferImmediatelyAvailableCredentials ?: true,
            )
            // The core observes the asserted credential ID inline and
            // returns it alongside the seeds.
            DeriveSeedsOutput(derivation.seeds, derivation.credentialId)
        } catch (e: CredentialManagerPrfCoreException) {
            throw e.toPrfProviderException()
        }

    override suspend fun isSupported(): Boolean =
        CredentialManagerPrfCore.isSupported()

    override suspend fun checkDomainAssociation(): DomainAssociation {
        val result = core.checkDomainAssociation()
        return when (result) {
            is DomainAssociationResult.Associated ->
                DomainAssociation.Associated

            // Soft-fail on Android: degrade NotAssociated to Skipped.
            //
            // Android's CredentialManager runs its own Digital Asset Links
            // check against Google Play Services' assetlinks cache, typically
            // fresher than the public DAL API the core queries. A "no match"
            // from our probe can be a false negative, so hard-blocking here
            // regresses vs. the native path: degrade to Skipped and let the
            // CredentialManager call surface a real error if one exists.
            // Logged at WARN so the DAL mismatch stays visible in logcat.
            is DomainAssociationResult.NotAssociated -> {
                android.util.Log.w(
                    "CredentialManagerPrf",
                    "Digital Asset Links reports mismatch; degrading to Skipped. " +
                        "source=${result.source} reason=${result.reason}"
                )
                DomainAssociation.Skipped(
                    reason = "[soft-fail on Android] ${result.reason}"
                )
            }

            is DomainAssociationResult.Skipped ->
                DomainAssociation.Skipped(reason = result.reason)
        }
    }

    /**
     * Register a new passkey with PRF support. One ceremony, no derivation.
     *
     * `user.id` is never host-supplied: the core mints a fresh random 16-byte
     * handle and returns it as [PasskeyCredential.userId]. Pass already-
     * registered IDs in `excludeCredentials` so the platform refuses a
     * duplicate even after reinstall.
     */
    override suspend fun createPasskey(excludeCredentials: List<ByteArray>): PasskeyCredential {
        try {
            val c = core.register(excludeCredentials)
            return PasskeyCredential(c.credentialId, c.userId, c.aaguid, c.backupEligible)
        } catch (e: CredentialManagerPrfCoreException) {
            throw e.toPrfProviderException()
        }
    }

    private fun CredentialManagerPrfCoreException.toPrfProviderException(): PrfProviderException =
        when (kind) {
            CredentialManagerPrfCore.Kind.PrfNotSupported ->
                PrfProviderException.PrfNotSupported()
            CredentialManagerPrfCore.Kind.UserCancelled ->
                PrfProviderException.UserCancelled()
            CredentialManagerPrfCore.Kind.UserTimedOut ->
                PrfProviderException.UserTimedOut()
            CredentialManagerPrfCore.Kind.CredentialNotFound ->
                PrfProviderException.CredentialNotFound(message ?: "Credential not found")
            CredentialManagerPrfCore.Kind.AuthenticationFailed ->
                PrfProviderException.AuthenticationFailed(message ?: "")
            CredentialManagerPrfCore.Kind.PrfEvaluationFailed ->
                PrfProviderException.PrfEvaluationFailed(message ?: "")
            CredentialManagerPrfCore.Kind.Configuration ->
                PrfProviderException.Configuration(message ?: "")
            CredentialManagerPrfCore.Kind.CredentialAlreadyExists ->
                PrfProviderException.CredentialAlreadyExists(message ?: "")
            CredentialManagerPrfCore.Kind.Generic ->
                PrfProviderException.Generic(message ?: "")
        }
}
