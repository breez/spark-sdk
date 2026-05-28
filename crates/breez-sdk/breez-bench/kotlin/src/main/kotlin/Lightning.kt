import breez_sdk_spark.BreezSdk
import breez_sdk_spark.ListPaymentsRequest
import breez_sdk_spark.PaymentDetails
import breez_sdk_spark.PaymentDetailsFilter
import breez_sdk_spark.PaymentStatus
import breez_sdk_spark.PaymentType
import breez_sdk_spark.PrepareSendPaymentRequest
import breez_sdk_spark.ReceivePaymentMethod
import breez_sdk_spark.ReceivePaymentRequest
import breez_sdk_spark.Seed
import breez_sdk_spark.SendPaymentMethod
import breez_sdk_spark.SyncWalletRequest

import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardOpenOption
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicInteger

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.sync.withPermit
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

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

    // Skip errored dispatches too: those never reached sendPayment (HTTP 500
    // from a handler crash, ConnectException from a dead server, etc.) so the
    // invoice was never spent. Counting them as "expected" would inflate
    // `not_found` and bake harness-side failures into the LN settled_rate.
    var skippedErrored = 0
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
                if (obj["error"]?.jsonPrimitive?.contentOrNull != null) {
                    skippedErrored++
                    return@inner
                }
                val idx = obj["invoice_idx"]?.jsonPrimitive?.intOrNull ?: return@inner
                val uid = obj["user_id"]?.jsonPrimitive?.contentOrNull ?: return@inner
                if (idx !in pool.indices) return@inner
                expected.add(Expected(rps, uid, idx, pool[idx]))
            }
        }
    }
    println("[audit] expected dispatches (non-dropped, non-errored send_ln): ${expected.size} " +
        "(skipped $skippedErrored errored)")
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
