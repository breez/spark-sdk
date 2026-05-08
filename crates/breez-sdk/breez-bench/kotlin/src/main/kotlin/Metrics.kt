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

/**
 * A single 1-Hz metrics tick written to `metrics.jsonl`.
 *
 * `-1` for a numeric field means "unavailable on this platform / this
 * tick" rather than zero. Keeping it numeric (vs. a JSON null) keeps
 * downstream aggregators trivial.
 *
 * `mysql_conns` counts rows in `INFORMATION_SCHEMA.PROCESSLIST` whose
 * `DB` matches the bench database name — server-authoritative count of
 * connections open against that DB. Coarse: if multiple bench
 * processes share the DB this over-counts; fine for the v1 single-
 * process bench.
 *
 * `remote_tcp_sockets` is all non-loopback TCP sockets in any state
 * held by this process. Includes ephemeral TIME_WAIT — those still
 * consume local ports, which is the failure mode we care about (port
 * exhaustion at high RPS during cold-start churn).
 */
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

    /**
     * `/proc/net/tcp{,6}` writes each address as a sequence of u32s in
     * the host's native byte order. On x86/ARM (LE) that means each
     * u32's hex string has the low byte first; we recover the raw
     * network bytes by reading each u32, then writing it out little-
     * endian. From there [InetAddress.isLoopbackAddress] gives a
     * correct answer for both IPv4 and IPv4-mapped/native IPv6.
     */
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

private class MacosProcessMetricsCollector(private val pid: Long) : ProcessMetricsCollector {
    override fun rssKb(): Long {
        // `ps -o rss=` prints just the RSS in KB, no header.
        val output = runCommand(listOf("ps", "-o", "rss=", "-p", pid.toString()))
            ?: return -1
        return output.trim().toLongOrNull() ?: -1
    }

    override fun remoteTcpSocketCount(): Int {
        // `lsof -nP -iTCP -p PID` lists every TCP socket the process holds.
        // The NAME column has shape "LOCAL:PORT->REMOTE:PORT (STATE)" for
        // connected sockets and "*:PORT (LISTEN)" for listeners. We count
        // connected, non-loopback entries.
        val output = runCommand(listOf("lsof", "-nP", "-iTCP", "-p", pid.toString()))
            ?: return -1
        var count = 0
        output.lineSequence().drop(1).forEach { line ->
            if (line.isBlank()) return@forEach
            val tcpAt = line.indexOf(" TCP ")
            if (tcpAt < 0) return@forEach
            val nameField = line.substring(tcpAt + " TCP ".length)
            val arrow = nameField.indexOf("->")
            if (arrow < 0) return@forEach  // not a connected socket
            val remoteAndAfter = nameField.substring(arrow + 2)
            val remote = remoteAndAfter.substringBefore(' ')
            val remoteHost = remote.substringBeforeLast(':')
                .removePrefix("[").removeSuffix("]")
            if (isLoopbackHost(remoteHost)) return@forEach
            count++
        }
        return count
    }

    private fun isLoopbackHost(host: String): Boolean {
        // Avoid hitting DNS — only treat numeric IPv4/IPv6 forms as loopback.
        return try {
            // lsof -n prints numeric addresses, so InetAddress.getByName here
            // won't perform DNS lookups in practice.
            InetAddress.getByName(host).isLoopbackAddress
        } catch (_: Exception) {
            false
        }
    }
}

private fun runCommand(cmd: List<String>): String? {
    return try {
        val proc = ProcessBuilder(cmd)
            .redirectErrorStream(false)
            .redirectError(ProcessBuilder.Redirect.DISCARD)
            .start()
        val finished = proc.waitFor(5, TimeUnit.SECONDS)
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
 * [JsonlWriter]. Cheap on Linux (file reads + a JDBC query); on macOS,
 * `lsof` adds tens of milliseconds per tick which is fine at 1 Hz.
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
        return MetricSample(
            ts = System.currentTimeMillis(),
            rssKb = collector.rssKb(),
            heapUsedBytes = rt.totalMemory() - rt.freeMemory(),
            heapTotalBytes = rt.totalMemory(),
            threadCount = threadMx.threadCount,
            fdCount = osMx?.openFileDescriptorCount ?: -1L,
            mysqlConns = mysqlPoller.count(),
            remoteTcpSockets = collector.remoteTcpSocketCount(),
        )
    }
}
