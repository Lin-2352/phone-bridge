package dev.phonebridge.app.devices

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class SendQueueTest {
    private fun add(q: SendQueue, n: String) = q.enqueue("content://x/$n", n, 10, ByteArray(16) { n.length.toByte() })!!

    @Test
    fun items_send_in_order_one_at_a_time() {
        val q = SendQueue()
        val a = add(q, "a"); val b = add(q, "b")
        assertEquals(a, q.next()!!.id)
        assertNull(q.next())
        q.finish(a)
        assertEquals(b, q.next()!!.id)
    }

    @Test
    fun pause_stops_the_next_item_not_the_current_one() {
        val q = SendQueue()
        val a = add(q, "a"); val b = add(q, "b")
        q.next()
        q.pause()
        assertEquals(QueueState.Sending, q.rows()[0].state)
        q.finish(a)
        assertNull(q.next())
        assertEquals(QueueState.Paused, q.rows()[1].state)
        q.resume()
        assertEquals(b, q.next()!!.id)
    }

    @Test
    fun cancel_tells_the_caller_when_it_was_sending() {
        val q = SendQueue()
        val a = add(q, "a"); val b = add(q, "b")
        q.next()
        assertTrue(q.cancel(a))
        assertFalse(q.cancel(b))
    }

    @Test
    fun max_64_unfinished_items() {
        val q = SendQueue()
        repeat(MAX_QUEUED_PER_DEVICE) { add(q, "f$it") }
        assertNull(q.enqueue("u", "x", 1, ByteArray(16)))
        val first = q.next()!!
        q.finish(first.id)
        assertNotNull(q.enqueue("u", "y", 1, ByteArray(16)))
    }

    @Test
    fun row_state_matches_engine_state() {
        val q = SendQueue()
        val a = add(q, "a")
        q.next()
        q.reconcile(emptySet())
        assertEquals(QueueState.Waiting, q.rows()[0].state)
        q.next()
        q.reconcile(setOf(a))
        assertEquals(QueueState.Sending, q.rows()[0].state)
    }

    @Test
    fun queue_survives_restart_as_paused_and_keeps_the_token() {
        val q = SendQueue()
        val a = add(q, "a"); add(q, "bb")
        q.next(); q.finish(a)
        val back = SendQueue.load(q.save()) { it.endsWith("bb") }
        assertTrue(back.paused)
        assertEquals(1, back.rows().size)
        assertEquals(QueueState.Paused, back.rows()[0].state)
        assertEquals(2, back.rows()[0].token[0].toInt())
        assertNull(back.next())
    }

    @Test
    fun a_source_that_cannot_be_read_after_restart_needs_the_file() {
        val q = SendQueue(); add(q, "gone")
        assertEquals(QueueState.NeedsFile, SendQueue.load(q.save()) { false }.rows()[0].state)
    }

    @Test
    fun odd_names_round_trip_and_bad_lines_are_skipped() {
        val q = SendQueue()
        q.enqueue("content://a/b%c", "na\tme\n.bin", 5, ByteArray(16))
        val back = SendQueue.load(q.save() + "junk\n1\t2\n") { true }
        assertEquals(1, back.rows().size)
        assertEquals("na\tme\n.bin", back.rows()[0].name)
        assertEquals("content://a/b%c", back.rows()[0].uri)
    }
}
