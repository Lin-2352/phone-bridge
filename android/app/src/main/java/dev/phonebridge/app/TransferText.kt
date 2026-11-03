package dev.phonebridge.app

/** The transport reports a user or peer cancel as text ("transfer cancelled" / "the other device cancelled ..."). */
fun isCancelled(reason: String): Boolean = reason.contains("cancelled", ignoreCase = true)

private fun size(b: Long): String = when {
    b >= 1_000_000_000 -> "%.1f GB".format(b / 1e9)
    b >= 1_000_000 -> "${(b / 1_000_000)} MB"
    b >= 1_000 -> "${(b / 1_000)} KB"
    else -> "$b B"
}

/** "46 MB of 400 MB, 11 MB/s, 32 s left" for the progress notification; speed needs a moment of data. */
fun transferLine(done: Long, total: Long, elapsedMs: Long): String {
    val base = "${size(done)} of ${size(total)}"
    if (elapsedMs < 1_000 || done <= 0) return base
    val rate = done * 1000.0 / elapsedMs
    val left = ((total - done) / rate).toLong().coerceAtLeast(0)
    val eta = if (left >= 60) "${left / 60} min left" else "$left s left"
    val speed = if (rate >= 1_000_000) "${(rate / 1_000_000).toLong()} MB/s" else "${(rate / 1_000).toLong()} KB/s"
    return "$base \u00b7 $speed \u00b7 $eta"
}

fun failureTitle(name: String, cancelled: Boolean, outgoing: Boolean): String = when {
    cancelled && outgoing -> "Sending cancelled"
    cancelled -> "Receiving $name cancelled"
    outgoing -> "Could not send $name"
    else -> "Could not receive $name"
}

/** Items waiting for a Retry tap, newest [max] kept; [put] returns what it dropped so the caller can release it. */
class RetryStore<T>(private val max: Int) {
    private val items = java.util.LinkedHashMap<Long, T>()

    @Synchronized fun put(id: Long, v: T): List<T> {
        items[id] = v
        val out = ArrayList<T>()
        while (items.size > max) { val k = items.keys.first(); items.remove(k)?.let(out::add) }
        return out
    }

    @Synchronized fun take(id: Long): T? = items.remove(id)
}
