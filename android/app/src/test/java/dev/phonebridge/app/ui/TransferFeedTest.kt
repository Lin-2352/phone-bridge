package dev.phonebridge.app.ui

import org.junit.Assert.assertEquals
import org.junit.Test

class TransferFeedTest {
    private fun progress(done: Long, total: Long = 10, incoming: Boolean = true, name: String = "a.bin", peer: String = "p") =
        TransferSignal.Progress(peer, "Pixel", name, done, total, incoming)

    @Test
    fun progress_creates_a_running_row_then_updates_it() {
        var list = applyTransferSignal(emptyList(), progress(2))
        list = applyTransferSignal(list, progress(5))
        assertEquals(1, list.size)
        assertEquals(TransferState.Running, list.single().state)
        assertEquals(5L, list.single().done)
        // The screen shows what the phone does: an incoming file is "from" the PC, so not outgoing.
        assertEquals(false, list.single().outgoing)
    }

    @Test
    fun reaching_the_total_marks_the_row_done() {
        val list = applyTransferSignal(applyTransferSignal(emptyList(), progress(5)), progress(10))
        assertEquals(TransferState.Done, list.single().state)
    }

    @Test
    fun a_finished_name_sent_again_gets_a_new_row() {
        var list = applyTransferSignal(emptyList(), progress(10))
        list = applyTransferSignal(list, progress(1))
        assertEquals(2, list.size)
    }

    @Test
    fun a_failure_stops_the_running_rows_of_that_peer_only() {
        var list = applyTransferSignal(emptyList(), progress(2, peer = "p"))
        list = applyTransferSignal(list, progress(2, peer = "q", name = "b.bin"))
        list = applyTransferSignal(list, TransferSignal.Failed("p", cancelled = false))
        assertEquals(TransferState.Failed, list.first { it.peerId == "p" }.state)
        assertEquals(TransferState.Running, list.first { it.peerId == "q" }.state)
        val c = applyTransferSignal(list, TransferSignal.Failed("q", cancelled = true))
        assertEquals(TransferState.Cancelled, c.first { it.peerId == "q" }.state)
    }

    @Test
    fun the_list_keeps_the_newest_fifty_and_never_drops_a_running_row() {
        var list = emptyList<TransferItem>()
        for (i in 1..60) list = applyTransferSignal(list, progress(10, name = "f$i"))
        list = applyTransferSignal(list, progress(1, name = "live"))
        assertEquals(MAX_TRANSFERS, list.size)
        assertEquals(true, list.any { it.name == "live" && it.state == TransferState.Running })
        assertEquals(false, list.any { it.name == "f1" })
    }
}
