import java.net.URI
import java.net.http.HttpClient
import java.net.http.HttpRequest
import java.net.http.HttpResponse
import java.nio.file.Files
import java.nio.file.Path
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
    val op: String,               // "info" | "send" | "receive"
    @SerialName("user_id") val userId: String,
    @SerialName("status_code") val statusCode: Int? = null,
    @SerialName("duration_ms") val durationMs: Long? = null,
    val error: String? = null,
    val dropped: Boolean = false,  // true if dispatch was skipped due to in-flight cap
)

// --- samplers -------------------------------------------------------------

private interface UserSampler {
    fun sample(rng: Random): Int
}

private class UniformSampler(private val n: Int) : UserSampler {
    override fun sample(rng: Random): Int = rng.nextInt(n)
}

/**
 * Zipf sampler over [0, n). Precomputes cumulative weights at construction
 * (8N bytes) and binary-searches per sample. For N ≤ 10^6 this is ≤ 8 MB
 * of memory; partner workloads at this scale don't need denser sampling.
 */
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

private fun sendReq(baseUrl: String, userId: String, destination: String, amountSats: Long): HttpRequest {
    // SendBody is the on-wire shape from Main.kt; encode minimal JSON by hand
    // to avoid coupling LoadGen to the server's @Serializable type.
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
    val opSampler = OpSampler(parseMix(mixSpec))
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

    // Treasurer Spark address: prefer the caller-provided value (sweep
    // driver caches it once per master secret to avoid paying treasurer
    // SDK cold-start sync on every per-step server restart). Fall back
    // to fetching via /receive only if not provided — useful for ad-hoc
    // single-step runs but ill-suited to a sweep where the cold-start
    // can take many minutes.
    val treasurerAddr = opts["treasurer-spark-addr"]?.takeIf { it.isNotBlank() } ?: run {
        println("[loadgen] fetching treasurer Spark address from $baseUrl …")
        fetchTreasurerSparkAddress(httpClient, baseUrl)
    }
    println("[loadgen] treasurer destination: $treasurerAddr")

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

    coroutineScope {
        // Periodic progress logger — prints every 5s while dispatching, then a
        // shorter interval during drain so the user sees in-flight tail off.
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
        while (System.nanoTime() < endNs) {
            val now = System.nanoTime()
            if (now < nextDispatchNs) {
                val sleepMs = (nextDispatchNs - now) / 1_000_000
                if (sleepMs > 0) delay(sleepMs) else yield()
                continue
            }
            val op = opSampler.sample(rng)
            val userId: String
            val request: HttpRequest
            when (op) {
                "info" -> {
                    userId = "u${userSampler.sample(rng)}"
                    request = infoReq(baseUrl, userId)
                }
                "receive" -> {
                    userId = "u${userSampler.sample(rng)}"
                    request = receiveReq(baseUrl, userId)
                }
                "send" -> {
                    userId = senderUserId(senderCursor)
                    senderCursor = (senderCursor + 1) % senderCount
                    request = sendReq(baseUrl, userId, treasurerAddr, paymentSats)
                }
                else -> error("Unknown op from sampler: $op")
            }

            if (inFlight.get() >= maxInFlight) {
                writer.submit(LogEntry(
                    ts = System.currentTimeMillis(),
                    op = op,
                    userId = userId,
                    dropped = true,
                ))
                dropped++
            } else {
                inFlight.incrementAndGet()
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
    println("[loadgen] dispatched=$dispatched  dropped=$dropped  " +
        "actual_rps=${"%.2f".format(dispatched.toDouble() * 1e9 / (System.nanoTime() - startNs))}")
    println("[loadgen] OK  out=$outDir")
}
