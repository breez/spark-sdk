import breez_sdk_spark.*
import java.io.File
import java.security.SecureRandom
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

/**
 * Passkey PRF provider type.
 */
enum class PasskeyProvider {
    FILE,
    YUBIKEY,
    FIDO2;

    companion object {
        fun fromString(s: String): PasskeyProvider {
            return when (s.lowercase()) {
                "file" -> FILE
                "yubikey" -> YUBIKEY
                "fido2" -> FIDO2
                else -> throw IllegalArgumentException("Invalid passkey provider '$s' (valid: file, yubikey, fido2)")
            }
        }
    }
}

/**
 * Configuration for passkey seed derivation.
 */
data class PasskeyConfig(
    /** The PRF provider to use. */
    val provider: PasskeyProvider,
    /** Optional label for seed derivation. If omitted, the core uses the default name. */
    val label: String?,
    /** Whether to list and select from labels published to Nostr. */
    val listLabels: Boolean,
    /** Whether to publish the label to Nostr. */
    val storeLabel: Boolean,
    /** Optional relying party ID for FIDO2 provider (default: keys.breez.technology). */
    val rpId: String?,
)

// ---------------------------------------------------------------------------
// File-based PRF provider
// ---------------------------------------------------------------------------

private const val SECRET_FILE_NAME = "seedless-restore-secret"

/**
 * File-based implementation of [PrfProvider].
 *
 * Uses HMAC-SHA256 with a secret stored in a file. The secret is generated
 * randomly on first use and persisted to disk.
 *
 * Security Notes:
 * - The secret file should be protected with appropriate file permissions
 * - This is less secure than hardware-backed solutions like YubiKey
 * - Suitable for development/testing or when hardware keys are unavailable
 */
class FilePrfProvider(dataDir: String) : PrfProvider {
    private val secret: ByteArray

    init {
        val secretFile = File(dataDir, SECRET_FILE_NAME)

        secret = if (secretFile.exists()) {
            val bytes = secretFile.readBytes()
            if (bytes.size != 32) {
                throw IllegalStateException("Invalid secret file: expected 32 bytes, got ${bytes.size}")
            }
            bytes
        } else {
            // Generate new random secret
            val newSecret = ByteArray(32)
            SecureRandom().nextBytes(newSecret)

            // Ensure data directory exists
            File(dataDir).mkdirs()

            // Save secret to file
            secretFile.writeBytes(newSecret)
            newSecret
        }
    }

    override suspend fun deriveSeeds(request: DeriveSeedsRequest): DeriveSeedsOutput {
        val mac = Mac.getInstance("HmacSHA256")
        mac.init(SecretKeySpec(secret, "HmacSHA256"))
        val seeds = request.salts.map { salt ->
            mac.reset()
            mac.doFinal(salt.toByteArray(Charsets.UTF_8))
        }
        return DeriveSeedsOutput(seeds = seeds, credentialId = null)
    }

    override suspend fun isSupported(): Boolean = true

    override suspend fun createPasskey(excludeCredentials: List<ByteArray>): PasskeyCredential {
        throw UnsupportedOperationException(
            "File-backed PRF provider does not implement create-credential; " +
                "use sign-in by label instead."
        )
    }

    override suspend fun checkDomainAssociation(): DomainAssociation =
        DomainAssociation.Skipped("FilePrfProvider does not verify domain association")
}

// ---------------------------------------------------------------------------
// Stub providers for hardware-dependent backends
// ---------------------------------------------------------------------------

/**
 * Stub provider for hardware-dependent backends that are not yet supported.
 */
class NotYetSupportedProvider(private val name: String) : PrfProvider {
    private fun notYet(): Nothing =
        throw UnsupportedOperationException("$name passkey provider is not yet supported in the Kotlin CLI")

    override suspend fun deriveSeeds(request: DeriveSeedsRequest): DeriveSeedsOutput = notYet()

    override suspend fun isSupported(): Boolean = notYet()

    override suspend fun createPasskey(excludeCredentials: List<ByteArray>): PasskeyCredential = notYet()

    override suspend fun checkDomainAssociation(): DomainAssociation =
        DomainAssociation.Skipped("$name does not verify domain association")
}

// ---------------------------------------------------------------------------
// Provider factory
// ---------------------------------------------------------------------------

/**
 * Creates a [PrfProvider] for the given provider type.
 */
fun buildPrfProvider(provider: PasskeyProvider, dataDir: String, rpId: String? = null): PrfProvider {
    return when (provider) {
        PasskeyProvider.FILE -> FilePrfProvider(dataDir)
        PasskeyProvider.YUBIKEY -> NotYetSupportedProvider("YubiKey")
        PasskeyProvider.FIDO2 -> NotYetSupportedProvider("FIDO2")
    }
}

// ---------------------------------------------------------------------------
// Passkey seed resolution (orchestration)
// ---------------------------------------------------------------------------

/**
 * Derives a wallet seed using the given PRF provider,
 * matching the Rust CLI's resolve_passkey_seed logic.
 */
suspend fun resolvePasskeySeed(
    provider: PrfProvider,
    breezApiKey: String?,
    label: String?,
    listLabels: Boolean,
    storeLabel: Boolean,
): Seed {
    val passkey = PasskeyClient(prfProvider = provider, breezApiKey = breezApiKey, config = null)

    // --list-labels: discovery sign-in (no cached label) returns the
    // discovered label set; prompt user to pick.
    val resolvedLabel: String? = if (listLabels) {
        println("Querying Nostr for available labels...")
        val response = passkey.signIn(SignInRequest(label = null))

        if (response.labels.isEmpty()) {
            throw IllegalStateException("No labels found on Nostr for this identity")
        }

        println("Available labels:")
        response.labels.forEachIndexed { i, name ->
            println("  ${i + 1}: $name")
        }

        print("Select label (1-${response.labels.size}): ")
        System.out.flush()
        val input = readlnOrNull()?.trim() ?: throw IllegalStateException("No input")
        val idx = input.toIntOrNull() ?: throw IllegalArgumentException("Invalid selection")

        if (idx < 1 || idx > response.labels.size) {
            throw IllegalArgumentException("Selection out of range")
        }

        response.labels[idx - 1]
    } else {
        label
    }

    // --store-label: publish before signing in so a fresh client can
    // discover the label later.
    if (storeLabel && resolvedLabel != null) {
        println("Publishing label '$resolvedLabel' to Nostr...")
        passkey.labels().store(resolvedLabel)
        println("Label '$resolvedLabel' published successfully.")
    }

    val response = passkey.signIn(SignInRequest(label = resolvedLabel))
    return response.wallet.seed
}
