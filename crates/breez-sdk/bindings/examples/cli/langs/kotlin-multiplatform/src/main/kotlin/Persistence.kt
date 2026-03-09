import java.io.File
import java.security.SecureRandom

private const val PHRASE_FILE_NAME = "phrase"
private const val HISTORY_FILE_NAME = "history.txt"

/**
 * BIP39 English wordlist (2048 words).
 * Used for mnemonic generation when no external BIP39 library is available.
 */
private val BIP39_WORDLIST: List<String> by lazy {
    val resource = object {}.javaClass.getResourceAsStream("/bip39-english.txt")
    if (resource != null) {
        resource.bufferedReader().readLines().filter { it.isNotBlank() }
    } else {
        // Fallback: load from the embedded wordlist
        loadEmbeddedWordlist()
    }
}

/**
 * Handles mnemonic and history file storage for the CLI.
 */
class CliPersistence(private val dataDir: String) {

    /**
     * Reads an existing mnemonic from the data directory,
     * or generates a new 12-word BIP39 mnemonic and saves it.
     */
    fun getOrCreateMnemonic(): String {
        val file = File(dataDir, PHRASE_FILE_NAME)

        if (file.exists()) {
            return file.readText().trim()
        }

        val mnemonic = generateMnemonic()
        file.writeText(mnemonic)
        return mnemonic
    }

    /**
     * Returns the path to the REPL history file.
     */
    fun historyFile(): String {
        return File(dataDir, HISTORY_FILE_NAME).absolutePath
    }

    /**
     * Generates a 12-word BIP39 mnemonic using SecureRandom.
     */
    private fun generateMnemonic(): String {
        val wordlist = BIP39_WORDLIST
        if (wordlist.size != 2048) {
            error("BIP39 wordlist must contain exactly 2048 words, got ${wordlist.size}")
        }

        // Generate 128 bits (16 bytes) of entropy for a 12-word mnemonic
        val entropy = ByteArray(16)
        SecureRandom().nextBytes(entropy)

        // Calculate checksum: first 4 bits of SHA-256 hash
        val digest = java.security.MessageDigest.getInstance("SHA-256")
        val hash = digest.digest(entropy)
        val checksumBits = (hash[0].toInt() and 0xFF) shr 4 // top 4 bits

        // Convert entropy + checksum to 11-bit groups
        // 128 bits entropy + 4 bits checksum = 132 bits = 12 * 11 bits
        val bits = StringBuilder()
        for (b in entropy) {
            bits.append(String.format("%8s", Integer.toBinaryString(b.toInt() and 0xFF)).replace(' ', '0'))
        }
        bits.append(String.format("%4s", Integer.toBinaryString(checksumBits)).replace(' ', '0'))

        val words = mutableListOf<String>()
        for (j in 0 until 12) {
            val index = Integer.parseInt(bits.substring(j * 11, j * 11 + 11), 2)
            words.add(wordlist[index])
        }

        return words.joinToString(" ")
    }
}

/**
 * Embedded BIP39 English wordlist. This is a fallback used only if the resource file is not found.
 * In practice, the wordlist file should be placed at src/main/resources/bip39-english.txt.
 *
 * Since the full wordlist is 2048 words, and embedding it directly in source code would be very large,
 * we generate a simple deterministic mnemonic using SecureRandom and the wordlist resource.
 * If the resource is missing, this function returns an empty list and the program will error.
 */
private fun loadEmbeddedWordlist(): List<String> {
    // If no resource file is available, try to download or error out.
    // For a CLI tool, we recommend placing the wordlist in src/main/resources/.
    System.err.println(
        "Warning: BIP39 wordlist resource not found. " +
        "Place 'bip39-english.txt' in src/main/resources/ for mnemonic generation. " +
        "Falling back to a minimal approach."
    )
    // Return an empty list; getOrCreateMnemonic will fail with an informative error
    return emptyList()
}
