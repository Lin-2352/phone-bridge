package dev.phonebridge.app.devices

import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ClipHistoryTest {
    private class Mem : ClipBlob {
        var bytes: ByteArray? = null
        override fun read() = bytes
        override fun write(bytes: ByteArray) { this.bytes = bytes }
        override fun delete() { bytes = null }
    }

    private fun key(): SecretKey = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()
    private fun history(mem: Mem, k: SecretKey? = key(), on: Boolean = true) =
        ClipHistory(mem, AesGcmClipCipher { k }, on)

    @Test
    fun off_by_default_stores_nothing() {
        val m = Mem()
        assertEquals(Stored.Off, history(m, on = false).add("hello", false, 100))
        assertNull(m.bytes)
    }

    @Test
    fun sensitive_clip_is_never_stored() {
        val m = Mem()
        val h = history(m)
        assertEquals(Stored.Sensitive, h.add("hunter2", true, 100))
        assertTrue(h.entries(100).isEmpty())
    }

    @Test
    fun one_time_code_is_never_stored() {
        val h = history(Mem())
        for (c in listOf("123456", "123 456", "12-34-56", " 4821 ")) {
            assertEquals(c, Stored.OneTimeCode, h.add(c, false, 100))
        }
        assertEquals(Stored.Yes, h.add("call me at 555 0100 tomorrow", false, 100))
        assertEquals(1, h.entries(100).size)
    }

    @Test
    fun the_stored_bytes_do_not_contain_the_text() {
        val m = Mem()
        history(m).add("the quick brown fox", false, 100)
        assertFalse(String(m.bytes!!, Charsets.ISO_8859_1).contains("quick"))
    }

    @Test
    fun entries_older_than_7_days_are_gone() {
        val h = history(Mem())
        h.add("old", false, 1_000)
        h.add("new", false, 1_000 + ClipHistory.KEEP_SECS - 1)
        assertEquals(listOf("new"), h.entries(1_000 + ClipHistory.KEEP_SECS).map { it.text })
    }

    @Test
    fun a_clip_dated_in_the_future_does_not_live_forever() {
        val h = history(Mem())
        h.add("from a wrong clock", false, 10 * ClipHistory.KEEP_SECS)
        assertTrue(h.entries(100).isEmpty())
        h.add("now", false, 100)
        assertEquals(listOf("now"), h.entries(100).map { it.text })
    }

    @Test
    fun history_with_lost_key_reads_as_empty_not_garbage() {
        val m = Mem()
        history(m, key()).add("secret plans", false, 100)
        val other = history(m, key())
        assertTrue(other.entries(100).isEmpty())
        assertNull("an unreadable file is deleted", m.bytes)
        assertTrue(history(m, null).entries(100).isEmpty())
    }

    @Test
    fun wrong_key_fails_closed() {
        val m = Mem()
        history(m, key()).add("x marks the spot", false, 100)
        assertNull(AesGcmClipCipher { key() }.open(m.bytes!!.copyOfRange(12, m.bytes!!.size), ByteArray(9)))
    }

    @Test
    fun editing_the_stored_time_breaks_the_record() {
        val m = Mem()
        val k = key()
        history(m, k).add("keep me", false, 1_000)
        val b = m.bytes!!.copyOf()
        b[4 + 7] = (b[4 + 7] + 1).toByte() // the created time of the first record
        m.bytes = b
        assertTrue("a record with an edited time is dropped", history(m, k).entries(1_000).isEmpty())
    }

    @Test
    fun search_matches_case_insensitively_newest_first() {
        val h = history(Mem())
        h.add("Invoice 2024 from ACME", false, 100)
        h.add("lunch plan", false, 200)
        h.add("acme contract draft", false, 300)
        assertEquals(listOf("acme contract draft", "Invoice 2024 from ACME"), h.search("AcMe", 300).map { it.text })
        assertEquals(3, h.search("", 300).size)
    }

    @Test
    fun keep_history_off_wipes_clip_history() {
        val m = Mem()
        val h = history(m)
        h.add("something", false, 100)
        assertNotNull(m.bytes)
        h.setEnabled(false)
        assertNull(m.bytes)
        assertTrue(h.entries(100).isEmpty())
    }

    @Test
    fun size_and_count_are_capped() {
        val h = history(Mem())
        assertEquals(Stored.TooLong, h.add("x".repeat(ClipHistory.MAX_CLIP + 1), false, 100))
        assertEquals(Stored.Empty, h.add("   ", false, 100))
        for (i in 0 until ClipHistory.MAX_ENTRIES + 5) h.add("clip number $i", false, 100L + i)
        val e = h.entries(400)
        assertEquals(ClipHistory.MAX_ENTRIES, e.size)
        assertEquals("clip number 5", e[0].text)
    }
}
