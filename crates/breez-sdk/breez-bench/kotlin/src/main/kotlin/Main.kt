import breez_sdk_spark.BreezSdk
import breez_sdk_spark.GetInfoRequest
import breez_sdk_spark.Network
import breez_sdk_spark.PrepareSendPaymentRequest
import breez_sdk_spark.ReceivePaymentMethod
import breez_sdk_spark.ReceivePaymentRequest
import breez_sdk_spark.SdkBuilder
import breez_sdk_spark.SdkContext
import breez_sdk_spark.SdkContextConfig
import breez_sdk_spark.Seed
import breez_sdk_spark.SendPaymentMethod
import breez_sdk_spark.SendPaymentRequest
import breez_sdk_spark.SyncWalletRequest
import breez_sdk_spark.defaultMysqlStorageConfig
import breez_sdk_spark.defaultServerConfig
import breez_sdk_spark.initLogging
import breez_sdk_spark.newSharedSdkContext

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Semaphore
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
import java.util.concurrent.ConcurrentHashMap

import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

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

/**
 * Server-mode regtest config. This bench is the server-mode use case (one
 * ephemeral SDK per request), so it uses `defaultServerConfig`:
 * `backgroundTasksEnabled = false`, no per-request sync loop / optimizer /
 * RT-sync websocket. `ensureSynced=true` is rejected in this mode; the
 * bench syncs explicitly via [syncedInfo] in the funding paths only.
 */
fun benchConfig(): breez_sdk_spark.Config = defaultServerConfig(Network.REGTEST)

// --- shared SDK transports ------------------------------------------------

/**
 * Process-wide shared resources, constructed once and threaded into every
 * `SdkBuilder`. One [SdkContext] bundles the shared HTTP client (SSP,
 * LNURL, JWT, and the chain service), the operator gRPC channels, the
 * Breez-backend gRPC client, and the MySQL pool. Without sharing, every
 * per-request build would open its own pool, reopen SSP TCP/TLS, and
 * redial operators — exhausting FDs / ephemeral ports under load.
 */
class SharedHandlers private constructor(
    val context: SdkContext,
) {
    companion object {
        suspend fun create(mysqlUrl: String): SharedHandlers {
            val mysqlConfig = defaultMysqlStorageConfig(mysqlUrl).also {
                it.recycleTimeoutSecs = 300UL
            }
            // CONNS_PER_OPERATOR: gRPC connections per operator, shared
            // across every SDK. Unset → null (one multiplexed connection,
            // the production default); set → fan out, to probe whether
            // that single connection caps throughput at the top of a sweep.
            val connsPerOperator: UInt? = System.getenv("CONNS_PER_OPERATOR")?.let {
                it.toUIntOrNull()?.takeIf { n -> n > 0u }
                    ?: error("CONNS_PER_OPERATOR must be a positive integer; got '$it'")
            }
            // network must match the SDK Config or build() rejects it.
            // mysqlConfig is the single DB source — don't also call
            // withMysqlConnectionPool (build() errors on a double source).
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

/**
 * Builds an SDK for [seed] on the shared [SharedHandlers.context]. Chain
 * service left unset so `build()` routes it over the context HTTP client.
 * The session store is auto-wired on the context's MySQL pool.
 */
suspend fun buildSdk(
    config: breez_sdk_spark.Config,
    seed: Seed,
    handlers: SharedHandlers,
): BreezSdk {
    val builder = SdkBuilder(config, seed)
    builder.withSharedContext(handlers.context)
    return builder.build()
}

/**
 * Explicit `syncWallet()` + local `getInfo` — the server-mode stand-in
 * for `getInfo(ensureSynced=true)`, which is rejected when background
 * tasks are off. Used only by the funding/diagnostic paths; user-facing
 * handlers do a pure local read (no wallet receives mid-run).
 */
suspend fun BreezSdk.syncedInfo(): breez_sdk_spark.GetInfoResponse {
    syncWallet(SyncWalletRequest)
    return getInfo(GetInfoRequest(ensureSynced = false))
}

// --- per-request SDK lifecycle --------------------------------------------

/**
 * Builds a fresh SDK per call (sharing transports via [handlers]) and tears
 * it down after `op`. Same-`userId` calls serialize through a per-userId
 * mutex so concurrent requests never race two SDK instances against the
 * same MySQL identity rows.
 *
 * The mutex map grows unboundedly with distinct user-ids — fine for the
 * bounded bench lifetime.
 */
class BenchSdkProvider(
    private val masterSecret: String,
    private val handlers: SharedHandlers,
) {
    private val config = benchConfig()
    private val mutexes = ConcurrentHashMap<String, Mutex>()

    suspend fun <T> withUser(userId: String, op: suspend (BreezSdk) -> T): T {
        val mutex = mutexes.computeIfAbsent(userId) { Mutex() }
        return mutex.withLock {
            val seed: Seed = Seed.Entropy(deriveSeedBytes(masterSecret, userId))
            val sdk = buildSdk(config, seed, handlers)
            try {
                op(sdk)
            } finally {
                try {
                    sdk.disconnect()
                } catch (e: Exception) {
                    System.err.println("[server] disconnect warn (user=$userId): ${e.message}")
                }
            }
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
data class ReceiveResult(val paymentRequest: String, val feeSats: String)

@Serializable
data class ErrorBody(val error: String)

/** Server-side handler timing for `requests.jsonl`. */
@Serializable
data class ServerRequestLogEntry(
    val ts: Long,
    val op: String,
    @SerialName("user_id") val userId: String,
    @SerialName("duration_ms") val durationMs: Long,
    val error: String? = null,
)

// --- reserved user-ids (funding pipeline) ---------------------------------

const val TREASURER_USER_ID = "__treasurer__"

fun senderUserId(idx: Int): String = "__sender_${idx}__"

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

/**
 * Builds an SDK for a chosen user-id with full Rust-side tracing
 * enabled, then times both `getInfo(ensureSynced=false)` (local read)
 * and an explicit `syncWallet()` (full sync). The resulting log file is
 * the SDK's view of what happens during a sync — useful for any
 * "why is this wallet slow?" investigation.
 *
 * Default user-id is the treasurer; override with `--user-id=<id>`
 * to inspect any other wallet (sender, user-pool entry, etc.).
 */
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
        // Fast path: if the local balance is already at-or-above target,
        // skip the full sync entirely. Closed-loop funding lands many
        // small sender→treasurer transfers per sweep that pile up
        // unclaimed on the treasurer; an explicit `syncWallet()` claims
        // them via O(N) FROST roundtrips to every operator and can stall
        // for minutes. The local read (from the persisted tree store) is
        // a strict lower bound on true balance (sync only adds incoming),
        // so local ≥ target ⇒ true ≥ target — safe to skip.
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

/**
 * Polls `syncWallet()` + `getInfo` every 5s until balance moves
 * above `currentBalance`. Prints a status line every 10s so a slow
 * faucet / regtest blip is visible instead of looking like a hang.
 * Throws if the deadline passes without progress.
 */
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

/**
 * Top each of K sender wallets up to `perSenderSats` from the
 * treasurer, concurrently up to `--parallelism`. Idempotent
 * (skip-if-balance-already-above). A per-sender failure is logged and
 * counted but does not abort the others; the run exits non-zero if any
 * sender is still unfunded, and simply re-running mops up the stragglers.
 */
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
        // Cached: lower bound on true balance (sync only adds incoming).
        // Sufficient for the "do we have enough?" warning below; avoids the
        // multi-minute sync caused by the treasurer's pending-transfer backlog.
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
                            // A single sender's failure must NOT cancel the
                            // scope: it shouldn't abort the other 99. Every
                            // sender gets its turn this run; we exit non-zero
                            // at the end if any failed. seed-senders is
                            // idempotent, so simply re-running mops up the
                            // stragglers.
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

    // Optional Rust-side tracing for diagnosing internal serialization /
    // bottlenecks. Off by default; set --log-filter=info|debug|trace to enable.
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
                handleAndLog(call, "info", userId, requestsWriter) {
                    provider.withUser(userId) { sdk ->
                        // Pure local read, no sync — the server-mode read path.
                        val info = sdk.getInfo(GetInfoRequest(ensureSynced = false))
                        InfoResponse(balanceSats = info.balanceSats.toLong())
                    }
                }
            }

            post("/users/{userId}/send") {
                val userId = call.parameters["userId"]!!
                val body = call.receive<SendBody>()
                handleAndLog(call, "send", userId, requestsWriter) {
                    provider.withUser(userId) { sdk ->
                        val prepared = sdk.prepareSendPayment(
                            PrepareSendPaymentRequest(
                                paymentRequest = body.destination,
                                amount = BigInteger.fromLong(body.amountSats),
                            )
                        )
                        val sendResp = sdk.sendPayment(SendPaymentRequest(prepareResponse = prepared))
                        SendResult(
                            paymentId = sendResp.payment.id,
                            feeSats = feeOf(prepared.paymentMethod),
                        )
                    }
                }
            }

            post("/users/{userId}/receive") {
                val userId = call.parameters["userId"]!!
                handleAndLog(call, "receive", userId, requestsWriter) {
                    provider.withUser(userId) { sdk ->
                        val resp = sdk.receivePayment(
                            ReceivePaymentRequest(paymentMethod = ReceivePaymentMethod.SparkAddress)
                        )
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

/**
 * Executes [block], times it, responds (with JSON on success or
 * 500 + ErrorBody on failure), and emits a [ServerRequestLogEntry] to
 * [requestsWriter] in the `finally` arm so failures land in
 * `requests.jsonl` too.
 *
 * `op` is the route op label ("info" / "send" / "receive"). The handler
 * never rethrows — Ktor logs handler exceptions on its own; we don't
 * need to bubble them up.
 */
private suspend inline fun <reified T : Any> handleAndLog(
    call: io.ktor.server.application.ApplicationCall,
    op: String,
    userId: String,
    requestsWriter: JsonlWriter<ServerRequestLogEntry>,
    crossinline block: suspend () -> T,
) {
    val tsMs = System.currentTimeMillis()
    val tStartNs = System.nanoTime()
    var errStr: String? = null
    try {
        call.respond(block())
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
                op = op,
                userId = userId,
                durationMs = (System.nanoTime() - tStartNs) / 1_000_000,
                error = errStr,
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
        "loadgen" -> runLoadGen(opts)
        "trace-sync" -> traceSync(opts)
        null, "help" -> {
            println(
                """
                breez-sdk-spark-benchmarks

                Usage: ./gradlew run --args="--mode=<mode> [options]"

                Modes:
                  smoke         Single-request flow check: derive seed for one user-id,
                                connect, getInfo, disconnect.
                  server        HTTP server with /users/{userId}/{info,send,receive}
                                endpoints. Fresh SDK per request. Emits requests.jsonl
                                + 1Hz metrics.jsonl to out/<run-id>/.
                  fund          Top up the reserved treasurer wallet via the Lightspark
                                regtest faucet. Idempotent. Requires FAUCET_USERNAME +
                                FAUCET_PASSWORD env vars (FAUCET_URL is optional).
                  seed-senders  One-shot top-up of the K reserved sender wallets from
                                the treasurer (Spark transfers). Idempotent — skips
                                senders already at or above --per-sender-sats.
                  loadgen       Open-loop HTTP load generator against the bench server.
                                Dispatches at --target-rps regardless of completion;
                                surfaces server backpressure as in-flight queue growth.

                Options (server / fund / seed-senders modes):
                  --mysql-url=mysql://user:pass@host:port/db   MySQL endpoint, including database name
                  --master-secret=<string>                     Master secret for HMAC seed derivation
                                                               (or set MASTER_SECRET env var)
                  --user-id=<id>                               (smoke) User id to derive seed for (default: smoke-default)
                  --port=<port>                                (server) HTTP listen port (default: 8080)
                  --run-id=<id>                                (server, loadgen) Defaults to filesystem-safe ISO-8601 timestamp
                  --out-dir=<path>                             (server, loadgen) Defaults to out/<run-id>/
                  --target-sats=<N>                            (fund) Treasurer balance target (default: 5_000_000;
                                                               sweep driver computes a workload-sized value)
                  --senders=<K>                                (seed-senders, loadgen) Number of sender wallets (default: 50)
                  --per-sender-sats=<N>                        (seed-senders) Top each sender up to N sats. Skips
                                                               senders already at or above N. Default: 5000;
                                                               sweep driver computes a workload-sized value
                  --parallelism=<N>                            (seed-senders) Concurrent top-ups (default: 5)

                Options (loadgen mode):
                  --base-url=<url>                             Bench server base URL (default: http://localhost:8080)
                  --target-rps=<R>                             Required. Open-loop dispatch rate (e.g. 100, 250.5)
                  --users=<N>                                  Workload pool size for /info+/receive user-ids (default: 10000)
                  --mix=info=A,receive=B,send=C                Op weights (any positive numbers; default: info=40,receive=30,send=30)
                  --user-distribution=uniform|zipf             Workload pool sampling (default: uniform)
                  --zipf-skew=<s>                              Zipf exponent (default: 1.0)
                  --duration=<10m|60s|1h|...>                  Required. Total run duration.
                  --payment-sats=<N>                           Sats per /send (default: 1)
                  --max-in-flight=<N>                          Hard cap; dispatch records 'dropped' if exceeded (default: 5000)
                  --treasurer-spark-addr=<addr>                Skip the bootstrap /receive call and use this address
                                                               as the /send destination. Sweep driver populates this
                                                               from a master-secret-scoped cache.
                """.trimIndent()
            )
        }
        else -> error("Unknown mode: ${opts["mode"]}. Use --mode=help.")
    }
}
