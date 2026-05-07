import breez_sdk_spark.BreezSdk
import breez_sdk_spark.GetInfoRequest
import breez_sdk_spark.Network
import breez_sdk_spark.PrepareSendPaymentRequest
import breez_sdk_spark.ReceivePaymentMethod
import breez_sdk_spark.ReceivePaymentRequest
import breez_sdk_spark.SdkBuilder
import breez_sdk_spark.Seed
import breez_sdk_spark.SendPaymentMethod
import breez_sdk_spark.SendPaymentRequest
import breez_sdk_spark.defaultConfig
import breez_sdk_spark.defaultMysqlStorageConfig

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

import java.util.concurrent.ConcurrentHashMap

import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
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

// --- per-request SDK lifecycle --------------------------------------------

/**
 * Spins up a fresh SDK instance per call and tears it down after `op`. Same-
 * `userId` calls serialize through a per-userId mutex so concurrent requests
 * never race two SDK instances against the same MySQL identity rows.
 *
 * The mutex map grows unboundedly with distinct user-ids. For v1 (bench
 * lifetime is bounded) that's acceptable. Phase 7 (LRU SDK pool) revisits
 * this together with instance reuse.
 */
class BenchSdkProvider(
    private val masterSecret: String,
    mysqlUrl: String,
) {
    private val config = defaultConfig(Network.REGTEST)
    private val storageCfg = defaultMysqlStorageConfig(mysqlUrl)
    private val mutexes = ConcurrentHashMap<String, Mutex>()

    suspend fun <T> withUser(userId: String, op: suspend (BreezSdk) -> T): T {
        val mutex = mutexes.computeIfAbsent(userId) { Mutex() }
        return mutex.withLock {
            val seed: Seed = Seed.Entropy(deriveSeedBytes(masterSecret, userId))
            val builder = SdkBuilder(config, seed)
            builder.withMysqlBackend(storageCfg)
            val sdk = builder.build()
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

    // Fail fast if creds are missing — otherwise we'd build the SDK,
    // request a deposit address, and only then discover we can't fund.
    System.getenv("FAUCET_USERNAME") ?: error("FAUCET_USERNAME env var is required")
    System.getenv("FAUCET_PASSWORD") ?: error("FAUCET_PASSWORD env var is required")

    println("[fund] treasurer top-up to $targetSats sats  mysql=${maskPassword(mysqlUrl)}")

    val seed: Seed = Seed.Entropy(deriveSeedBytes(masterSecret, TREASURER_USER_ID))
    val builder = SdkBuilder(defaultConfig(Network.REGTEST), seed)
    builder.withMysqlBackend(defaultMysqlStorageConfig(mysqlUrl))
    val sdk = builder.build()

    try {
        // Reuse an existing deposit address if the treasurer has one.
        val depositAddr = sdk.receivePayment(
            ReceivePaymentRequest(paymentMethod = ReceivePaymentMethod.BitcoinAddress(newAddress = false))
        ).paymentRequest
        println("[fund] deposit address: $depositAddr")

        var chunkIdx = 0
        while (true) {
            val info = sdk.getInfo(GetInfoRequest(ensureSynced = true))
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
            waitForBalanceIncrease(sdk, balance.toULong(), timeoutMs = 240_000)
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
 * Polls `getInfo({ensureSynced=true})` every 5s until balance moves
 * above `currentBalance`. Throws if the deadline passes without
 * progress — the caller's loop decides whether to retry the faucet.
 */
private suspend fun waitForBalanceIncrease(sdk: BreezSdk, currentBalance: ULong, timeoutMs: Long) {
    val deadline = System.currentTimeMillis() + timeoutMs
    while (System.currentTimeMillis() < deadline) {
        delay(5_000)
        val info = sdk.getInfo(GetInfoRequest(ensureSynced = true))
        if (info.balanceSats > currentBalance) return
    }
    error("Balance did not increase within ${timeoutMs}ms (was $currentBalance sats)")
}

// --- seed-senders mode (top up sender pool from treasurer) ----------------

/**
 * One-shot seeding pass for the K reserved sender wallets
 * (`__sender_0__` … `__sender_{K-1}__`). For each sender, if its
 * balance is below `minSats`, the treasurer Spark-transfers enough to
 * bring it up to `targetSats`.
 *
 * In the closed-loop bench design (load generator's `/send` always
 * targets the treasurer), senders barely drain — only by per-transfer
 * fees. So a single seeding pass before a long run is enough; no
 * background replenisher is needed. Re-running this is idempotent.
 *
 * Senders are processed in parallel with bounded concurrency so the
 * treasurer SDK isn't hit by K simultaneous sendPayment calls.
 */
fun seedSenders(opts: Map<String, String>) = runBlocking {
    val mysqlUrl = opts["mysql-url"]
        ?: error("--mysql-url=mysql://user:pass@host:port/dbname is required")
    val masterSecret = opts["master-secret"]
        ?: System.getenv("MASTER_SECRET")
        ?: error("--master-secret=<hex-or-string> or MASTER_SECRET env var is required")
    val senderCount = opts["senders"]?.toIntOrNull() ?: 50
    val minSats = opts["min-sats"]?.toLongOrNull() ?: 10_000L
    val targetSats = opts["target-sats"]?.toLongOrNull() ?: 50_000L
    val parallelism = opts["parallelism"]?.toIntOrNull() ?: 5

    require(senderCount > 0) { "--senders must be > 0" }
    require(minSats in 1..targetSats) { "--min-sats must be in [1, --target-sats]" }
    require(parallelism > 0) { "--parallelism must be > 0" }

    println(
        "[seed] senders=$senderCount  min=$minSats  target=$targetSats  parallel=$parallelism  " +
            "mysql=${maskPassword(mysqlUrl)}"
    )

    val config = defaultConfig(Network.REGTEST)
    val storageCfg = defaultMysqlStorageConfig(mysqlUrl)

    val treasurerSeed: Seed = Seed.Entropy(deriveSeedBytes(masterSecret, TREASURER_USER_ID))
    val treasurer = SdkBuilder(config, treasurerSeed)
        .also { it.withMysqlBackend(storageCfg) }
        .build()

    try {
        val treasurerInfo = treasurer.getInfo(GetInfoRequest(ensureSynced = true))
        val treasurerBalance = treasurerInfo.balanceSats.toLong()
        println("[seed] treasurer balance: $treasurerBalance sats")
        // Worst-case spend if every sender is at zero. We only warn —
        // partial seeding is still useful for diagnostics.
        val maxSpend = senderCount.toLong() * targetSats
        if (treasurerBalance < maxSpend) {
            System.err.println(
                "[seed] warning: treasurer has $treasurerBalance sats; up to $maxSpend may be needed " +
                    "if all senders are empty. Run 'make fund' first."
            )
        }

        val sem = Semaphore(parallelism)
        var fundedCount = 0
        var skippedCount = 0
        coroutineScope {
            for (i in 0 until senderCount) {
                launch {
                    sem.withPermit {
                        val outcome = seedOneSender(
                            treasurer = treasurer,
                            senderIdx = i,
                            masterSecret = masterSecret,
                            config = config,
                            storageCfg = storageCfg,
                            minSats = minSats,
                            targetSats = targetSats,
                        )
                        synchronized(this@runBlocking) {
                            when (outcome) {
                                SeedOutcome.FUNDED -> fundedCount++
                                SeedOutcome.SKIPPED -> skippedCount++
                            }
                        }
                    }
                }
            }
        }
        println("[seed] OK  funded=$fundedCount  skipped=$skippedCount")
    } finally {
        try {
            treasurer.disconnect()
        } catch (e: Exception) {
            System.err.println("[seed] treasurer disconnect warn: ${e.message}")
        }
    }
}

private enum class SeedOutcome { FUNDED, SKIPPED }

private suspend fun seedOneSender(
    treasurer: BreezSdk,
    senderIdx: Int,
    masterSecret: String,
    config: breez_sdk_spark.Config,
    storageCfg: breez_sdk_spark.MysqlStorageConfig,
    minSats: Long,
    targetSats: Long,
): SeedOutcome {
    val userId = senderUserId(senderIdx)
    val seed: Seed = Seed.Entropy(deriveSeedBytes(masterSecret, userId))
    val sender = SdkBuilder(config, seed).also { it.withMysqlBackend(storageCfg) }.build()

    return try {
        val info = sender.getInfo(GetInfoRequest(ensureSynced = true))
        val balance = info.balanceSats.toLong()
        if (balance >= minSats) {
            println("[seed] sender $senderIdx: $balance sats (≥ $minSats, skip)")
            return SeedOutcome.SKIPPED
        }
        val sparkAddr = sender.receivePayment(
            ReceivePaymentRequest(paymentMethod = ReceivePaymentMethod.SparkAddress)
        ).paymentRequest
        val toSend = targetSats - balance
        println("[seed] sender $senderIdx: $balance sats → topping up by $toSend to $targetSats")

        val prepared = treasurer.prepareSendPayment(
            PrepareSendPaymentRequest(
                paymentRequest = sparkAddr,
                amount = BigInteger.fromLong(toSend),
            )
        )
        treasurer.sendPayment(SendPaymentRequest(prepareResponse = prepared))

        // Verify the receiver sees the transfer. Spark transfers are
        // typically sub-second, but the receiver's SDK still has to
        // sync to surface the new balance.
        waitForBalanceIncrease(sender, balance.toULong(), timeoutMs = 60_000)
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
    val masterSecret = opts["master-secret"]
        ?: System.getenv("MASTER_SECRET")
        ?: error("--master-secret=<hex-or-string> or MASTER_SECRET env var is required")
    val port = opts["port"]?.toIntOrNull() ?: 8080

    val provider = BenchSdkProvider(masterSecret, mysqlUrl)

    println("[server] listening on :$port  mysql=${maskPassword(mysqlUrl)}")

    embeddedServer(Netty, port = port) {
        install(ContentNegotiation) { json() }
        routing {
            // GET /users/{userId}/info → InfoResponse
            //
            // ensureSynced=true: balance against fresh per-request SDK state
            // is meaningless without forcing a sync. v1 (per-request lifecycle)
            // pays the full sync cost on every call; Phase 7 (pooled SDK) will
            // amortize it to once-per-pool-admission.
            get("/users/{userId}/info") {
                val userId = call.parameters["userId"]!!
                handle(call) {
                    provider.withUser(userId) { sdk ->
                        val info = sdk.getInfo(GetInfoRequest(ensureSynced = true))
                        InfoResponse(balanceSats = info.balanceSats.toLong())
                    }
                }
            }

            // POST /users/{userId}/send  body=SendBody → SendResult
            //
            // Spans both prepareSendPayment + sendPayment. The plan reports
            // /send latency as a single number; /send-by-stage breakdown can
            // be added later if a partner wants it.
            post("/users/{userId}/send") {
                val userId = call.parameters["userId"]!!
                val body = call.receive<SendBody>()
                handle(call) {
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

            // POST /users/{userId}/receive → ReceiveResult
            //
            // Address generation only — no funds are landing during the
            // measurement window. This number is the cost of producing a
            // deposit destination, not the end-to-end cost of a payment
            // arriving. Worth flagging in RESULTS.md so partners don't
            // misread the number.
            post("/users/{userId}/receive") {
                val userId = call.parameters["userId"]!!
                handle(call) {
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

private suspend inline fun <reified T : Any> handle(
    call: io.ktor.server.application.ApplicationCall,
    crossinline op: suspend () -> T,
) {
    try {
        call.respond(op())
    } catch (e: Throwable) {
        System.err.println("[server] handler error: ${e.message}")
        call.respond(HttpStatusCode.InternalServerError, ErrorBody(error = e.message ?: e::class.qualifiedName ?: "error"))
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
        null, "help" -> {
            println(
                """
                breez-sdk-spark-benchmarks

                Usage: ./gradlew run --args="--mode=<mode> [options]"

                Modes:
                  smoke         Single-request flow check: derive seed for one user-id,
                                connect, getInfo, disconnect.
                  server        HTTP server with /users/{userId}/{info,send,receive}
                                endpoints. Each request spins up a fresh SDK instance
                                (per-request lifecycle, v1 baseline).
                  fund          Top up the reserved treasurer wallet via the Lightspark
                                regtest faucet. Idempotent. Requires FAUCET_USERNAME +
                                FAUCET_PASSWORD env vars (FAUCET_URL is optional).
                  seed-senders  One-shot top-up of the K reserved sender wallets from
                                the treasurer (Spark transfers). Idempotent — only
                                refills senders whose balance is below --min-sats.
                  loadgen       Open-loop HTTP load generator against the bench server.
                                Dispatches at --target-rps regardless of completion;
                                surfaces server backpressure as in-flight queue growth.

                Options (server / fund / seed-senders modes):
                  --mysql-url=mysql://user:pass@host:port/db   MySQL endpoint, including database name
                  --master-secret=<string>                     Master secret for HMAC seed derivation
                                                               (or set MASTER_SECRET env var)
                  --user-id=<id>                               (smoke) User id to derive seed for (default: smoke-default)
                  --port=<port>                                (server) HTTP listen port (default: 8080)
                  --target-sats=<N>                            (fund) Treasurer balance target (default: 5_000_000)
                                                               (seed-senders) Per-sender top-up target (default: 50_000)
                  --senders=<K>                                (seed-senders, loadgen) Number of sender wallets (default: 50)
                  --min-sats=<N>                               (seed-senders) Refill threshold per sender (default: 10_000)
                  --parallelism=<N>                            (seed-senders) Concurrent top-ups (default: 5)

                Options (loadgen mode):
                  --base-url=<url>                             Bench server base URL (default: http://localhost:8080)
                  --target-rps=<R>                             Required. Open-loop dispatch rate (e.g. 100, 250.5)
                  --users=<N>                                  Workload pool size for /info+/receive user-ids (default: 10000)
                  --mix=info=A,receive=B,send=C                Op weights (any positive numbers; default: info=40,receive=30,send=30)
                  --user-distribution=uniform|zipf             Workload pool sampling (default: uniform)
                  --zipf-skew=<s>                              Zipf exponent (default: 1.0)
                  --duration=<10m|60s|1h|...>                  Required. Total run duration.
                  --warmup-secs=<N>                            Mark first N seconds of samples as warmup (default: 60)
                  --payment-sats=<N>                           Sats per /send (default: 1)
                  --max-in-flight=<N>                          Hard cap; dispatch records 'dropped' if exceeded (default: 5000)
                  --run-id=<id>                                Defaults to filesystem-safe ISO-8601 timestamp
                  --out-dir=<path>                             Defaults to out/<run-id>/
                """.trimIndent()
            )
        }
        else -> error("Unknown mode: ${opts["mode"]}. Use --mode=help.")
    }
}
