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
    /** Optional wallet name for seed derivation. If omitted, the core uses the default name. */
    val walletName: String?,
    /** Whether to list and select from wallet names published to Nostr. */
    val listWalletNames: Boolean,
    /** Whether to publish the wallet name to Nostr. */
    val storeWalletName: Boolean,
    /** Optional relying party ID for FIDO2 provider (default: keys.breez.technology). */
    val rpId: String?,
)

// ---------------------------------------------------------------------------
// File-based PRF provider
// ---------------------------------------------------------------------------

private const val SECRET_FILE_NAME = "seedless-restore-secret"

/**
 * File-based implementation of [PasskeyPrfProvider].
 *
 * Uses HMAC-SHA256 with a secret stored in a file. The secret is generated
 * randomly on first use and persisted to disk.
 *
 * Security Notes:
 * - The secret file should be protected with appropriate file permissions
 * - This is less secure than hardware-backed solutions like YubiKey
 * - Suitable for development/testing or when hardware keys are unavailable
 */
class FilePrfProvider(dataDir: String) : PasskeyPrfProvider {
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

    override suspend fun derivePrfSeed(salt: String): ByteArray {
        val mac = Mac.getInstance("HmacSHA256")
        mac.init(SecretKeySpec(secret, "HmacSHA256"))
        return mac.doFinal(salt.toByteArray(Charsets.UTF_8))
    }

    override suspend fun isPrfAvailable(): Boolean {
        return true
    }
}

// ---------------------------------------------------------------------------
// Stub providers for hardware-dependent backends
// ---------------------------------------------------------------------------

/**
 * Stub provider for hardware-dependent backends that are not yet supported.
 */
class NotYetSupportedProvider(private val name: String) : PasskeyPrfProvider {
    override suspend fun derivePrfSeed(salt: String): ByteArray {
        throw UnsupportedOperationException("$name passkey provider is not yet supported in the Kotlin CLI")
    }

    override suspend fun isPrfAvailable(): Boolean {
        throw UnsupportedOperationException("$name passkey provider is not yet supported in the Kotlin CLI")
    }
}

// ---------------------------------------------------------------------------
// Provider factory
// ---------------------------------------------------------------------------

/**
 * Creates a [PasskeyPrfProvider] for the given provider type.
 */
fun buildPrfProvider(provider: PasskeyProvider, dataDir: String, rpId: String? = null): PasskeyPrfProvider {
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
    provider: PasskeyPrfProvider,
    breezApiKey: String?,
    walletName: String?,
    listWalletNames: Boolean,
    storeWalletName: Boolean,
): Seed {
    val relayConfig = NostrRelayConfig(breezApiKey = breezApiKey)
    val passkey = Passkey(provider, relayConfig)

    // --store-wallet-name: publish the wallet name to Nostr
    if (storeWalletName && walletName != null) {
        println("Publishing wallet name '$walletName' to Nostr...")
        passkey.storeWalletName(walletName)
        println("Wallet name '$walletName' published successfully.")
    }

    // --list-wallet-names: query Nostr and prompt user to select
    val resolvedName: String? = if (listWalletNames) {
        println("Querying Nostr for available wallet names...")
        val walletNames = passkey.listWalletNames()

        if (walletNames.isEmpty()) {
            throw IllegalStateException("No wallet names found on Nostr for this identity")
        }

        println("Available wallet names:")
        walletNames.forEachIndexed { i, name ->
            println("  ${i + 1}: $name")
        }

        print("Select wallet name (1-${walletNames.size}): ")
        System.out.flush()
        val input = readlnOrNull()?.trim() ?: throw IllegalStateException("No input")
        val idx = input.toIntOrNull() ?: throw IllegalArgumentException("Invalid selection")

        if (idx < 1 || idx > walletNames.size) {
            throw IllegalArgumentException("Selection out of range")
        }

        walletNames[idx - 1]
    } else {
        walletName
    }

    val wallet = passkey.getWallet(resolvedName)
    return wallet.seed
}
