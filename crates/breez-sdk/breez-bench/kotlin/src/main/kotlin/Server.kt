import breez_sdk_spark.GetInfoRequest
import breez_sdk_spark.PrepareSendPaymentRequest
import breez_sdk_spark.ReceivePaymentMethod
import breez_sdk_spark.ReceivePaymentRequest
import breez_sdk_spark.SendPaymentMethod
import breez_sdk_spark.SendPaymentOptions
import breez_sdk_spark.SendPaymentRequest
import breez_sdk_spark.initLogging

import com.ionspin.kotlin.bignum.integer.BigInteger

import io.ktor.http.HttpStatusCode
import io.ktor.serialization.kotlinx.json.json
import io.ktor.server.application.call
import io.ktor.server.application.install
import io.ktor.server.engine.embeddedServer
import io.ktor.server.netty.Netty
import io.ktor.server.plugins.contentnegotiation.ContentNegotiation
import io.ktor.server.request.receive
import io.ktor.server.request.receiveText
import io.ktor.server.response.respond
import io.ktor.server.routing.get
import io.ktor.server.routing.post
import io.ktor.server.routing.routing

import java.nio.file.Files
import java.nio.file.Path
import java.util.UUID

import kotlinx.coroutines.runBlocking
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

// --- HTTP request/response shapes -----------------------------------------

// Preset filter for `--bench-trace`. Targets:
//   - `spark::tree::service=trace`           — payment-time leaf swaps
//     (the "Swapped leaves to match target amount" log line)
//   - `breez_sdk_core::send_payment=info`    — top-level `send_payment`
//     span; carries `payment_id` field.
//   - `spark::operator_rpc=info`             — every operator-RPC span
//     (carries `operator_id`).
//   - `spark::ssp=info`                      — every SSP-method span.
// The three info-level targets are SDK span-trace targets — they only
// produce close-event lines when the SDK is built with the `span-trace`
// cargo feature (off by default, on for bench builds). `aggregate.py`
// reads the close events to render the per-RPC slow-payment breakdown.
// Trailing `error` keeps every other module at error-level so the file
// surfaces real failures without unbounded growth at high RPS.
private const val BENCH_TRACE_FILTER =
    "spark::tree::service=trace," +
    "breez_sdk_core::send_payment=info," +
    "spark::operator_rpc=info," +
    "spark::ssp=info," +
    "error"

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

// --- server entry point ---------------------------------------------------

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

    // Tracing wiring. Three states:
    //   1. --bench-trace defaults to true → preset filter that turns on
    //      leaves-swap detection + per-RPC close-event spans (consumed
    //      by aggregate.py). Close-event capture requires the SDK to be
    //      built with the `span-trace` cargo feature — prod builds (no
    //      feature) contain none of that code, so these targets are
    //      structurally inaccessible to integrators.
    //   2. --bench-trace=false           → no SDK tracing (zero overhead;
    //      use to A/B against (1) when measuring instrumentation cost).
    //   3. --log-filter=<spec>           → explicit override, user takes
    //      full control (must include the span-trace targets themselves
    //      if they want aggregate.py to pick up swap/phase data).
    val explicitFilter = opts["log-filter"]
    val benchTrace = opts["bench-trace"]?.equals("true", ignoreCase = true) ?: true
    val effectiveFilter = when {
        explicitFilter != null -> explicitFilter
        benchTrace -> BENCH_TRACE_FILTER
        else -> null
    }
    if (effectiveFilter != null) {
        val logDir = opts["log-dir"] ?: outDir.resolve(".trace-logs").toString()
        Files.createDirectories(Path.of(logDir))
        println("[server] init_logging dir=$logDir filter=$effectiveFilter")
        initLogging(logDir, null, effectiveFilter)
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
                        // Pre-supply the idempotency key so the SDK records
                        // it as `payment_id` on the bench send_payment span
                        // before any child RPC span closes. Lets aggregate.py
                        // correlate per-RPC close events back to this send.
                        val idempotencyKey = UUID.randomUUID().toString()
                        t.paymentId = idempotencyKey
                        val tSendNs = System.nanoTime()
                        val sendResp = sdk.sendPayment(
                            SendPaymentRequest(
                                prepareResponse = prepared,
                                options = sendOptions,
                                idempotencyKey = idempotencyKey,
                            )
                        )
                        t.sendMs = (System.nanoTime() - tSendNs) / 1_000_000
                        // sendResp.payment.id == idempotencyKey for the paths
                        // exercised here (LN + spark transfer); reuse the
                        // SDK-returned value rather than the local for the
                        // wire response in case the SDK ever normalizes.
                        SendResult(
                            paymentId = sendResp.payment.id,
                            feeSats = feeOf(prepared.paymentMethod),
                        )
                    }
                }
            }

            post("/users/{userId}/receive") {
                val userId = call.parameters["userId"]!!
                // Empty body → spark-address default. Non-empty but malformed
                // → 400. (Silently defaulting on parse failure would let a
                // loadgen wire-format typo show up as a successful spark
                // receive, conflating it with the LN path in the measurement.)
                val raw = call.receiveText()
                val body: ReceiveBody = if (raw.isBlank()) {
                    ReceiveBody()
                } else {
                    try {
                        Json.decodeFromString(ReceiveBody.serializer(), raw)
                    } catch (e: Exception) {
                        call.respond(
                            HttpStatusCode.BadRequest,
                            ErrorBody(error = "malformed receive body: ${e.message}"),
                        )
                        return@post
                    }
                }
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
                paymentId = timings.paymentId,
            )
        )
    }
}
