import java.nio.file.Files
import java.nio.file.Path
import java.time.LocalDateTime
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.concurrent.LinkedBlockingQueue
import kotlinx.serialization.KSerializer
import kotlinx.serialization.json.Json

// --- run-id helpers (shared by server + loadgen) --------------------------

val RUN_ID_FORMAT: DateTimeFormatter =
    DateTimeFormatter.ofPattern("yyyy-MM-dd'T'HH-mm-ss")

fun defaultRunId(): String =
    LocalDateTime.now(ZoneId.systemDefault()).format(RUN_ID_FORMAT)

// --- JSONL writer ---------------------------------------------------------

/**
 * Append-only JSONL writer with a dedicated daemon writer thread.
 *
 * Producers call [submit] from any thread; entries are queued onto an
 * unbounded [LinkedBlockingQueue] and drained onto disk by the writer
 * thread. Per-write flush keeps the file readable by another process
 * while the run is in flight.
 *
 * Unbounded queue: producers never block. At our throughput targets
 * (≤ a few thousand entries/sec, small entries) memory pressure is
 * negligible. If a future phase pushes that, swap in a bounded queue
 * with a back-pressure policy.
 *
 * The thread shape (vs. the earlier coroutine + Channel design) lets
 * the server use this from a non-coroutine context (Ktor's start hook)
 * without forcing the whole server into a single coroutine scope.
 */
class JsonlWriter<T : Any>(
    path: Path,
    private val serializer: KSerializer<T>,
) : AutoCloseable {
    private val codec = Json { encodeDefaults = true }
    private val queue = LinkedBlockingQueue<Any>()  // T or SENTINEL
    private val writer = Files.newBufferedWriter(path)

    @Volatile private var closing = false

    private val thread: Thread = Thread {
        try {
            while (true) {
                val item = queue.take()
                if (item === SENTINEL) break
                @Suppress("UNCHECKED_CAST")
                writer.write(codec.encodeToString(serializer, item as T))
                writer.newLine()
                writer.flush()
            }
        } catch (e: InterruptedException) {
            Thread.currentThread().interrupt()
        }
    }.apply {
        isDaemon = true
        name = "jsonl-writer-${path.fileName}"
        start()
    }

    /** Non-blocking submit. Drops silently if the writer has already been closed. */
    fun submit(entry: T) {
        if (closing) return
        queue.add(entry)
    }

    /**
     * Drains the queue and closes the underlying file. Idempotent.
     *
     * Blocks up to ~10s for the writer thread to finish; if a producer
     * keeps submitting after [close] was called, those entries are
     * silently dropped (see [submit]).
     */
    override fun close() {
        if (closing) return
        closing = true
        queue.add(SENTINEL)
        thread.join(10_000)
        writer.close()
    }

    companion object {
        private val SENTINEL = Any()
    }
}
