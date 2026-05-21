import breez_sdk_spark.BreezSdk
import breez_sdk_spark.GetInfoRequest
import breez_sdk_spark.ListPaymentsRequest
import breez_sdk_spark.Network
import breez_sdk_spark.PaymentDetails
import breez_sdk_spark.PaymentDetailsFilter
import breez_sdk_spark.PaymentStatus
import breez_sdk_spark.PaymentType
import breez_sdk_spark.PrepareSendPaymentRequest
import breez_sdk_spark.ReceivePaymentMethod
import breez_sdk_spark.ReceivePaymentRequest
import breez_sdk_spark.SdkBuilder
import breez_sdk_spark.SdkContext
import breez_sdk_spark.SdkContextConfig
import breez_sdk_spark.Seed
import breez_sdk_spark.SendPaymentMethod
import breez_sdk_spark.SendPaymentOptions
import breez_sdk_spark.SendPaymentRequest
import breez_sdk_spark.SyncWalletRequest
import breez_sdk_spark.defaultMysqlStorageConfig
import breez_sdk_spark.defaultServerConfig
import breez_sdk_spark.initLogging
import breez_sdk_spark.newSharedSdkContext

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.sync.withPermit

import com.ionspin.kotlin.bignum.integer.BigInteger

import io.ktor.http.HttpStatusCode
import io.ktor.serialization.kotlinx.json.json
import io.ktor.server.application.call
import io.ktor.server.application.install
import io.ktor.server.engine.embeddedServer
import io.ktor.server.netty.Netty
import io.ktor.server.plugins.contentnegotiation.ContentNegotiation
import io.ktor.server.request.receive
import io.ktor.server.response.respond
import io.ktor.server.routing.get
import io.ktor.server.routing.post
import io.ktor.server.routing.routing

import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardOpenOption
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicInteger

import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

import kotlinx.coroutines.runBlocking
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull

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

private fun maskPassword(url: String): String =
    url.replace(Regex("://([^:]*):[^@/]*@"), "://$1:***@")

/** HMAC-SHA512(masterSecret, userId) → 64-byte entropy seed. */
fun deriveSeedBytes(masterSecret: String, userId: String): ByteArray {
    val mac = Mac.getInstance("HmacSHA512")
    mac.init(SecretKeySpec(masterSecret.toByteArray(Charsets.UTF_8), "HmacSHA512"))
    return mac.doFinal(userId.toByteArray(Charsets.UTF_8))
}

// --- bench config ---------------------------------------------------------

// Raised from SDK default of 4 to drain the treasurer's claim backlog faster.
fun benchConfig(): breez_sdk_spark.Config = defaultServerConfig(Network.REGTEST).apply {
    maxConcurrentClaims = 32u
}

// --- shared SDK transports ------------------------------------------------

class SharedHandlers private constructor(
    val context: SdkContext,
) {
    companion object {
        suspend fun create(mysqlUrl: String): SharedHandlers {
            val mysqlConfig = defaultMysqlStorageConfig(mysqlUrl).also {
                it.recycleTimeoutSecs = 300UL
                System.getenv("MYSQL_MAX_POOL")?.let { v ->
                    it.maxPoolSize = v.toUIntOrNull()?.takeIf { n -> n > 0u }
                        ?: error("MYSQL_MAX_POOL must be a positive integer; got '$v'")
                }
            }
            val connsPerOperator: UInt? = System.getenv("CONNS_PER_OPERATOR")?.let {
                it.toUIntOrNull()?.takeIf { n -> n > 0u }
                    ?: error("CONNS_PER_OPERATOR must be a positive integer; got '$it'")
            }
            val context = newSharedSdkContext(
                SdkContextConfig(
                    network = Network.REGTEST,
                    connectionsPerOperator = connsPerOperator,
                    mysqlConfig = mysqlConfig,
                )
            )
            return SharedHandlers(context = context)
        }
    }
}

suspend fun buildSdk(
    config: breez_sdk_spark.Config,
    seed: Seed,
    handlers: SharedHandlers,
): BreezSdk {
    val builder = SdkBuilder(config, seed)
    builder.withSharedContext(handlers.context)
    return builder.build()
}

suspend fun BreezSdk.syncedInfo(): breez_sdk_spark.GetInfoResponse {
    syncWallet(SyncWalletRequest)
    return getInfo(GetInfoRequest(ensureSynced = false))
}

// --- per-request SDK lifecycle --------------------------------------------

/**
 * Builds a fresh SDK per call (sharing transports via [handlers]) and tears
 * it down after `op`. Concurrent requests for the same `userId` run in
 * parallel — there is no per-user serialization.
 */
class BenchSdkProvider(
    private val masterSecret: String,
    private val handlers: SharedHandlers,
) {
    private val config = benchConfig()

    suspend fun <T> withUser(
        userId: String,
        timings: RequestTimings? = null,
        op: suspend (BreezSdk) -> T,
    ): T {
        val seed: Seed = Seed.Entropy(deriveSeedBytes(masterSecret, userId))
        val tBuildNs = System.nanoTime()
        val sdk = buildSdk(config, seed, handlers)
        timings?.buildMs = (System.nanoTime() - tBuildNs) / 1_000_000
        return try {
            val tOpNs = System.nanoTime()
            val r = op(sdk)
            timings?.opMs = (System.nanoTime() - tOpNs) / 1_000_000
            r
        } finally {
            val tDiscNs = System.nanoTime()
            try {
                sdk.disconnect()
            } catch (e: Exception) {
                System.err.println("[server] disconnect warn (user=$userId): ${e.message}")
            }
            timings?.disconnectMs = (System.nanoTime() - tDiscNs) / 1_000_000
        }
    }
}

// --- HTTP request/response shapes -----------------------------------------

@Serializable
data class InfoResponse(val balanceSats: Long)

@Serializable
data class SendBody(val destination: String, val amountSats: Long)

@Serializable
data class SendResult(val paymentId: String, val feeSats: String)

@Serializable
data class ReceiveBody(
    val method: String? = null,
    val amountSats: Long? = null,
)

@Serializable
data class ReceiveResult(val paymentRequest: String, val feeSats: String)

@Serializable
data class ErrorBody(val error: String)

@Serializable
data class ServerRequestLogEntry(
    val ts: Long,
    val op: String,
    @SerialName("user_id") val userId: String,
    @SerialName("duration_ms") val durationMs: Long,
    val error: String? = null,
    @SerialName("build_ms") val buildMs: Long? = null,
    @SerialName("op_ms") val opMs: Long? = null,
    @SerialName("prepare_ms") val prepareMs: Long? = null,
    @SerialName("send_ms") val sendMs: Long? = null,
    @SerialName("disconnect_ms") val disconnectMs: Long? = null,
)

class RequestTimings {
    var buildMs: Long? = null
    var opMs: Long? = null
    var prepareMs: Long? = null
    var sendMs: Long? = null
    var disconnectMs: Long? = null
    // Post-classified op label, set after the SDK call resolves spark-vs-LN.
    var opOverride: String? = null
}

// --- reserved user-ids (funding pipeline) ---------------------------------

const val TREASURER_USER_ID = "__treasurer__"

fun senderUserId(idx: Int): String = "__sender_${idx}__"

fun bankUserId(idx: Int): String = "__bank_${idx}__"

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

// --- fund mode (treasurer top-up via Lightspark regtest faucet) -----------

/**
 * Idempotent treasurer top-up: walks the treasurer's balance up to
 * `targetSats` by repeatedly hitting the faucet (capped at
 * [Faucet.MAX_PER_CALL_SATS] per call) and waiting for each on-chain
 * deposit to be claimed before requesting the next chunk.
 *
 * If the treasurer is already at or above `targetSats`, exits without
 * calling the faucet — safe to re-run.
 */
fun fundTreasurer(opts: Map<String, String>) = runBlocking {
    val mysqlUrl = opts["mysql-url"]
        ?: error("--mysql-url=mysql://user:pass@host:port/dbname is required")
    val masterSecret = opts["master-secret"]
        ?: System.getenv("MASTER_SECRET")
        ?: error("--master-secret=<hex-or-string> or MASTER_SECRET env var is required")
    val targetSats = opts["target-sats"]?.toLongOrNull() ?: 5_000_000L

    System.getenv("FAUCET_USERNAME") ?: error("FAUCET_USERNAME env var is required")
    System.getenv("FAUCET_PASSWORD") ?: error("FAUCET_PASSWORD env var is required")

    println("[fund] treasurer top-up to $targetSats sats  mysql=${maskPassword(mysqlUrl)}")

    val seed: Seed = Seed.Entropy(deriveSeedBytes(masterSecret, TREASURER_USER_ID))
    val handlers = SharedHandlers.create(mysqlUrl)
    val sdk = buildSdk(benchConfig(), seed, handlers)

    try {
        // Skip sync — local balance is a lower bound, sufficient for ≥-target check.
        val cachedBalance = sdk.getInfo(GetInfoRequest(ensureSynced = false)).balanceSats.toLong()
        if (cachedBalance >= targetSats) {
            println("[fund] cached balance: $cachedBalance sats (≥ $targetSats target, skipping sync)")
            println("[fund] OK")
            return@runBlocking
        }
        println("[fund] cached balance: $cachedBalance sats (below $targetSats; syncing to confirm)")

        // Reuse an existing deposit address if the treasurer has one.
        val depositAddr = sdk.receivePayment(
            ReceivePaymentRequest(paymentMethod = ReceivePaymentMethod.BitcoinAddress(newAddress = false))
        ).paymentRequest
        println("[fund] deposit address: $depositAddr")

        var chunkIdx = 0
        while (true) {
            val info = sdk.syncedInfo()
            val balance = info.balanceSats.toLong()
            if (balance >= targetSats) {
                println("[fund] treasurer balance: $balance sats (target reached)")
                break
            }
            val remaining = targetSats - balance
            var chunk = remaining.coerceAtMost(Faucet.MAX_PER_CALL_SATS)
            if (chunk < Faucet.MIN_PER_CALL_SATS) chunk = Faucet.MIN_PER_CALL_SATS
            chunkIdx++
            println("[fund] chunk #$chunkIdx: requesting $chunk sats (balance $balance/$targetSats)")
            val txid = Faucet.fundBitcoinAddress(depositAddr, chunk)
            println("[fund] chunk #$chunkIdx faucet txid: $txid")
            waitForBalanceIncrease(
                sdk,
                balance.toULong(),
                timeoutMs = 240_000,
                pollLabel = "[fund] chunk #$chunkIdx",
            )
        }
        println("[fund] OK")
    } finally {
        try {
            sdk.disconnect()
        } catch (e: Exception) {
            System.err.println("[fund] disconnect warn: ${e.message}")
        }
    }
}

private suspend fun waitForBalanceIncrease(
    sdk: BreezSdk,
    currentBalance: ULong,
    timeoutMs: Long,
    pollLabel: String,
) {
    val startMs = System.currentTimeMillis()
    val deadline = startMs + timeoutMs
    var nextLogAtMs = startMs + 10_000
    while (System.currentTimeMillis() < deadline) {
        delay(5_000)
        val info = sdk.syncedInfo()
        if (info.balanceSats > currentBalance) return
        val now = System.currentTimeMillis()
        if (now >= nextLogAtMs) {
            val elapsedSec = (now - startMs) / 1000
            val timeoutSec = timeoutMs / 1000
            println("$pollLabel waiting for balance increase… elapsed=${elapsedSec}s/${timeoutSec}s (still $currentBalance sats)")
            nextLogAtMs = now + 10_000
        }
    }
    error("Balance did not increase within ${timeoutMs}ms (was $currentBalance sats)")
}

// --- seed-senders mode (top up sender pool from treasurer) ----------------

fun seedSenders(opts: Map<String, String>) = runBlocking {
    val mysqlUrl = opts["mysql-url"]
        ?: error("--mysql-url=mysql://user:pass@host:port/dbname is required")
    val masterSecret = opts["master-secret"]
        ?: System.getenv("MASTER_SECRET")
        ?: error("--master-secret=<hex-or-string> or MASTER_SECRET env var is required")
    val senderCount = opts["senders"]?.toIntOrNull() ?: 50
    val perSenderSats = opts["per-sender-sats"]?.toLongOrNull() ?: 5_000L
    val parallelism = opts["parallelism"]?.toIntOrNull() ?: 5

    require(senderCount > 0) { "--senders must be > 0" }
    require(perSenderSats > 0) { "--per-sender-sats must be > 0" }
    require(parallelism > 0) { "--parallelism must be > 0" }

    println(
        "[seed] senders=$senderCount  per-sender=$perSenderSats  parallel=$parallelism  " +
            "mysql=${maskPassword(mysqlUrl)}"
    )

    val config = benchConfig()
    val handlers = SharedHandlers.create(mysqlUrl)

    val treasurerSeed: Seed = Seed.Entropy(deriveSeedBytes(masterSecret, TREASURER_USER_ID))
    val treasurer = buildSdk(benchConfig(), treasurerSeed, handlers)

    try {
        // Lower bound on true balance (sync only adds incoming) — fine for the warning below.
        val treasurerInfo = treasurer.getInfo(GetInfoRequest(ensureSynced = false))
        val treasurerBalance = treasurerInfo.balanceSats.toLong()
        println("[seed] treasurer balance (cached): $treasurerBalance sats")
        val maxSpend = senderCount.toLong() * perSenderSats
        if (treasurerBalance < maxSpend) {
            System.err.println(
                "[seed] warning: treasurer has $treasurerBalance sats; up to $maxSpend may be needed " +
                    "if all senders are empty. Run 'make fund' first."
            )
        }

        val sem = Semaphore(parallelism)
        var fundedCount = 0
        var skippedCount = 0
        var failedCount = 0
        coroutineScope {
            for (i in 0 until senderCount) {
                launch {
                    sem.withPermit {
                        val outcome = try {
                            seedOneSender(
                                treasurer = treasurer,
                                senderIdx = i,
                                masterSecret = masterSecret,
                                config = config,
                                handlers = handlers,
                                perSenderSats = perSenderSats,
                            )
                        } catch (e: CancellationException) {
                            throw e
                        } catch (e: Exception) {
                            // Swallow per-sender failure; non-zero exit at the end, re-run is idempotent.
                            System.err.println("[seed] sender $i FAILED: ${e.message}")
                            SeedOutcome.FAILED
                        }
                        synchronized(this@runBlocking) {
                            when (outcome) {
                                SeedOutcome.FUNDED -> fundedCount++
                                SeedOutcome.SKIPPED -> skippedCount++
                                SeedOutcome.FAILED -> failedCount++
                            }
                        }
                    }
                }
            }
        }
        println("[seed] funded=$fundedCount  skipped=$skippedCount  failed=$failedCount")
        if (failedCount > 0) {
            error("[seed] $failedCount sender(s) still unfunded after this pass")
        }
        println("[seed] OK")
    } finally {
        try {
            treasurer.disconnect()
        } catch (e: Exception) {
            System.err.println("[seed] treasurer disconnect warn: ${e.message}")
        }
    }
}

private enum class SeedOutcome { FUNDED, SKIPPED, FAILED }

private suspend fun seedOneSender(
    treasurer: BreezSdk,
    senderIdx: Int,
    masterSecret: String,
    config: breez_sdk_spark.Config,
    handlers: SharedHandlers,
    perSenderSats: Long,
): SeedOutcome {
    val userId = senderUserId(senderIdx)
    val seed: Seed = Seed.Entropy(deriveSeedBytes(masterSecret, userId))
    val sender = buildSdk(config, seed, handlers)

    return try {
        val info = sender.syncedInfo()
        val balance = info.balanceSats.toLong()
        if (balance >= perSenderSats) {
            println("[seed] sender $senderIdx: $balance sats (≥ $perSenderSats, skip)")
            return SeedOutcome.SKIPPED
        }
        val sparkAddr = sender.receivePayment(
            ReceivePaymentRequest(paymentMethod = ReceivePaymentMethod.SparkAddress)
        ).paymentRequest
        val toSend = perSenderSats - balance
        println("[seed] sender $senderIdx: $balance sats → topping up by $toSend to $perSenderSats")

        val t0 = System.currentTimeMillis()
        val prepared = treasurer.prepareSendPayment(
            PrepareSendPaymentRequest(
                paymentRequest = sparkAddr,
                amount = BigInteger.fromLong(toSend),
            )
        )
        treasurer.sendPayment(SendPaymentRequest(prepareResponse = prepared))
        println("[seed] sender $senderIdx: treasurer send ${System.currentTimeMillis() - t0}ms")

        println("[seed] sender $senderIdx: waitForBalanceIncrease …")
        waitForBalanceIncrease(
            sender,
            balance.toULong(),
            timeoutMs = 60_000,
            pollLabel = "[seed] sender $senderIdx",
        )
        SeedOutcome.FUNDED
    } finally {
        try {
            sender.disconnect()
        } catch (e: Exception) {
            System.err.println("[seed] sender $senderIdx disconnect warn: ${e.message}")
        }
    }
}

// --- mint-invoices mode (pre-mint bolt11 invoice pool for LN sends) -------

// Pre-mints a bolt11 pool + probes the LN fee (writes `<out>.fee`). Idempotent.
fun mintInvoices(opts: Map<String, String>) = runBlocking {
    val mysqlUrl = opts["mysql-url"]
        ?: error("--mysql-url=mysql://user:pass@host:port/dbname is required")
    val masterSecret = opts["master-secret"]
        ?: System.getenv("MASTER_SECRET")
        ?: error("--master-secret=<hex-or-string> or MASTER_SECRET env var is required")
    val count = opts["count"]?.toIntOrNull()
        ?: error("--count=<N> is required (number of invoices to mint)")
    val amountSats = opts["amount-sats"]?.toULongOrNull()
        ?: error("--amount-sats=<N> is required (fixed amount per invoice)")
    val banks = opts["banks"]?.toIntOrNull() ?: 50
    val expirySecs = opts["expiry-secs"]?.toUIntOrNull() ?: 604_800u  // 7 days
    val parallelism = opts["parallelism"]?.toIntOrNull() ?: 20
    val poolPath = opts["out"] ?: error("--out=<path> is required (pool file path)")
    val feePath = "$poolPath.fee"

    require(count > 0) { "--count must be > 0" }
    require(banks >= 2) { "--banks must be >= 2 (one for the fee probe payer)" }
    require(parallelism > 0) { "--parallelism must be > 0" }

    val poolFile = Path.of(poolPath)
    val feeFile = Path.of(feePath)
    poolFile.toAbsolutePath().parent?.let { Files.createDirectories(it) }

    // Idempotent skip: pool already big enough + fee already probed.
    if (Files.exists(poolFile) && Files.exists(feeFile)) {
        val existing = Files.newBufferedReader(poolFile).use { r ->
            r.lines().filter { it.isNotBlank() }.count()
        }
        if (existing >= count) {
            val cachedFee = Files.readString(feeFile).trim()
            println("[mint] pool already has $existing invoices (≥ $count requested), skipping")
            println("[mint] ln_fee_sats=$cachedFee")
            println("[mint] OK")
            return@runBlocking
        }
        println("[mint] pool has $existing invoices, need $count; re-minting")
    }

    println("[mint] count=$count  banks=$banks  amount=${amountSats}sat  expiry=${expirySecs}s  parallel=$parallelism")
    println("[mint] mysql=${maskPassword(mysqlUrl)}  out=$poolPath")

    val handlers = SharedHandlers.create(mysqlUrl)
    val config = benchConfig()

    println("[mint] building $banks bank SDKs in parallel …")
    val tBankBuild = System.currentTimeMillis()
    val bankSdks: List<BreezSdk> = coroutineScope {
        (0 until banks).map { idx ->
            async {
                val seed: Seed = Seed.Entropy(deriveSeedBytes(masterSecret, bankUserId(idx)))
                buildSdk(config, seed, handlers)
            }
        }.awaitAll()
    }
    println("[mint] $banks bank SDKs ready (${System.currentTimeMillis() - tBankBuild}ms)")

    try {
        Files.newBufferedWriter(
            poolFile,
            StandardOpenOption.CREATE,
            StandardOpenOption.TRUNCATE_EXISTING,
        ).use { writer ->
            val writerLock = Mutex()
            val sem = Semaphore(parallelism)
            val minted = AtomicInteger(0)
            val failed = AtomicInteger(0)
            val tStart = System.currentTimeMillis()
            // Transient SSP rate-limits / operator blips dominate failures; retry with backoff.
            val retried = AtomicInteger(0)
            coroutineScope {
                for (i in 0 until count) {
                    launch {
                        sem.withPermit {
                            val bankSdk = bankSdks[i % banks]
                            val maxAttempts = 8
                            var attempt = 0
                            while (true) {
                                attempt++
                                try {
                                    val resp = bankSdk.receivePayment(
                                        ReceivePaymentRequest(
                                            paymentMethod = ReceivePaymentMethod.Bolt11Invoice(
                                                description = "bench-pool",
                                                amountSats = amountSats,
                                                expirySecs = expirySecs,
                                                paymentHash = null,
                                            )
                                        )
                                    )
                                    writerLock.withLock {
                                        writer.write(resp.paymentRequest)
                                        writer.newLine()
                                        writer.flush()
                                    }
                                    val n = minted.incrementAndGet()
                                    if (n % 200 == 0 || n == count) {
                                        val elapsedMs = (System.currentTimeMillis() - tStart).coerceAtLeast(1)
                                        val rate = 1000.0 * n / elapsedMs
                                        val etaSec = if (rate > 0) ((count - n) / rate).toLong() else 0
                                        println("[mint] $n/$count  (${"%.1f".format(rate)}/s, ETA ${etaSec}s, retries=${retried.get()})")
                                    }
                                    return@withPermit
                                } catch (e: CancellationException) {
                                    throw e
                                } catch (e: Exception) {
                                    if (attempt < maxAttempts) {
                                        retried.incrementAndGet()
                                        // Exponential backoff with jitter: 200ms × 2^(attempt-1) ± 50%.
                                        val baseMs = 200L shl (attempt - 1).coerceAtMost(6)  // cap 2^6×200 = 12.8s
                                        val jitter = (baseMs * (kotlin.random.Random.nextDouble() - 0.5)).toLong()
                                        delay(baseMs + jitter)
                                        continue
                                    }
                                    failed.incrementAndGet()
                                    System.err.println("[mint] invoice $i FAILED after $attempt attempt(s): ${e.message}")
                                    return@withPermit
                                }
                            }
                            @Suppress("UNREACHABLE_CODE")
                            Unit
                        }
                    }
                }
            }
            if (retried.get() > 0) {
                println("[mint] ${retried.get()} mint(s) retried (SSP rate-limit transient)")
            }
            if (failed.get() > 0) {
                error("[mint] ${failed.get()} invoice(s) failed after retries; re-run to retry (idempotent)")
            }
        }
        println("[mint] minted=$count  pool=$poolPath")

        // LN fee probe: bank #0 mints, bank #1 prepares (never sends).
        println("[mint] LN fee probe …")
        val probeReceiver = bankSdks[0]
        val probePayer = bankSdks[1]
        val probeInvoice = probeReceiver.receivePayment(
            ReceivePaymentRequest(
                paymentMethod = ReceivePaymentMethod.Bolt11Invoice(
                    description = "bench-fee-probe",
                    amountSats = amountSats,
                    expirySecs = expirySecs,
                    paymentHash = null,
                )
            )
        ).paymentRequest
        val prepared = probePayer.prepareSendPayment(
            PrepareSendPaymentRequest(
                paymentRequest = probeInvoice,
                amount = null,  // amount embedded in the fixed-amount invoice
            )
        )
        val fee = when (val pm = prepared.paymentMethod) {
            is SendPaymentMethod.Bolt11Invoice -> pm.lightningFeeSats
            else -> error("[mint] fee probe got unexpected paymentMethod: ${pm::class.simpleName}")
        }
        Files.writeString(feeFile, fee.toString())
        println("[mint] ln_fee_sats=$fee")
        println("[mint] OK")
    } finally {
        for ((idx, sdk) in bankSdks.withIndex()) {
            try {
                sdk.disconnect()
            } catch (e: Exception) {
                System.err.println("[mint] bank $idx disconnect warn: ${e.message}")
            }
        }
    }
}

// --- audit-bolt11 mode (validate that send_ln dispatches actually settled) -

@Serializable
data class AuditStepReport(
    val rps: Int,
    val expected: Int,
    val completed: Int,
    val pending: Int,
    val failed: Int,
    @SerialName("not_found") val notFound: Int,
)

@Serializable
data class AuditSenderReport(
    @SerialName("user_id") val userId: String,
    val expected: Int,
    val completed: Int,
    val pending: Int,
    val failed: Int,
    @SerialName("not_found") val notFound: Int,
)

@Serializable
data class AuditDoc(
    @SerialName("pool_size") val poolSize: Int,
    @SerialName("expected_total") val expectedTotal: Int,
    val completed: Int,
    val pending: Int,
    val failed: Int,
    @SerialName("not_found") val notFound: Int,
    /** Fraction completed of expected; in [0, 1]. */
    @SerialName("settled_rate") val settledRate: Double,
    @SerialName("per_step") val perStep: List<AuditStepReport>,
    @SerialName("per_sender") val perSender: List<AuditSenderReport>,
)

fun auditBolt11(opts: Map<String, String>) = runBlocking {
    val mysqlUrl = opts["mysql-url"]
        ?: error("--mysql-url=mysql://user:pass@host:port/dbname is required")
    val masterSecret = opts["master-secret"]
        ?: System.getenv("MASTER_SECRET")
        ?: error("--master-secret=<hex-or-string> or MASTER_SECRET env var is required")
    val sweepDirArg = opts["sweep-dir"]
        ?: error("--sweep-dir=<path> is required (e.g. out/<sweep-id>)")
    val parallelism = opts["parallelism"]?.toIntOrNull() ?: 5
    require(parallelism > 0) { "--parallelism must be > 0" }

    val sweepDir = Path.of(sweepDirArg)
    require(Files.isDirectory(sweepDir)) { "$sweepDir is not a directory" }
    val poolPath = sweepDir.resolve("invoices.txt")
    require(Files.exists(poolPath)) { "no invoice pool at $poolPath — not a bolt11 sweep dir?" }

    val pool = Files.readAllLines(poolPath).map { it.trim() }.filter { it.isNotEmpty() }
    println("[audit] pool size: ${pool.size}")

    val stepDirs = Files.list(sweepDir).use { stream ->
        stream.filter {
            val n = it.fileName.toString()
            n.startsWith("rps-") && Files.isDirectory(it) &&
                n.removePrefix("rps-").toIntOrNull() != null
        }.toList()
    }.sortedBy { it.fileName.toString().removePrefix("rps-").toInt() }
    println("[audit] step dirs: ${stepDirs.size}")

    val codec = Json { ignoreUnknownKeys = true }
    data class Expected(val rps: Int, val userId: String, val invoiceIdx: Int, val invoice: String)
    val expected = mutableListOf<Expected>()

    for (dir in stepDirs) {
        val rps = dir.fileName.toString().removePrefix("rps-").toInt()
        val latencyPath = dir.resolve("latency.jsonl")
        if (!Files.exists(latencyPath)) continue
        Files.newBufferedReader(latencyPath).use { r ->
            r.lineSequence().forEach inner@{ line ->
                if (line.isBlank()) return@inner
                val obj = codec.parseToJsonElement(line).jsonObject
                if (obj["op"]?.jsonPrimitive?.contentOrNull != "send_ln") return@inner
                if (obj["dropped"]?.jsonPrimitive?.booleanOrNull == true) return@inner
                val idx = obj["invoice_idx"]?.jsonPrimitive?.intOrNull ?: return@inner
                val uid = obj["user_id"]?.jsonPrimitive?.contentOrNull ?: return@inner
                if (idx !in pool.indices) return@inner
                expected.add(Expected(rps, uid, idx, pool[idx]))
            }
        }
    }
    println("[audit] expected non-dropped send_ln dispatches: ${expected.size}")
    if (expected.isEmpty()) {
        // Nothing to audit — write a trivially-empty doc and return.
        val emptyDoc = AuditDoc(pool.size, 0, 0, 0, 0, 0, 0.0, emptyList(), emptyList())
        Files.writeString(
            sweepDir.resolve("audit.json"),
            Json { prettyPrint = true }.encodeToString(AuditDoc.serializer(), emptyDoc),
        )
        println("[audit] no send_ln dispatches to audit; wrote empty audit.json")
        return@runBlocking
    }

    val perSender: Map<String, List<Expected>> = expected.groupBy { it.userId }
    println("[audit] unique senders: ${perSender.size}")

    val handlers = SharedHandlers.create(mysqlUrl)
    val config = benchConfig()

    val auditMap = ConcurrentHashMap<String, Map<String, PaymentStatus>>()
    val sem = Semaphore(parallelism)
    coroutineScope {
        for ((uid, _) in perSender) {
            launch {
                sem.withPermit {
                    val seed: Seed = Seed.Entropy(deriveSeedBytes(masterSecret, uid))
                    val sdk = buildSdk(config, seed, handlers)
                    try {
                        val tSync = System.currentTimeMillis()
                        sdk.syncWallet(SyncWalletRequest)
                        val syncMs = System.currentTimeMillis() - tSync
                        val resp = sdk.listPayments(
                            ListPaymentsRequest(
                                typeFilter = listOf(PaymentType.SEND),
                                paymentDetailsFilter = listOf(
                                    PaymentDetailsFilter.Lightning(htlcStatus = null)
                                ),
                                limit = 100_000u,
                            )
                        )
                        val byInvoice = HashMap<String, PaymentStatus>(resp.payments.size)
                        for (p in resp.payments) {
                            val det = p.details as? PaymentDetails.Lightning ?: continue
                            byInvoice[det.invoice] = p.status
                        }
                        auditMap[uid] = byInvoice
                        println("[audit] sender $uid: sync ${syncMs}ms, ${byInvoice.size} LN sends on wallet")
                    } catch (e: CancellationException) {
                        throw e
                    } catch (e: Exception) {
                        System.err.println("[audit] sender $uid FAILED: ${e.message}")
                        auditMap[uid] = emptyMap()
                    } finally {
                        try {
                            sdk.disconnect()
                        } catch (e: Exception) {
                            System.err.println("[audit] sender $uid disconnect warn: ${e.message}")
                        }
                    }
                }
            }
        }
    }

    data class Counts(var c: Int = 0, var p: Int = 0, var f: Int = 0, var nf: Int = 0) {
        fun add(s: PaymentStatus?) = when (s) {
            PaymentStatus.COMPLETED -> c++
            PaymentStatus.PENDING -> p++
            PaymentStatus.FAILED -> f++
            null -> nf++
        }
    }

    val total = Counts()
    val perStepCounts = HashMap<Int, Counts>()
    val perSenderCounts = HashMap<String, Counts>()
    for (e in expected) {
        val status = auditMap[e.userId]?.get(e.invoice)
        total.add(status)
        perStepCounts.getOrPut(e.rps) { Counts() }.add(status)
        perSenderCounts.getOrPut(e.userId) { Counts() }.add(status)
    }

    val perStep = perStepCounts.toSortedMap().map { (rps, c) ->
        val n = c.c + c.p + c.f + c.nf
        AuditStepReport(rps, n, c.c, c.p, c.f, c.nf)
    }
    val perSenderOut = perSenderCounts.toSortedMap().map { (uid, c) ->
        val n = c.c + c.p + c.f + c.nf
        AuditSenderReport(uid, n, c.c, c.p, c.f, c.nf)
    }
    val doc = AuditDoc(
        poolSize = pool.size,
        expectedTotal = expected.size,
        completed = total.c,
        pending = total.p,
        failed = total.f,
        notFound = total.nf,
        settledRate = total.c.toDouble() / expected.size,
        perStep = perStep,
        perSender = perSenderOut,
    )

    val outPath = sweepDir.resolve("audit.json")
    Files.writeString(
        outPath,
        Json { prettyPrint = true }.encodeToString(AuditDoc.serializer(), doc),
    )

    val pct = 100.0 * total.c / expected.size
    println("[audit] expected=${expected.size}  completed=${total.c}  pending=${total.p}  " +
        "failed=${total.f}  not_found=${total.nf}  settled=${"%.1f".format(pct)}%")
    println("[audit] per-step:")
    for (s in perStep) {
        val sp = if (s.expected > 0) 100.0 * s.completed / s.expected else 0.0
        println("[audit]   rps=${s.rps}: expected=${s.expected}  completed=${s.completed}  " +
            "pending=${s.pending}  failed=${s.failed}  not_found=${s.notFound}  " +
            "settled=${"%.1f".format(sp)}%")
    }
    println("[audit] wrote $outPath")
}

// --- server mode ----------------------------------------------------------

fun runServer(opts: Map<String, String>) {
    val mysqlUrl = opts["mysql-url"]
        ?: error("--mysql-url=mysql://user:pass@host:port/dbname is required")
    val mysqlParts = parseMysqlUrl(mysqlUrl)
    val masterSecret = opts["master-secret"]
        ?: System.getenv("MASTER_SECRET")
        ?: error("--master-secret=<hex-or-string> or MASTER_SECRET env var is required")
    val port = opts["port"]?.toIntOrNull() ?: 8080
    val runId = opts["run-id"] ?: defaultRunId()
    val outDir = Path.of(opts["out-dir"] ?: "out/$runId").also { Files.createDirectories(it) }

    opts["log-filter"]?.let { logFilter ->
        val logDir = opts["log-dir"] ?: outDir.resolve(".trace-logs").toString()
        Files.createDirectories(Path.of(logDir))
        println("[server] init_logging dir=$logDir filter=$logFilter")
        initLogging(logDir, null, logFilter)
    }

    val handlers = runBlocking { SharedHandlers.create(mysqlUrl) }
    val provider = BenchSdkProvider(masterSecret, handlers)

    val requestsWriter = JsonlWriter(outDir.resolve("requests.jsonl"), ServerRequestLogEntry.serializer())
    val metricsWriter = JsonlWriter(outDir.resolve("metrics.jsonl"), MetricSample.serializer())
    val mysqlPoller = MysqlConnPoller(mysqlParts)
    val sampler = MetricsSampler(
        collector = ProcessMetricsCollector.create(),
        mysqlPoller = mysqlPoller,
        writer = metricsWriter,
    )
    sampler.start()

    // Flush JSONL writers on Ctrl-C.
    Runtime.getRuntime().addShutdownHook(Thread {
        sampler.stop()
        try { mysqlPoller.close() } catch (_: Exception) {}
        try { requestsWriter.close() } catch (_: Exception) {}
        try { metricsWriter.close() } catch (_: Exception) {}
    })

    println("[server] run-id=$runId  out=$outDir")
    println("[server] listening on :$port  mysql=${maskPassword(mysqlUrl)}")

    embeddedServer(Netty, port = port) {
        install(ContentNegotiation) { json() }
        routing {
            get("/users/{userId}/info") {
                val userId = call.parameters["userId"]!!
                handleAndLog(call, "info", userId, requestsWriter) { t ->
                    provider.withUser(userId, t) { sdk ->
                        val info = sdk.getInfo(GetInfoRequest(ensureSynced = false))
                        InfoResponse(balanceSats = info.balanceSats.toLong())
                    }
                }
            }

            post("/users/{userId}/send") {
                val userId = call.parameters["userId"]!!
                val body = call.receive<SendBody>()
                handleAndLog(call, "send", userId, requestsWriter) { t ->
                    provider.withUser(userId, t) { sdk ->
                        val tPrepNs = System.nanoTime()
                        val prepared = sdk.prepareSendPayment(
                            PrepareSendPaymentRequest(
                                paymentRequest = body.destination,
                                amount = BigInteger.fromLong(body.amountSats),
                            )
                        )
                        t.prepareMs = (System.nanoTime() - tPrepNs) / 1_000_000
                        val sendOptions: SendPaymentOptions? = when (prepared.paymentMethod) {
                            is SendPaymentMethod.Bolt11Invoice -> {
                                t.opOverride = "send_ln"
                                SendPaymentOptions.Bolt11Invoice(
                                    preferSpark = false,
                                    completionTimeoutSecs = 0u,
                                )
                            }
                            else -> null
                        }
                        val tSendNs = System.nanoTime()
                        val sendResp = sdk.sendPayment(
                            SendPaymentRequest(prepareResponse = prepared, options = sendOptions)
                        )
                        t.sendMs = (System.nanoTime() - tSendNs) / 1_000_000
                        SendResult(
                            paymentId = sendResp.payment.id,
                            feeSats = feeOf(prepared.paymentMethod),
                        )
                    }
                }
            }

            post("/users/{userId}/receive") {
                val userId = call.parameters["userId"]!!
                val body = runCatching { call.receive<ReceiveBody>() }.getOrElse { ReceiveBody() }
                handleAndLog(call, "receive", userId, requestsWriter) { t ->
                    provider.withUser(userId, t) { sdk ->
                        val method: ReceivePaymentMethod = when (body.method?.lowercase()) {
                            "bolt11", "ln", "lightning" -> {
                                t.opOverride = "receive_ln"
                                ReceivePaymentMethod.Bolt11Invoice(
                                    description = "bench",
                                    amountSats = body.amountSats?.toULong(),
                                    expirySecs = 604_800u,  // 7 days; well under SDK 30d max
                                    paymentHash = null,
                                )
                            }
                            null, "", "spark", "spark_address", "sparkaddress" -> ReceivePaymentMethod.SparkAddress
                            else -> error("unknown receive method: ${body.method}")
                        }
                        val resp = sdk.receivePayment(ReceivePaymentRequest(paymentMethod = method))
                        ReceiveResult(
                            paymentRequest = resp.paymentRequest,
                            feeSats = resp.fee.toString(),
                        )
                    }
                }
            }
        }
    }.start(wait = true)
}

private fun feeOf(pm: SendPaymentMethod): String = when (pm) {
    is SendPaymentMethod.SparkAddress -> pm.fee.toString()
    is SendPaymentMethod.SparkInvoice -> pm.fee.toString()
    is SendPaymentMethod.Bolt11Invoice -> pm.lightningFeeSats.toString()
    is SendPaymentMethod.BitcoinAddress -> {
        val q = pm.feeQuote.speedFast
        (q.userFeeSat + q.l1BroadcastFeeSat).toString()
    }
}

private suspend inline fun <reified T : Any> handleAndLog(
    call: io.ktor.server.application.ApplicationCall,
    op: String,
    userId: String,
    requestsWriter: JsonlWriter<ServerRequestLogEntry>,
    crossinline block: suspend (RequestTimings) -> T,
) {
    val tsMs = System.currentTimeMillis()
    val tStartNs = System.nanoTime()
    val timings = RequestTimings()
    var errStr: String? = null
    try {
        call.respond(block(timings))
    } catch (e: Throwable) {
        errStr = "${e::class.simpleName}: ${e.message ?: ""}"
        System.err.println("[server] handler error (op=$op user=$userId): ${e.message}")
        call.respond(
            HttpStatusCode.InternalServerError,
            ErrorBody(error = e.message ?: e::class.qualifiedName ?: "error"),
        )
    } finally {
        requestsWriter.submit(
            ServerRequestLogEntry(
                ts = tsMs,
                op = timings.opOverride ?: op,
                userId = userId,
                durationMs = (System.nanoTime() - tStartNs) / 1_000_000,
                error = errStr,
                buildMs = timings.buildMs,
                opMs = timings.opMs,
                prepareMs = timings.prepareMs,
                sendMs = timings.sendMs,
                disconnectMs = timings.disconnectMs,
            )
        )
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
