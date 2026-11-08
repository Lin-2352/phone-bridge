package dev.phonebridge.app.ui

import androidx.compose.runtime.mutableStateListOf

/** Where a transfer is. */
enum class TransferState { Waiting, Running, Done, Failed, Cancelled }

/** One file moving between this phone and a PC (what the Transfers screen shows). */
data class TransferItem(
    val id: Long,
    val peerId: String,
    val peerName: String,
    val name: String,
    val size: Long,
    val done: Long,
    val outgoing: Boolean,
    val state: TransferState,
)

/** A transfer as its row shows it: the title, a status line, and the progress percentage text. */
data class TransferListRow(
    val id: Long,
    val title: String,
    val status: String,
    val state: TransferState,
    val percentText: String,
)

/** The transfers this process knows about, fed where the service posts transfer notifications. */
object Transfers {
    val items = mutableStateListOf<TransferItem>()

    /** Applies a signal from the service; safe from any thread (the list is replaced as a whole). */
    @Synchronized
    fun feed(signal: TransferSignal) {
        val next = applyTransferSignal(items.toList(), signal)
        items.clear()
        items.addAll(next)
    }
}

/** Running first, then waiting, then everything finished with the newest first. */
fun transferRows(items: List<TransferItem>): List<TransferListRow> {
    val rank = { s: TransferState ->
        when (s) {
            TransferState.Running -> 0
            TransferState.Waiting -> 1
            else -> 2
        }
    }
    return items
        .sortedWith(compareBy<TransferItem>({ rank(it.state) }, { if (rank(it.state) == 2) -it.id else it.id }))
        .map { t ->
            val pct = if (t.size <= 0L) 0 else Math.round(t.done * 100.0 / t.size).toInt()
            TransferListRow(
                id = t.id,
                title = t.name,
                status = statusText(t),
                state = t.state,
                percentText = "$pct%",
            )
        }
}

private fun statusText(t: TransferItem): String {
    val who = if (t.outgoing) "to ${t.peerName}" else "from ${t.peerName}"
    return when (t.state) {
        TransferState.Waiting -> "Waiting $who"
        TransferState.Running -> if (t.outgoing) "Sending $who" else "Receiving $who"
        TransferState.Done -> "Done $who"
        TransferState.Failed -> "Failed $who"
        TransferState.Cancelled -> "Cancelled $who"
    }
}

/** What an empty Transfers screen says. */
fun emptyTransfersText(): String = "No transfers yet"
