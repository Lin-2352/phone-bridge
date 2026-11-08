package dev.phonebridge.app.devices

/** The send queue of one PC: what waits, what sends, what is paused. Pure rules; the service asks
 *  [next], reports [finish]/[fail]/[retry], and the Transfers screen draws [rows]. State shown is
 *  derived from the engine through [reconcile], never kept as a second truth. Local only: no wire
 *  message for pause, resume or cancel. */
enum class QueueState { Waiting, Sending, Paused, NeedsFile, Done, Failed }

data class QueueItem(
    val id: Long,
    val uri: String,
    val name: String,
    val size: Long,
    /** 16 bytes, kept across retries and restarts so a resume can match the receiver's partial. */
    val token: ByteArray,
    val state: QueueState = QueueState.Waiting,
)

const val MAX_QUEUED_PER_DEVICE = 64

class SendQueue {
    private val items = mutableListOf<QueueItem>()
    private var nextId = 0L
    var paused = false
        private set

    private fun unfinished() = items.count { it.state != QueueState.Done && it.state != QueueState.Failed }

    /** Returns the new id, or null when 64 unfinished items already wait. */
    fun enqueue(uri: String, name: String, size: Long, token: ByteArray): Long? {
        if (unfinished() >= MAX_QUEUED_PER_DEVICE) return null
        val id = ++nextId
        items += QueueItem(id, uri, name, size, token)
        return id
    }

    /** The item to send now (marked Sending); null while one sends, while paused, or when none waits. */
    fun next(): QueueItem? {
        if (paused || items.any { it.state == QueueState.Sending }) return null
        val i = items.indexOfFirst { it.state == QueueState.Waiting }
        if (i < 0) return null
        items[i] = items[i].copy(state = QueueState.Sending)
        return items[i]
    }

    private fun set(id: Long, s: QueueState) {
        val i = items.indexOfFirst { it.id == id }
        if (i >= 0) items[i] = items[i].copy(state = s)
    }

    fun finish(id: Long) = set(id, QueueState.Done)
    fun fail(id: Long) = set(id, QueueState.Failed)
    fun retry(id: Long) = set(id, QueueState.Waiting)
    fun needsFile(id: Long) = set(id, QueueState.NeedsFile)
    fun pause() { paused = true }
    fun resume() { paused = false }

    /** Removes an item; true when it was the one sending, so the caller cancels the transfer. */
    fun cancel(id: Long): Boolean {
        val was = items.any { it.id == id && it.state == QueueState.Sending }
        items.removeAll { it.id == id }
        return was
    }

    /** An item marked Sending that the engine does not run waits again instead of showing "sending". */
    fun reconcile(active: Set<Long>) {
        for (i in items.indices) {
            if (items[i].state == QueueState.Sending && items[i].id !in active) {
                items[i] = items[i].copy(state = QueueState.Waiting)
            }
        }
    }

    fun rows(): List<QueueItem> = items.map {
        if (paused && it.state == QueueState.Waiting) it.copy(state = QueueState.Paused) else it
    }

    /** Tab-separated lines, `%` escaping tab and newline; Done and Failed are not kept. */
    fun save(): String = items
        .filter { it.state != QueueState.Done && it.state != QueueState.Failed }
        .joinToString("") { "${it.id}\t${hex(it.token)}\t${it.size}\t${esc(it.name)}\t${esc(it.uri)}\n" }

    companion object {
        /** After a restart the queue is paused; an item whose source [readable] says is gone needs the file. */
        fun load(text: String, readable: (String) -> Boolean): SendQueue {
            val q = SendQueue()
            for (line in text.lines()) {
                val f = line.split('\t')
                if (f.size != 5) continue
                val id = f[0].toLongOrNull() ?: continue
                val token = unhex(f[1]) ?: continue
                val size = f[2].toLongOrNull() ?: continue
                if (q.unfinished() >= MAX_QUEUED_PER_DEVICE) break
                val uri = unesc(f[4])
                q.nextId = maxOf(q.nextId, id)
                q.items += QueueItem(
                    id, uri, unesc(f[3]), size, token,
                    if (readable(uri)) QueueState.Waiting else QueueState.NeedsFile,
                )
                q.paused = true
            }
            return q
        }

        private fun esc(s: String) = s.replace("%", "%25").replace("\t", "%09").replace("\n", "%0A").replace("\r", "%0D")
        private fun unesc(s: String) = s.replace("%0D", "\r").replace("%0A", "\n").replace("%09", "\t").replace("%25", "%")
        private fun hex(b: ByteArray) = b.joinToString("") { "%02x".format(it) }
        private fun unhex(s: String): ByteArray? {
            if (s.length != 32) return null
            return try {
                ByteArray(16) { s.substring(it * 2, it * 2 + 2).toInt(16).toByte() }
            } catch (e: NumberFormatException) {
                null
            }
        }
    }
}
