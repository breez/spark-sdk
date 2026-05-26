import breez_sdk_spark.BreezSdk
import breez_sdk_spark.GetInfoRequest
import breez_sdk_spark.Network
import breez_sdk_spark.SdkBuilder
import breez_sdk_spark.SdkContext
import breez_sdk_spark.SdkContextConfig
import breez_sdk_spark.Seed
import breez_sdk_spark.SyncWalletRequest
import breez_sdk_spark.defaultMysqlStorageConfig
import breez_sdk_spark.defaultServerConfig
import breez_sdk_spark.newSharedSdkContext

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

// --- bench config ---------------------------------------------------------

// Config for the funding/setup pipeline (treasurer top-up, sender seeding,
// invoice minting, post-run audit). The treasurer accumulates an unclaimed
// transfer backlog across runs; 32 concurrent claims (vs SDK default 4)
// drains it ~8× faster.
fun benchConfig(): breez_sdk_spark.Config = defaultServerConfig(Network.REGTEST).apply {
    maxConcurrentClaims = 32u
}

// Config for the per-request server handler. SDK defaults — handlers
// disconnect after one op and almost never run claims, so the bumped
// `maxConcurrentClaims` from benchConfig() would only add noise here.
fun handlerConfig(): breez_sdk_spark.Config = defaultServerConfig(Network.REGTEST)

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
    private val config = handlerConfig()

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
    // Populated only for successful sends; lets aggregate.py join slow
    // requests against `send_payment` tracing events to render
    // the per-payment phase breakdown.
    @SerialName("payment_id") val paymentId: String? = null,
)

class RequestTimings {
    var buildMs: Long? = null
    var opMs: Long? = null
    var prepareMs: Long? = null
    var sendMs: Long? = null
    var disconnectMs: Long? = null
    // Post-classified op label, set after the SDK call resolves spark-vs-LN.
    var opOverride: String? = null
    // Populated by the /send handler after the SDK returns.
    var paymentId: String? = null
}

// --- reserved user-ids (funding pipeline) ---------------------------------

const val TREASURER_USER_ID = "__treasurer__"

fun senderUserId(idx: Int): String = "__sender_${idx}__"

fun bankUserId(idx: Int): String = "__bank_${idx}__"
