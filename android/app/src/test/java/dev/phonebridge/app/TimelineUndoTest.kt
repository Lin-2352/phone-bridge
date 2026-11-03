package dev.phonebridge.app

import java.io.FileNotFoundException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class TimelineUndoTest {
    private val now = 1_800_000_000_000L

    private class FakeFile : TlFile {
        var bytes: ByteArray? = null
        override fun read(): ByteArray = bytes ?: throw FileNotFoundException()
        override fun write(b: ByteArray) { bytes = b }
        override fun delete() { bytes = null }
    }

    private class Rig(on: Boolean = true) {
        val file = FakeFile()
        val io = ArrayDeque<Runnable>()
        val later = ArrayDeque<Runnable>()
        val store = TimelineStore(exec = { io.add(it) }, later = { later.add(it) }, log = {}, now = { 1_800_000_000_000L })
        init { store.start(file, on); runAll() }
        fun runAll() { while (io.isNotEmpty() || later.isNotEmpty()) { if (io.isNotEmpty()) io.removeFirst().run() else later.removeFirst().run() } }
    }

    private fun e(n: Int) = clipTextEntry("PC", "x".repeat(n), at = now - n * 1000L)

    @Test fun remove_takes_the_entry_out_and_the_file_follows() {
        val r = Rig()
        r.store.add(e(1)); r.store.add(e(2)); r.runAll()
        assertTrue(r.store.remove(e(1)))
        r.runAll()
        assertEquals(listOf(e(2)), r.store.entries.value)
        assertEquals(listOf(e(2)), decode(r.file.bytes!!.toString(Charsets.UTF_8)))
    }

    @Test fun undo_puts_it_back_in_its_place() {
        val r = Rig()
        r.store.add(e(1)); r.store.add(e(2)); r.store.add(e(3)); r.runAll()
        val victim = e(2)
        r.store.remove(victim)
        r.store.restore(victim)
        r.runAll()
        assertEquals(listOf(e(1), e(2), e(3)), r.store.entries.value)
        assertEquals(3, decode(r.file.bytes!!.toString(Charsets.UTF_8)).size)
    }

    @Test fun removing_one_of_two_equal_entries_removes_one() {
        val r = Rig()
        r.store.add(e(1)); r.store.add(e(1)); r.runAll()
        assertTrue(r.store.remove(e(1)))
        assertEquals(1, r.store.entries.value.size)
    }

    @Test fun removing_something_not_there_says_so() {
        val r = Rig()
        r.store.add(e(1)); r.runAll()
        assertFalse(r.store.remove(e(9)))
        assertEquals(1, r.store.entries.value.size)
    }

    @Test fun undo_after_history_was_turned_off_records_nothing() {
        val r = Rig()
        r.store.add(e(1)); r.runAll()
        val victim = e(1)
        r.store.remove(victim)
        r.store.setEnabled(false)
        r.store.restore(victim)
        r.runAll()
        assertTrue(r.store.entries.value.isEmpty())
        assertEquals(null, r.file.bytes)
    }
}
