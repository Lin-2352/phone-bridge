package dev.phonebridge.app.ui

import org.junit.Assert.assertEquals
import org.junit.Test

class TransferRowsTest {
    private fun item(id: Long, state: TransferState, done: Long = 0, size: Long = 10) = TransferItem(
        id = id,
        peerId = "p",
        peerName = "Pixel",
        name = "file$id.bin",
        size = size,
        done = done,
        outgoing = true,
        state = state,
    )

    @Test
    fun running_before_waiting_before_finished() {
        val rows = transferRows(
            listOf(
                item(1, TransferState.Done),
                item(2, TransferState.Waiting),
                item(3, TransferState.Running, done = 5),
                item(4, TransferState.Failed),
            ),
        )
        // Running, then waiting; then everything finished, newest first.
        assertEquals(listOf(3L, 2L, 4L, 1L), rows.map { it.id })
    }

    @Test
    fun progress_text_is_rounded_to_whole_percent() {
        assertEquals("33%", transferRows(listOf(item(1, TransferState.Running, done = 1, size = 3))).single().percentText)
        assertEquals("67%", transferRows(listOf(item(1, TransferState.Running, done = 2, size = 3))).single().percentText)
        assertEquals("0%", transferRows(listOf(item(1, TransferState.Waiting, size = 0))).single().percentText)
    }

    @Test
    fun empty_list_text() {
        assertEquals("No transfers yet", emptyTransfersText())
    }
}
