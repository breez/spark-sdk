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
 * Append-only JSONL writer; daemon thread drains an unbounded queue.
 * Per-write flush so the file is readable mid-run.
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
