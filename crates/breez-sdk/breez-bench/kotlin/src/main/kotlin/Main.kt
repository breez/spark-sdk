import breez_sdk_spark.GetInfoRequest
import breez_sdk_spark.Network
import breez_sdk_spark.SdkBuilder
import breez_sdk_spark.Seed
import breez_sdk_spark.defaultConfig
import breez_sdk_spark.defaultMysqlStorageConfig
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec
import kotlinx.coroutines.runBlocking

fun parseArgs(args: Array<String>): Map<String, String> {
    val out = mutableMapOf<String, String>()
    for (raw in args) {
        if (!raw.startsWith("--")) continue
        val eq = raw.indexOf('=')
        if (eq < 0) {
            out[raw.substring(2)] = "true"
        } else {
            out[raw.substring(2, eq)] = raw.substring(eq + 1)
        }
    }
    return out
}

private fun maskPassword(url: String): String =
    url.replace(Regex("://([^:]*):[^@/]*@"), "://$1:***@")

/**
 * Deterministic seed derivation: HMAC-SHA512(masterSecret, userId) → 64 bytes.
 *
 * The bench uses raw entropy via [Seed.Entropy] rather than a BIP39 mnemonic;
 * the SDK accepts both, and entropy avoids carrying around the wordlist.
 *
 * In a real deployment, the partner replaces this with their own secrets
 * store lookup (user id → seed bytes). The shape of "stable per-user bytes"
 * is what matters — the SDK derives the wallet from there.
 */
fun deriveSeedBytes(masterSecret: String, userId: String): ByteArray {
    val mac = Mac.getInstance("HmacSHA512")
    mac.init(SecretKeySpec(masterSecret.toByteArray(Charsets.UTF_8), "HmacSHA512"))
    return mac.doFinal(userId.toByteArray(Charsets.UTF_8))
}

fun smokeTest(opts: Map<String, String>) = runBlocking {
    val mysqlUrl = opts["mysql-url"]
        ?: error("--mysql-url=mysql://user:pass@host:port/dbname is required")
    val masterSecret = opts["master-secret"]
        ?: System.getenv("MASTER_SECRET")
        ?: error("--master-secret=<hex-or-string> or MASTER_SECRET env var is required")
    val userId = opts["user-id"] ?: "smoke-default"

    println("[smoke] user-id=$userId  mysql=${maskPassword(mysqlUrl)}")

    val seed: Seed = Seed.Entropy(deriveSeedBytes(masterSecret, userId))
    val config = defaultConfig(Network.REGTEST)

    val builder = SdkBuilder(config, seed)
    builder.withMysqlBackend(defaultMysqlStorageConfig(mysqlUrl))

    println("[smoke] building SDK")
    val tConnect = System.currentTimeMillis()
    val sdk = builder.build()
    println("[smoke] connect=${System.currentTimeMillis() - tConnect}ms")

    try {
        val info = sdk.getInfo(GetInfoRequest(ensureSynced = false))
        println("[smoke] balance=${info.balanceSats} sats")
    } finally {
        try {
            sdk.disconnect()
        } catch (e: Exception) {
            System.err.println("[smoke] disconnect warn: ${e.message}")
        }
    }
    println("[smoke] OK")
}

fun main(args: Array<String>) {
    val opts = parseArgs(args)
    when (opts["mode"]) {
        "smoke" -> smokeTest(opts)
        null, "help" -> {
            println(
                """
                breez-sdk-spark-benchmarks

                Usage: ./gradlew run --args="--mode=<mode> [options]"

                Modes:
                  smoke      Single-request flow check: derive seed for one user-id,
                             connect, getInfo, disconnect. No HTTP server yet (Phase 2).

                Options:
                  --mysql-url=mysql://user:pass@host:port/db   MySQL endpoint, including database name
                  --master-secret=<string>                     Master secret for HMAC seed derivation
                                                               (or set MASTER_SECRET env var)
                  --user-id=<id>                               User id to derive seed for (default: smoke-default)
                """.trimIndent()
            )
        }
        else -> error("Unknown mode: ${opts["mode"]}. Use --mode=help.")
    }
}
