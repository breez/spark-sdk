import java.net.URI
import java.net.http.HttpClient
import java.net.http.HttpRequest
import java.net.http.HttpResponse
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardCopyOption
import java.time.Duration
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong

import kotlin.random.Random

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.future.await
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.yield
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

// --- record shape ---------------------------------------------------------

@Serializable
data class LogEntry(
    val ts: Long,                 // unix millis
    val op: String,               // "info" | "send" | "send_spark" | "send_ln" | "receive" | "receive_spark" | "receive_ln"
    @SerialName("user_id") val userId: String,
    @SerialName("status_code") val statusCode: Int? = null,
    @SerialName("duration_ms") val durationMs: Long? = null,
    val error: String? = null,
    val dropped: Boolean = false,  // true if dispatch was skipped due to in-flight cap
    // For send_ln only: index into the invoice pool, used by audit-bolt11.
    @SerialName("invoice_idx") val invoiceIdx: Int? = null,
)

// --- samplers -------------------------------------------------------------

private interface UserSampler {
    fun sample(rng: Random): Int
}

private class UniformSampler(private val n: Int) : UserSampler {
    override fun sample(rng: Random): Int = rng.nextInt(n)
}

/** Zipf sampler over [0, n). Precomputed cumulative weights, binary search per sample. */
private class ZipfSampler(private val n: Int, skew: Double) : UserSampler {
    private val cum: DoubleArray
    private val total: Double

    init {
        require(n > 0)
        require(skew > 0.0) { "zipf skew must be > 0" }
        cum = DoubleArray(n)
        var sum = 0.0
        for (i in 0 until n) {
            sum += 1.0 / Math.pow((i + 1).toDouble(), skew)
            cum[i] = sum
        }
        total = sum
    }

    override fun sample(rng: Random): Int {
        val u = rng.nextDouble() * total
        var lo = 0
        var hi = n - 1
        while (lo < hi) {
            val mid = (lo + hi) ushr 1
            if (cum[mid] < u) lo = mid + 1 else hi = mid
        }
        return lo
    }
}

private fun makeUserSampler(distribution: String, n: Int, zipfSkew: Double): UserSampler =
    when (distribution.lowercase()) {
        "uniform" -> UniformSampler(n)
        "zipf" -> ZipfSampler(n, zipfSkew)
        else -> error("Unknown --user-distribution: $distribution (expected uniform|zipf)")
    }

private class OpSampler(mix: List<Pair<String, Double>>) {
    private val ops: Array<String>
    private val cum: DoubleArray
    private val total: Double

    init {
        require(mix.isNotEmpty()) { "Op mix must not be empty" }
        ops = Array(mix.size) { mix[it].first }
        cum = DoubleArray(mix.size)
        var sum = 0.0
        for (i in mix.indices) {
            require(mix[i].second > 0.0) { "Op weight must be > 0: ${mix[i]}" }
            sum += mix[i].second
            cum[i] = sum
        }
        total = sum
    }

    fun sample(rng: Random): String {
        val u = rng.nextDouble() * total
        for (i in cum.indices) if (u <= cum[i]) return ops[i]
        return ops.last()
    }
}

// --- op-kind vocabulary ---------------------------------------------------

enum class OpKind { INFO, SEND_SPARK, SEND_LN, RECEIVE_SPARK, RECEIVE_LN }

fun opKindOf(label: String): OpKind = when (label.lowercase()) {
    "info" -> OpKind.INFO
    "send", "send_spark" -> OpKind.SEND_SPARK
    "send_ln", "send_lightning", "send_bolt11" -> OpKind.SEND_LN
    "receive", "receive_spark" -> OpKind.RECEIVE_SPARK
    "receive_ln", "receive_lightning", "receive_bolt11" -> OpKind.RECEIVE_LN
    else -> error(
        "unknown op label '$label'. Valid: info, send (alias send_spark), send_ln, " +
            "receive (alias receive_spark), receive_ln"
    )
}

// --- arg parsing helpers --------------------------------------------------

/** Parse e.g. "info=70,receive=20,send=10" → [("info",70),("receive",20),("send",10)]. */
private fun parseMix(spec: String): List<Pair<String, Double>> =
    spec.split(",").map { entry ->
        val (op, w) = entry.split("=", limit = 2).also {
            require(it.size == 2) { "Bad --mix entry: $entry (expected key=value)" }
        }
        op.trim() to (w.trim().toDoubleOrNull() ?: error("Bad weight in --mix: $entry"))
    }

/** Parse "10m" / "60s" / "1h" / "300" (bare = seconds) into a Duration. */
private fun parseDuration(spec: String): Duration {
    val s = spec.trim().lowercase()
    val (numStr, unit) = when {
        s.endsWith("ms") -> s.dropLast(2) to "ms"
        s.endsWith("s") -> s.dropLast(1) to "s"
        s.endsWith("m") -> s.dropLast(1) to "m"
        s.endsWith("h") -> s.dropLast(1) to "h"
        else -> s to "s"
    }
    val num = numStr.toLongOrNull() ?: error("Bad duration: $spec")
    return when (unit) {
        "ms" -> Duration.ofMillis(num)
        "s" -> Duration.ofSeconds(num)
        "m" -> Duration.ofMinutes(num)
        "h" -> Duration.ofHours(num)
        else -> error("Unreachable")
    }
}

// --- HTTP request shapes (must match the server's wire types) -------------

private val jsonOut = Json { encodeDefaults = false }

private fun infoReq(baseUrl: String, userId: String): HttpRequest =
    HttpRequest.newBuilder()
        .uri(URI.create("$baseUrl/users/$userId/info"))
        .timeout(Duration.ofSeconds(60))
        .GET()
        .build()

private fun receiveReq(baseUrl: String, userId: String): HttpRequest =
    HttpRequest.newBuilder()
        .uri(URI.create("$baseUrl/users/$userId/receive"))
        .timeout(Duration.ofSeconds(60))
        .header("Content-Type", "application/json")
        .POST(HttpRequest.BodyPublishers.ofString("{}"))
        .build()

private fun receiveLnReq(baseUrl: String, userId: String, amountSats: Long): HttpRequest {
    val body = """{"method":"bolt11","amountSats":$amountSats}"""
    return HttpRequest.newBuilder()
        .uri(URI.create("$baseUrl/users/$userId/receive"))
        .timeout(Duration.ofSeconds(60))
        .header("Content-Type", "application/json")
        .POST(HttpRequest.BodyPublishers.ofString(body))
        .build()
}

private fun sendReq(baseUrl: String, userId: String, destination: String, amountSats: Long): HttpRequest {
    val body = """{"destination":"$destination","amountSats":$amountSats}"""
    return HttpRequest.newBuilder()
        .uri(URI.create("$baseUrl/users/$userId/send"))
        .timeout(Duration.ofSeconds(60))
        .header("Content-Type", "application/json")
        .POST(HttpRequest.BodyPublishers.ofString(body))
        .build()
}

private suspend fun fetchTreasurerSparkAddress(httpClient: HttpClient, baseUrl: String): String {
    val resp = httpClient
        .sendAsync(receiveReq(baseUrl, TREASURER_USER_ID), HttpResponse.BodyHandlers.ofString())
        .await()
    check(resp.statusCode() == 200) { "treasurer /receive failed: HTTP ${resp.statusCode()}: ${resp.body()}" }
    val payload = Json.parseToJsonElement(resp.body()).let {
        it as? kotlinx.serialization.json.JsonObject ?: error("non-object response")
    }
    return payload["paymentRequest"]?.let { it as? kotlinx.serialization.json.JsonPrimitive }?.content
        ?: error("no paymentRequest in /receive response: ${resp.body()}")
}

// --- entry point ----------------------------------------------------------

fun runLoadGen(opts: Map<String, String>) = runBlocking {
    val baseUrl = (opts["base-url"] ?: "http://localhost:8080").trimEnd('/')
    val targetRps = opts["target-rps"]?.toDoubleOrNull()
        ?: error("--target-rps=<R> is required")
    require(targetRps > 0) { "--target-rps must be > 0" }
    val users = opts["users"]?.toIntOrNull() ?: 10_000
    require(users > 0) { "--users must be > 0" }
    val mixSpec = opts["mix"] ?: "info=40,receive=30,send=30"
    val parsedMix = parseMix(mixSpec)
    // Validate every label up front so a typo fails fast (before the
    // treasurer-bootstrap / pool-load latency) rather than surfacing as
    // an "unknown op" mid-run.
    val mixKinds: Set<OpKind> = parsedMix.map { opKindOf(it.first) }.toSet()
    val opSampler = OpSampler(parsedMix)
    val distribution = opts["user-distribution"] ?: "uniform"
    val zipfSkew = opts["zipf-skew"]?.toDoubleOrNull() ?: 1.0
    val userSampler = makeUserSampler(distribution, users, zipfSkew)
    val duration = parseDuration(opts["duration"] ?: error("--duration=<10m|60s|1h> is required"))
    val senderCount = opts["senders"]?.toIntOrNull() ?: 50
    require(senderCount > 0) { "--senders must be > 0" }
    val maxInFlight = opts["max-in-flight"]?.toIntOrNull() ?: 5_000
    val paymentSats = opts["payment-sats"]?.toLongOrNull() ?: 1L
    val runId = opts["run-id"] ?: defaultRunId()
    val outDir = Path.of(opts["out-dir"] ?: "out/$runId").also { Files.createDirectories(it) }

    println("[loadgen] base=$baseUrl  rps=$targetRps  users=$users  mix=$mixSpec  " +
        "dist=$distribution  duration=$duration  senders=$senderCount  " +
        "payment=${paymentSats}sat  max_in_flight=$maxInFlight  out=$outDir")

    val httpClient: HttpClient = HttpClient.newBuilder()
        .connectTimeout(Duration.ofSeconds(10))
        .build()

    // Only needed when a spark-send op is in the mix.
    val treasurerAddr: String? = if (OpKind.SEND_SPARK in mixKinds) {
        opts["treasurer-spark-addr"]?.takeIf { it.isNotBlank() } ?: run {
            println("[loadgen] fetching treasurer Spark address from $baseUrl …")
            fetchTreasurerSparkAddress(httpClient, baseUrl)
        }.also { println("[loadgen] treasurer destination: $it") }
    } else {
        println("[loadgen] mix has no spark-send op — skipping treasurer addr fetch")
        null
    }

    // Required iff mix has send_ln. Cursor persists across sweep steps so single-use invoices aren't reused.
    val invoicePoolPath: String? = opts["invoice-pool"]?.takeIf { it.isNotBlank() }
    val invoiceCursorPath: Path? = invoicePoolPath?.let { Path.of("$it.cursor") }
    val invoiceStartCursor: Int = invoiceCursorPath?.let {
        if (Files.exists(it)) {
            val v = Files.readString(it).trim().toIntOrNull() ?: 0
            println("[loadgen] resuming invoice pool at cursor=$v (from ${it.fileName})")
            v
        } else 0
    } ?: 0
    val invoicePool: List<String>? = if (OpKind.SEND_LN in mixKinds) {
        val poolPath = invoicePoolPath
            ?: error("--invoice-pool=<file> is required when the mix has send_ln")
        val pool = Files.readAllLines(Path.of(poolPath))
            .map { it.trim() }
            .filter { it.isNotEmpty() }
        require(pool.isNotEmpty()) { "invoice pool $poolPath is empty" }
        require(invoiceStartCursor <= pool.size) {
            "invoice cursor ($invoiceStartCursor) exceeds pool size (${pool.size})"
        }
        println("[loadgen] loaded ${pool.size} invoices from $poolPath (cursor starts at $invoiceStartCursor, " +
            "${pool.size - invoiceStartCursor} unspent)")
        pool
    } else {
        if (opts["invoice-pool"]?.isNotBlank() == true) {
            println("[loadgen] --invoice-pool set but mix has no send_ln — ignoring")
        }
        null
    }

    val writer = JsonlWriter(outDir.resolve("latency.jsonl"), LogEntry.serializer())
    val rng = Random.Default

    val intervalNs = (1_000_000_000.0 / targetRps).toLong()
    val startNs = System.nanoTime()
    val endNs = startNs + duration.toNanos()
    val inFlight = AtomicInteger(0)
    val completedCount = AtomicLong(0)
    val errorCount = AtomicLong(0)
    var dispatched = 0L
    var dropped = 0L
    var senderCursor = 0
    var invoiceCursor = invoiceStartCursor
    // Hoisted: read after coroutineScope to drive a non-zero exit.
    var poolExhausted = false

    coroutineScope {
        // 5s progress tick.
        val progressJob = launch {
            var lastDispatched = 0L
            var lastCompleted = 0L
            var lastTickNs = startNs
            while (true) {
                delay(5_000)
                val now = System.nanoTime()
                val elapsedSec = (now - startNs) / 1_000_000_000
                val tickSec = (now - lastTickNs).coerceAtLeast(1).let { it / 1_000_000_000.0 }
                val dispRate = (dispatched - lastDispatched) / tickSec
                val complRate = (completedCount.get() - lastCompleted) / tickSec
                println(
                    "[loadgen] +${elapsedSec}s  dispatched=$dispatched (Δ${"%.1f".format(dispRate)}/s)  " +
                        "in_flight=${inFlight.get()}  completed=${completedCount.get()} (Δ${"%.1f".format(complRate)}/s)  " +
                        "errors=${errorCount.get()}  dropped=$dropped"
                )
                lastDispatched = dispatched
                lastCompleted = completedCount.get()
                lastTickNs = now
            }
        }

        var nextDispatchNs = startNs
        while (System.nanoTime() < endNs && !poolExhausted) {
            val now = System.nanoTime()
            if (now < nextDispatchNs) {
                val sleepMs = (nextDispatchNs - now) / 1_000_000
                if (sleepMs > 0) delay(sleepMs) else yield()
                continue
            }
            val op = opSampler.sample(rng)
            val userId: String
            val request: HttpRequest
            // Only set on send_ln; threaded into LogEntry so the post-run
            // audit can map this dispatch to a specific invoice.
            var dispatchInvoiceIdx: Int? = null
            when (opKindOf(op)) {
                OpKind.INFO -> {
                    userId = "u${userSampler.sample(rng)}"
                    request = infoReq(baseUrl, userId)
                }
                OpKind.RECEIVE_SPARK -> {
                    userId = "u${userSampler.sample(rng)}"
                    request = receiveReq(baseUrl, userId)
                }
                OpKind.RECEIVE_LN -> {
                    userId = "u${userSampler.sample(rng)}"
                    request = receiveLnReq(baseUrl, userId, paymentSats)
                }
                OpKind.SEND_SPARK -> {
                    val dest = treasurerAddr ?: error("send_spark in mix but no treasurer addr — bug?")
                    userId = senderUserId(senderCursor)
                    senderCursor = (senderCursor + 1) % senderCount
                    request = sendReq(baseUrl, userId, dest, paymentSats)
                }
                OpKind.SEND_LN -> {
                    val pool = invoicePool ?: error("send_ln in mix but no invoice pool — bug?")
                    if (invoiceCursor >= pool.size) {
                        // Hard-stop: pool was undersized — results from here are noise.
                        System.err.println(
                            "[loadgen] invoice pool exhausted at ${pool.size} send_ln dispatches — stopping"
                        )
                        writer.submit(LogEntry(
                            ts = System.currentTimeMillis(),
                            op = op,
                            userId = senderUserId(senderCursor),
                            error = "invoice_pool_exhausted",
                            dropped = true,
                        ))
                        dropped++
                        poolExhausted = true
                        continue
                    }
                    dispatchInvoiceIdx = invoiceCursor
                    val invoice = pool[invoiceCursor]
                    invoiceCursor++
                    // Persist per-dispatch with atomic rename; a lost cursor re-dispatches paid invoices.
                    if (invoiceCursorPath != null) {
                        val tmp = invoiceCursorPath.resolveSibling("${invoiceCursorPath.fileName}.tmp")
                        Files.writeString(tmp, invoiceCursor.toString())
                        Files.move(
                            tmp,
                            invoiceCursorPath,
                            StandardCopyOption.ATOMIC_MOVE,
                            StandardCopyOption.REPLACE_EXISTING,
                        )
                    }
                    userId = senderUserId(senderCursor)
                    senderCursor = (senderCursor + 1) % senderCount
                    request = sendReq(baseUrl, userId, invoice, paymentSats)
                }
            }

            if (inFlight.get() >= maxInFlight) {
                writer.submit(LogEntry(
                    ts = System.currentTimeMillis(),
                    op = op,
                    userId = userId,
                    dropped = true,
                    invoiceIdx = dispatchInvoiceIdx,
                ))
                dropped++
            } else {
                inFlight.incrementAndGet()
                val invoiceIdxAtDispatch = dispatchInvoiceIdx
                launch(Dispatchers.IO) {
                    val tStart = System.nanoTime()
                    try {
                        val resp = httpClient
                            .sendAsync(request, HttpResponse.BodyHandlers.discarding())
                            .await()
                        val durMs = (System.nanoTime() - tStart) / 1_000_000
                        val ok = resp.statusCode() in 200..299
                        if (!ok) errorCount.incrementAndGet()
                        writer.submit(LogEntry(
                            ts = System.currentTimeMillis(),
                            op = op,
                            userId = userId,
                            statusCode = resp.statusCode(),
                            durationMs = durMs,
                            error = if (ok) null else "http_${resp.statusCode()}",
                            invoiceIdx = invoiceIdxAtDispatch,
                        ))
                    } catch (e: Throwable) {
                        val durMs = (System.nanoTime() - tStart) / 1_000_000
                        errorCount.incrementAndGet()
                        writer.submit(LogEntry(
                            ts = System.currentTimeMillis(),
                            op = op,
                            userId = userId,
                            durationMs = durMs,
                            error = "${e::class.simpleName}: ${e.message}",
                            invoiceIdx = invoiceIdxAtDispatch,
                        ))
                    } finally {
                        inFlight.decrementAndGet()
                        completedCount.incrementAndGet()
                    }
                }
            }
            dispatched++
            nextDispatchNs += intervalNs
        }

        // Drain: wait up to 60s for in-flight requests to complete.
        println("[loadgen] dispatch done; draining ${inFlight.get()} in-flight requests (max 60s)")
        val drainEndNs = System.nanoTime() + 60_000_000_000L
        while (inFlight.get() > 0 && System.nanoTime() < drainEndNs) {
            delay(100)
        }
        if (inFlight.get() > 0) {
            System.err.println("[loadgen] drain warning: ${inFlight.get()} requests still in flight after 60s")
        }
        progressJob.cancel()
    }

    writer.close()  // drains the queue + closes the file (blocks up to 10s)
    if (invoiceCursorPath != null && invoicePool != null) {
        println("[loadgen] invoice cursor at $invoiceCursor / ${invoicePool.size} " +
            "(persisted per-dispatch to ${invoiceCursorPath.fileName})")
    }
    println("[loadgen] dispatched=$dispatched  dropped=$dropped  " +
        "actual_rps=${"%.2f".format(dispatched.toDouble() * 1e9 / (System.nanoTime() - startNs))}")
    if (poolExhausted) {
        System.err.println(
            "[loadgen] FAILED: invoice pool exhausted before duration elapsed " +
                "(consumed $invoiceCursor/${invoicePool?.size}). The pool was undersized " +
                "vs the offered load; results from this step are not a valid measurement. " +
                "Re-mint a larger pool (sweep driver computes the count)."
        )
        kotlin.system.exitProcess(1)
    }
    println("[loadgen] OK  out=$outDir")
}
