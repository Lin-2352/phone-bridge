package dev.phonebridge.app.ui

/** The most transfers kept for the Transfers screen; the oldest finished ones go first. */
const val MAX_TRANSFERS = 50

/** What the service learns about a transfer, in the shape the screen needs. */
sealed interface TransferSignal {
    data class Progress(
        val peerId: String,
        val peerName: String,
        val name: String,
        val done: Long,
        val total: Long,
        val incoming: Boolean,
    ) : TransferSignal

    /** A transfer with this peer failed or was cancelled; its running rows stop. */
    data class Failed(val peerId: String, val cancelled: Boolean) : TransferSignal
}

/**
 * The list after a signal. Progress finds the row for (peer, file, direction) or adds one; reaching the total
 * marks it done. A failure ends every running row of that peer. Ids grow, so newer rows sort first once done.
 */
fun applyTransferSignal(items: List<TransferItem>, signal: TransferSignal): List<TransferItem> {
    val next = when (signal) {
        is TransferSignal.Progress -> {
            val at = items.indexOfFirst {
                it.peerId == signal.peerId && it.name == signal.name && it.outgoing == !signal.incoming &&
                    (it.state == TransferState.Running || it.state == TransferState.Waiting)
            }
            val state = if (signal.total > 0 && signal.done >= signal.total) TransferState.Done else TransferState.Running
            if (at >= 0) {
                items.toMutableList().also { it[at] = it[at].copy(done = signal.done, size = signal.total, state = state) }
            } else {
                items + TransferItem(
                    id = (items.maxOfOrNull { it.id } ?: 0L) + 1,
                    peerId = signal.peerId,
                    peerName = signal.peerName,
                    name = signal.name,
                    size = signal.total,
                    done = signal.done,
                    outgoing = !signal.incoming,
                    state = state,
                )
            }
        }
        is TransferSignal.Failed -> items.map {
            if (it.peerId == signal.peerId && it.state == TransferState.Running) {
                it.copy(state = if (signal.cancelled) TransferState.Cancelled else TransferState.Failed)
            } else {
                it
            }
        }
    }
    if (next.size <= MAX_TRANSFERS) return next
    // Drop the oldest finished rows first; never a running one.
    val finished = next.filter { it.state != TransferState.Running && it.state != TransferState.Waiting }.sortedBy { it.id }
    val drop = finished.take(next.size - MAX_TRANSFERS).map { it.id }.toSet()
    return next.filter { it.id !in drop }
}
