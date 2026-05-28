import com.sun.management.UnixOperatingSystemMXBean
import java.lang.management.ManagementFactory
import java.net.InetAddress
import java.net.URI
import java.nio.file.Files
import java.nio.file.Path
import java.sql.Connection
import java.sql.DriverManager
import java.util.concurrent.TimeUnit
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

// --- record shape ---------------------------------------------------------

/** 1Hz process-metrics tick. Numeric `-1` means "unavailable". */
@Serializable
data class MetricSample(
    val ts: Long,
    @SerialName("rss_kb") val rssKb: Long,
    @SerialName("heap_used_bytes") val heapUsedBytes: Long,
    @SerialName("heap_total_bytes") val heapTotalBytes: Long,
    @SerialName("thread_count") val threadCount: Int,
    @SerialName("fd_count") val fdCount: Long,
    @SerialName("mysql_conns") val mysqlConns: Int,
    @SerialName("remote_tcp_sockets") val remoteTcpSockets: Int,
    /** Fraction of total host CPU (summed across cores) used by this
     *  JVM process, in [0.0, 1.0]. A value of 1.0 means the process is
     *  pegging every core. -1.0 if the JVM hasn't sampled yet (typical
     *  for the first ~1 sec) or the platform doesn't expose it. */
    @SerialName("process_cpu_load") val processCpuLoad: Double = -1.0,
    /** Whole-host CPU usage in [0.0, 1.0], for context (other processes
     *  + ours). -1.0 if unavailable. */
    @SerialName("host_cpu_load") val hostCpuLoad: Double = -1.0,
    /** Number of logical CPUs visible to the JVM. Constant across the
     *  run; included per-sample so aggregators don't need a separate
     *  source. -1 if unavailable. */
    @SerialName("available_processors") val availableProcessors: Int = -1,
)

// --- mysql url parsing ----------------------------------------------------

data class MysqlUrlParts(
    val host: String,
    val port: Int,
    val database: String,
    val user: String,
    val password: String,
) {
    /** JDBC URL for the standard MySQL connector. Database is included so
     *  the DriverManager session is scoped to the bench DB by default. */
    fun toJdbcUrl(): String = "jdbc:mysql://$host:$port/$database"
}

/**
 * Parses `mysql://user:pass@host:port/dbname` into its parts.
 *
 * The bench takes a single `--mysql-url` argument and the SDK consumes
 * it directly, but the JDBC sampler needs structured access to derive
 * a `jdbc:mysql://…` URL and to extract the database name for the
 * `PROCESSLIST` filter.
 */
fun parseMysqlUrl(url: String): MysqlUrlParts {
    val uri = URI(url)
    require(uri.scheme == "mysql") {
        "Expected mysql:// URL, got: ${uri.scheme}://… in $url"
    }
    val userInfo = uri.userInfo
        ?: error("MySQL URL is missing user:password component: $url")
    val (user, password) = userInfo.split(":", limit = 2).also {
        require(it.size == 2) { "MySQL URL userinfo must be 'user:password': $url" }
    }.let { it[0] to it[1] }
    val host = uri.host ?: error("MySQL URL is missing host: $url")
    val port = uri.port.let { if (it < 0) 3306 else it }
    val database = uri.path?.removePrefix("/").orEmpty()
    require(database.isNotEmpty()) { "MySQL URL is missing database name: $url" }
    return MysqlUrlParts(host, port, database, user, password)
}

// --- mysql connection poller ---------------------------------------------

/**
 * Holds one persistent JDBC connection and runs a `COUNT(*)` against
 * `INFORMATION_SCHEMA.PROCESSLIST` filtered by database name on each tick.
 *
 * If the connection drops mid-run (network blip, MySQL restart) the
 * next [count] call re-establishes it. A failure within a single tick
 * surfaces as `-1` in [MetricSample.mysqlConns] and a stderr line —
 * the sampler doesn't crash.
 */
class MysqlConnPoller(
    private val parts: MysqlUrlParts,
) : AutoCloseable {
    private var conn: Connection? = null

    @Synchronized
    fun count(): Int {
        val c = ensureConn() ?: return -1
        return try {
            c.prepareStatement(
                "SELECT COUNT(*) FROM INFORMATION_SCHEMA.PROCESSLIST WHERE DB = ?"
            ).use { ps ->
                ps.setString(1, parts.database)
                ps.executeQuery().use { rs -> if (rs.next()) rs.getInt(1) else -1 }
            }
        } catch (e: Exception) {
            System.err.println("[metrics] mysql sample failed: ${e.message}")
            try { c.close() } catch (_: Exception) {}
            conn = null
            -1
        }
    }

    private fun ensureConn(): Connection? {
        var c = conn
        if (c == null || c.isClosed) {
            c = try {
                DriverManager.getConnection(parts.toJdbcUrl(), parts.user, parts.password)
            } catch (e: Exception) {
                System.err.println("[metrics] mysql connect failed: ${e.message}")
                null
            }
            conn = c
        }
        return c
    }

    @Synchronized
    override fun close() {
        try { conn?.close() } catch (_: Exception) {}
        conn = null
    }
}

// --- process metrics: platform shim --------------------------------------

interface ProcessMetricsCollector {
    /** RSS in KB. Returns -1 if unavailable. */
    fun rssKb(): Long
    /** Count of non-loopback TCP sockets (any state). Returns -1 if unavailable. */
    fun remoteTcpSocketCount(): Int

    companion object {
        fun create(): ProcessMetricsCollector {
            val os = System.getProperty("os.name").lowercase()
            return when {
                os.contains("linux") -> LinuxProcessMetricsCollector()
                os.contains("mac") || os.contains("darwin") ->
                    MacosProcessMetricsCollector(ProcessHandle.current().pid())
                else -> error("Unsupported OS for bench metrics: $os (expected linux or macos)")
            }
        }
    }
}

private class LinuxProcessMetricsCollector : ProcessMetricsCollector {
    override fun rssKb(): Long {
        return runCatching {
            Files.lines(Path.of("/proc/self/status")).use { lines ->
                lines.filter { it.startsWith("VmRSS:") }
                    .findFirst()
                    .map { line ->
                        // "VmRSS:    123456 kB" → second whitespace-delimited token
                        line.trim().split(Regex("\\s+"))
                            .getOrNull(1)
                            ?.toLongOrNull()
                            ?: -1L
                    }
                    .orElse(-1L)
            }
        }.getOrElse { -1L }
    }

    override fun remoteTcpSocketCount(): Int {
        var count = 0
        var sawAny = false
        for (path in listOf("/proc/self/net/tcp", "/proc/self/net/tcp6")) {
            val p = Path.of(path)
            if (!Files.isReadable(p)) continue
            sawAny = true
            try {
                Files.lines(p).use { lines ->
                    lines.skip(1).forEach { raw ->
                        // Format: "  0: LOCAL_HEX:PORT REMOTE_HEX:PORT STATE …"
                        val parts = raw.trim().split(Regex("\\s+"))
                        if (parts.size < 4) return@forEach
                        val state = parts[3]
                        if (state == "0A") return@forEach  // LISTEN — not a remote connection
                        val remoteHex = parts[2].substringBefore(':')
                        if (isLoopbackProcAddr(remoteHex)) return@forEach
                        count++
                    }
                }
            } catch (_: Exception) {
                // Per-file failure is non-fatal; a transient read race on /proc is OK.
            }
        }
        return if (sawAny) count else -1
    }

    // /proc/net/tcp{,6} writes addresses as native-byte-order u32s; reverse to
    // network order before handing to InetAddress.
    private fun isLoopbackProcAddr(hex: String): Boolean {
        val bytes = decodeProcAddr(hex) ?: return false
        return try {
            InetAddress.getByAddress(bytes).isLoopbackAddress
        } catch (_: Exception) {
            false
        }
    }

    private fun decodeProcAddr(hex: String): ByteArray? {
        if (hex.length != 8 && hex.length != 32) return null
        if (hex.length % 8 != 0) return null
        val numU32 = hex.length / 8
        val out = ByteArray(numU32 * 4)
        for (i in 0 until numU32) {
            val u32 = hex.substring(i * 8, (i + 1) * 8).toLongOrNull(16) ?: return null
            // Write u32 in LE so out[…] mirrors the on-the-wire (network) byte order.
            out[i * 4 + 0] = (u32 and 0xFF).toByte()
            out[i * 4 + 1] = ((u32 ushr 8) and 0xFF).toByte()
            out[i * 4 + 2] = ((u32 ushr 16) and 0xFF).toByte()
            out[i * 4 + 3] = ((u32 ushr 24) and 0xFF).toByte()
        }
        return out
    }
}

// RSS via `ps -o rss=`; no socket count on macOS (no JVM API). Use Linux for measurements.
private class MacosProcessMetricsCollector(private val pid: Long) : ProcessMetricsCollector {
    override fun rssKb(): Long {
        val output = runCommand(listOf("ps", "-o", "rss=", "-p", pid.toString()), timeoutSecs = 5)
            ?: return -1
        return output.trim().toLongOrNull() ?: -1
    }

    override fun remoteTcpSocketCount(): Int = -1
}

private fun runCommand(cmd: List<String>, timeoutSecs: Long = 5): String? {
    return try {
        val proc = ProcessBuilder(cmd)
            .redirectErrorStream(false)
            .redirectError(ProcessBuilder.Redirect.DISCARD)
            .start()
        val finished = proc.waitFor(timeoutSecs, TimeUnit.SECONDS)
        if (!finished) {
            proc.destroyForcibly()
            return null
        }
        val out = proc.inputStream.bufferedReader().readText()
        if (proc.exitValue() == 0) out else null
    } catch (_: Exception) {
        null
    }
}

// --- sampler --------------------------------------------------------------

/**
 * Daemon thread that emits one [MetricSample] per [intervalMs] to a
 * [JsonlWriter]. Cheap on Linux (file reads + a JDBC query); on macOS
 * sockets are unavailable (see [MacosProcessMetricsCollector]).
 *
 * Errors inside a single tick are swallowed (a stderr line + best-effort
 * fields) so the sampler outlives transient hiccups (MySQL flap, /proc
 * read race, etc.).
 */
class MetricsSampler(
    private val collector: ProcessMetricsCollector,
    private val mysqlPoller: MysqlConnPoller,
    private val writer: JsonlWriter<MetricSample>,
    private val intervalMs: Long = 1_000L,
) {
    private val osMx = ManagementFactory.getOperatingSystemMXBean() as? UnixOperatingSystemMXBean
    private val threadMx = ManagementFactory.getThreadMXBean()
    // availableProcessors() can change at runtime in containerised envs;
    // sample once here since MetricsSampler is created once per server run.
    private val cpuCount: Int = Runtime.getRuntime().availableProcessors()

    @Volatile private var thread: Thread? = null

    fun start() {
        check(thread == null) { "MetricsSampler already started" }
        val t = Thread {
            try {
                while (!Thread.currentThread().isInterrupted) {
                    try {
                        writer.submit(sampleNow())
                    } catch (e: Throwable) {
                        System.err.println("[metrics] sample tick failed: ${e.message}")
                    }
                    Thread.sleep(intervalMs)
                }
            } catch (_: InterruptedException) {
                Thread.currentThread().interrupt()
            }
        }
        t.isDaemon = true
        t.name = "metrics-sampler"
        thread = t
        t.start()
    }

    fun stop() {
        thread?.interrupt()
        thread = null
    }

    private fun sampleNow(): MetricSample {
        val rt = Runtime.getRuntime()
        val procCpu = osMx?.processCpuLoad ?: -1.0
        val hostCpu = osMx?.cpuLoad ?: -1.0
        return MetricSample(
            ts = System.currentTimeMillis(),
            rssKb = collector.rssKb(),
            heapUsedBytes = rt.totalMemory() - rt.freeMemory(),
            heapTotalBytes = rt.totalMemory(),
            threadCount = threadMx.threadCount,
            fdCount = osMx?.openFileDescriptorCount ?: -1L,
            mysqlConns = mysqlPoller.count(),
            remoteTcpSockets = collector.remoteTcpSocketCount(),
            processCpuLoad = procCpu,
            hostCpuLoad = hostCpu,
            availableProcessors = cpuCount,
        )
    }
}
