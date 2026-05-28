import breez_sdk_spark.GetInfoRequest
import breez_sdk_spark.Seed
import breez_sdk_spark.SyncWalletRequest
import breez_sdk_spark.initLogging

import java.nio.file.Files
import java.nio.file.Path

import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

import kotlinx.coroutines.runBlocking

// --- arg parsing -----------------------------------------------------------

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

fun maskPassword(url: String): String =
    url.replace(Regex("://([^:]*):[^@/]*@"), "://$1:***@")

/** HMAC-SHA512(masterSecret, userId) → 64-byte entropy seed. */
fun deriveSeedBytes(masterSecret: String, userId: String): ByteArray {
    val mac = Mac.getInstance("HmacSHA512")
    mac.init(SecretKeySpec(masterSecret.toByteArray(Charsets.UTF_8), "HmacSHA512"))
    return mac.doFinal(userId.toByteArray(Charsets.UTF_8))
}

// --- smoke mode -----------------------------------------------------------

fun smokeTest(opts: Map<String, String>) = runBlocking {
    val mysqlUrl = opts["mysql-url"]
        ?: error("--mysql-url=mysql://user:pass@host:port/dbname is required")
    val masterSecret = opts["master-secret"]
        ?: System.getenv("MASTER_SECRET")
        ?: error("--master-secret=<hex-or-string> or MASTER_SECRET env var is required")
    val userId = opts["user-id"] ?: "smoke-default"

    println("[smoke] user-id=$userId  mysql=${maskPassword(mysqlUrl)}")

    val seed: Seed = Seed.Entropy(deriveSeedBytes(masterSecret, userId))
    val config = benchConfig()
    val handlers = SharedHandlers.create(mysqlUrl)

    println("[smoke] building SDK")
    val tConnect = System.currentTimeMillis()
    val sdk = buildSdk(config, seed, handlers)
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

// --- trace-sync mode (verbose explicit sync with Rust tracing) ------------

fun traceSync(opts: Map<String, String>) = runBlocking {
    val mysqlUrl = opts["mysql-url"]
        ?: error("--mysql-url=mysql://user:pass@host:port/dbname is required")
    val masterSecret = opts["master-secret"]
        ?: System.getenv("MASTER_SECRET")
        ?: error("--master-secret=<hex-or-string> or MASTER_SECRET env var is required")
    val userId = opts["user-id"] ?: TREASURER_USER_ID
    val logDir = opts["log-dir"] ?: "out/.trace-logs/$userId-${System.currentTimeMillis()}"
    val logFilter = opts["log-filter"] ?: "debug"
    Files.createDirectories(Path.of(logDir))

    println("[trace] user-id=$userId  mysql=${maskPassword(mysqlUrl)}")
    println("[trace] init_logging dir=$logDir filter=$logFilter")
    initLogging(logDir, null, logFilter)

    val seed: Seed = Seed.Entropy(deriveSeedBytes(masterSecret, userId))
    val handlers = SharedHandlers.create(mysqlUrl)

    println("[trace] building SDK …")
    val tBuild = System.currentTimeMillis()
    val sdk = buildSdk(benchConfig(), seed, handlers)
    println("[trace] build took ${System.currentTimeMillis() - tBuild}ms")

    try {
        val tCached = System.currentTimeMillis()
        val cached = sdk.getInfo(GetInfoRequest(ensureSynced = false))
        println("[trace] getInfo(local) ${System.currentTimeMillis() - tCached}ms — balance=${cached.balanceSats}")

        println("[trace] calling syncWallet() …")
        val tSync = System.currentTimeMillis()
        sdk.syncWallet(SyncWalletRequest)
        val syncMs = System.currentTimeMillis() - tSync
        val synced = sdk.getInfo(GetInfoRequest(ensureSynced = false))
        println("[trace] syncWallet + getInfo ${syncMs}ms — balance=${synced.balanceSats}")
        println("[trace] log file: $logDir/sdk.log")
    } finally {
        try {
            sdk.disconnect()
        } catch (e: Exception) {
            System.err.println("[trace] disconnect warn: ${e.message}")
        }
    }
}

// --- CLI dispatch ---------------------------------------------------------

fun main(args: Array<String>) {
    val opts = parseArgs(args)
    when (opts["mode"]) {
        "smoke" -> smokeTest(opts)
        "server" -> runServer(opts)
        "fund" -> fundTreasurer(opts)
        "seed-senders" -> seedSenders(opts)
        "mint-invoices" -> mintInvoices(opts)
        "audit-bolt11" -> auditBolt11(opts)
        "loadgen" -> runLoadGen(opts)
        "trace-sync" -> traceSync(opts)
        null, "help" -> {
            println(
                """
                breez-sdk-spark-benchmarks

                Usage: ./gradlew run --args="--mode=<mode> [options]"

                Modes:
                  smoke           One-shot connect / getInfo / disconnect.
                  server          HTTP bench server (/info, /send, /receive).
                  fund            Top up the reserved treasurer via the Lightspark faucet.
                  seed-senders    Top up the K sender wallets from the treasurer.
                  mint-invoices   Pre-mint a bolt11 pool + probe the LN fee (for send_ln runs).
                  audit-bolt11    Post-run settlement audit of a bolt11 sweep dir.
                  loadgen         Open-loop HTTP load generator.
                  trace-sync      Verbose explicit sync for one wallet (diagnostic).

                See README.md and `make help` for the full set of options; the
                sweep driver (`make run`) wires them automatically.
                """.trimIndent()
            )
        }
        else -> error("Unknown mode: ${opts["mode"]}. Use --mode=help.")
    }
}
