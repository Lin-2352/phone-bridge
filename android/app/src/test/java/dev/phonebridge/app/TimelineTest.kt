package dev.phonebridge.app

import dev.phonebridge.app.ui.timeline.RecentItem
import dev.phonebridge.app.ui.timeline.recentItem
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class TimelineTest {
    private val day = 24L * 3600 * 1000
    private val now = 1_800_000_000_000L

    private fun pic(w: Int = 1080, h: Int = 2412, size: Long = 1_200_000) =
        TlEntry(TlKind.ClipImage, TlDir.ToPc, "Lin's PC", now, size = size, width = w, height = h)

    private fun at(ms: Long, peer: String = "p") = TlEntry(TlKind.ClipText, TlDir.ToPc, peer, ms, chars = 1)

    @Test fun keeps_newest_1000_within_30_days() {
        // Oldest first on purpose: prune sorts newest first.
        val many = (0 until 1200).map { at(now - 1200 + it) }
        val kept = prune(many, now)
        assertEquals(TL_MAX_ENTRIES, kept.size)
        assertEquals(now - 1, kept.first().at)
        assertEquals(now - 1000, kept.last().at)

        val old = listOf(at(now - 30 * day), at(now - 30 * day - 1), at(now - 31 * day))
        assertEquals(listOf(now - 30 * day), prune(old, now).map { it.at })

        // Ties keep their order (stable).
        val tie = listOf(at(now, "a"), at(now, "b"))
        assertEquals(listOf("a", "b"), prune(tie, now).map { it.peer })
    }

    @Test fun round_trips_every_kind() {
        val list = listOf(
            fileEntry("PC \"one\"", TlDir.ToPhone, "photo.jpg", 3_400_000, "content://media/external/downloads/42", at = now),
            fileEntry("PC", TlDir.ToPc, "", 0, null, at = now - 1),
            clipTextEntry("PC", "hello", TlDir.ToPhone, at = now - 2),
            clipImageEntry("PC", TlDir.ToPc, 5_000_000_000L, 1080 to 2412, at = now - 3),
            clipImageEntry("PC", TlDir.ToPhone, 10, null, at = now - 4),
            linkEntry("PC\ttab\nline", "https://example.com/", TlDir.ToPc, at = now - 5),
            notificationEntry("PC", "WhatsApp", at = now - 6),
            TlEntry(TlKind.Notification, TlDir.ToPc, "", now - 7),
        )
        assertEquals(list, decode(encode(list)))
        assertEquals(emptyList<TlEntry>(), decode(encode(emptyList())))
        // "" survives as "", not as absent.
        assertEquals("", decode(encode(list))[1].name)
        assertNull(decode(encode(list))[1].uri)
    }

    @Test fun link_and_text_entries_store_counts_not_text() {
        val t = clipTextEntry("PC", "secret words")
        assertEquals(12, t.chars)
        assertFalse(encode(listOf(t)).contains("secret"))
        val l = linkEntry("PC", "https://bank.example/reset?token=abc")
        assertEquals(36, l.chars)
        assertFalse(encode(listOf(l)).contains("bank"))
        assertFalse(encode(listOf(l)).contains("token"))
        // Characters, not UTF-16 units (the PC counts chars()).
        assertEquals(3, clipTextEntry("PC", "a😀b").chars)
    }

    @Test fun audio_sessions_keep_who_which_way_and_how_long_never_audio() {
        val e = audioEntry("Lin's PC", TlDir.ToPc, audioTitle(TlDir.ToPc), 252, now)
        assertEquals("Phone audio on this PC, 4 min 12 s", summary(e))
        assertEquals("PC audio on the phone, 12 s", summary(audioEntry("p", TlDir.ToPhone, audioTitle(TlDir.ToPhone), 12, now)))
        assertNull(e.size)
        assertNull(e.uri)
        assertEquals(listOf(e), decode(encode(listOf(e))))
        assertEquals(listOf("0 s", "59 s", "1 min", "4 min 12 s", "1 h", "1 h 3 min"), listOf(0L, 59, 60, 252, 3600, 3780).map(::durationText))
    }

    @Test fun summaries_match_the_pc_wording() {
        assertEquals("Picture, 1080 × 2412, 1.2 MB", summary(pic()))
        assertEquals("Picture, 1.2 MB", summary(clipImageEntry("p", TlDir.ToPc, 1_200_000, null)))
        assertEquals("Picture", summary(TlEntry(TlKind.ClipImage, TlDir.ToPc, "p", now)))
        assertEquals("Text, 142 characters", summary(clipTextEntry("p", "a".repeat(142))))
        assertEquals("Text, 1 character", summary(clipTextEntry("p", "a")))
        assertEquals("Text, 0 characters", summary(clipTextEntry("p", "")))
        assertEquals("Text", summary(TlEntry(TlKind.ClipText, TlDir.ToPc, "p", now)))
        assertEquals("photo.jpg, 3.4 MB", summary(fileEntry("p", TlDir.ToPc, "photo.jpg", 3_400_000, null)))
        assertEquals("File", summary(TlEntry(TlKind.File, TlDir.ToPc, "p", now)))
        assertEquals("WhatsApp notification", summary(notificationEntry("p", "WhatsApp")))
        assertEquals("App notification", summary(TlEntry(TlKind.Notification, TlDir.ToPc, "p", now)))
        assertEquals("Link, 20 characters", summary(linkEntry("p", "https://example.com/")))
        assertEquals("Link, 1 character", summary(linkEntry("p", "x")))
    }

    @Test fun sizes_round_like_the_pc() {
        fun f(n: Long) = summary(fileEntry("p", TlDir.ToPc, "a", n, null)).removePrefix("a, ")
        assertEquals("0 B", f(0))
        assertEquals("999 B", f(999))
        assertEquals("1.0 KB", f(1000))
        // Rust's {:.1} rounds ties to even on the exact binary value.
        assertEquals("1.2 KB", f(1250))
        assertEquals("1.4 KB", f(1350))
        assertEquals("1.1 KB", f(1050))
        assertEquals("1.2 MB", f(1_234_567))
        assertEquals("1000.0 KB", f(999_950))
        assertEquals("2.5 MB", f(2_500_000))
        assertEquals("5.0 GB", f(5_000_000_000))
    }

    @Test fun png_dimensions_come_from_ihdr() {
        val sig = byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A)
        val ihdr = byteArrayOf(0, 0, 0, 13, 0x49, 0x48, 0x44, 0x52, 0, 0, 4, 56, 0, 0, 9, 108)
        assertEquals(1080 to 2412, pngDims(sig + ihdr))
        assertNull(pngDims(sig))
        assertNull(pngDims(ByteArray(24)))
        val zero = sig + byteArrayOf(0, 0, 0, 13, 0x49, 0x48, 0x44, 0x52, 0, 0, 0, 0, 0, 0, 9, 108)
        assertNull(pngDims(zero))
    }

    @Test fun corrupt_json_reads_as_empty() {
        for (bad in listOf("", "not json", "{", "[]", "{\"v\":2,\"e\":[]}", "{\"v\":1,\"e\":[{\"k\":\"Nope\",\"d\":\"c\",\"peer\":\"p\",\"at\":1}]}", "{\"v\":1,\"e\":[{\"k\":\"F\",\"d\":\"ToPc\",\"peer\":\"p\",\"at\":1}]}",
            "{\"v\":1,\"e\":[{\"k\":\"F\",\"d\":\"c\",\"peer\":\"p\"}]}", "{\"v\":1,\"e\":[{\"k\":\"I\",\"d\":\"c\",\"peer\":\"p\",\"at\":1,\"w\":3}]}")) {
            assertEquals(bad, emptyList<TlEntry>(), decode(bad))
            assertNull(bad, decodeOrNull(bad))
        }
        assertTrue(decodeOrNull(encode(emptyList())) != null)
        // The stable on-disk codes, not enum names.
        assertEquals(listOf(TlEntry(TlKind.Link, TlDir.ToPhone, "p", 1, chars = 3)),
            decodeOrNull("{\"v\":1,\"e\":[{\"k\":\"L\",\"d\":\"p\",\"peer\":\"p\",\"at\":1,\"chars\":3}]}"))
    }

    @Test fun home_recent_rows_show_the_summary_not_content() {
        val r = recentItem(fileEntry("Lin's PC", TlDir.ToPhone, "photo.jpg", 3_400_000, "content://x/1"))
        assertEquals(RecentItem("photo.jpg, 3.4 MB", "From Lin's PC", RecentItem.Kind.File, "content://x/1"), r)
        val t = recentItem(clipTextEntry("Lin's PC", "secret words"))
        assertEquals(RecentItem("Text, 12 characters", "Sent to Lin's PC", RecentItem.Kind.Clipboard), t)
        assertEquals(RecentItem.Kind.Link, recentItem(linkEntry("p", "https://a")).kind)
    }

    // --- The store, with a fake file and hand-run I/O queue (deterministic ordering). ---

    private class FakeFile(var bytes: ByteArray? = null) : TlFile {
        var readError: java.io.IOException? = null
        var writeError: Exception? = null
        var writes = 0
        override fun read(): ByteArray = readError?.let { throw it } ?: bytes ?: throw java.io.FileNotFoundException()
        override fun write(b: ByteArray) {
            writes++
            writeError?.let { throw it }
            bytes = b
        }
        override fun delete() { bytes = null }
        val text get() = bytes?.toString(Charsets.UTF_8)
    }

    private class Rig(val file: FakeFile, enabled: Boolean = true) {
        val io = ArrayDeque<Runnable>()
        val later = ArrayDeque<Runnable>()
        val log = mutableListOf<String>()
        val store = TimelineStore(exec = { io.add(it) }, later = { later.add(it) }, log = { log.add(it) }, now = { 1_800_000_000_000L })
        init { store.start(file, enabled) }
        fun runIo() { while (io.isNotEmpty()) io.removeFirst().run() }
        fun runAll() { runIo(); while (later.isNotEmpty()) { later.removeFirst().run(); runIo() } }
    }

    private val onDisk = encode(listOf(fileEntry("PC", TlDir.ToPhone, "old.jpg", 5, null, at = now - 10)))

    @Test fun store_loads_and_writes_newest_first() {
        val r = Rig(FakeFile(onDisk.toByteArray()))
        r.runAll()
        assertEquals(1, r.store.entries.value.size)
        r.store.add(clipTextEntry("PC", "abc", at = now))
        r.runAll()
        assertEquals(listOf(TlKind.ClipText, TlKind.File), decode(r.file.text!!).map { it.kind })
    }

    @Test fun clear_before_the_first_load_does_not_bring_history_back() {
        val r = Rig(FakeFile(onDisk.toByteArray()))
        r.store.clear() // the load is still queued
        r.runAll()
        assertEquals(emptyList<TlEntry>(), r.store.entries.value)
        assertNull(r.file.bytes)
        r.store.add(clipTextEntry("PC", "abc", at = now))
        r.runAll()
        assertEquals(listOf(TlKind.ClipText), decode(r.file.text!!).map { it.kind })
    }

    @Test fun history_off_before_the_first_load_does_not_bring_it_back() {
        val r = Rig(FakeFile(onDisk.toByteArray()))
        r.store.setEnabled(false)
        r.runAll()
        r.store.setEnabled(true)
        r.runAll()
        assertEquals(emptyList<TlEntry>(), r.store.entries.value)
        assertNull(r.file.bytes)
    }

    @Test fun add_while_history_is_off_never_reaches_disk() {
        val r = Rig(FakeFile())
        r.runAll()
        r.store.setEnabled(false)
        r.store.add(clipTextEntry("PC", "abc", at = now))
        assertEquals(emptyList<TlEntry>(), r.store.entries.value)
        r.store.setEnabled(true)
        r.store.add(notificationEntry("PC", "Mail", at = now))
        r.runAll()
        assertEquals(listOf(TlKind.Notification), decode(r.file.text!!).map { it.kind })
    }

    @Test fun any_write_failure_is_caught_logged_without_content_and_retried() {
        val r = Rig(FakeFile())
        r.runAll()
        r.file.writeError = IllegalStateException("secret words")
        r.store.add(clipTextEntry("PC", "abc", at = now))
        r.runAll() // must not throw
        assertEquals(1, r.file.writes)
        assertTrue(r.log.any { it.startsWith("TIMELINE write failed") })
        assertFalse(r.log.any { it.contains("secret") })
        // Not stuck: the next add schedules another write, which succeeds.
        r.file.writeError = null
        r.store.add(clipTextEntry("PC", "abcd", at = now))
        assertEquals(1, r.later.size)
        r.runAll()
        assertEquals(2, decode(r.file.text!!).size)
    }

    @Test fun flush_now_writes_a_queued_write_at_once_and_only_once() {
        val r = Rig(FakeFile())
        r.runAll()
        r.store.flushNow()
        assertEquals(0, r.file.writes)
        r.store.add(clipTextEntry("PC", "abc", at = now))
        r.store.flushNow()
        assertEquals(1, r.file.writes)
        r.runAll() // the delayed write finds nothing left to do
        assertEquals(1, r.file.writes)
    }

    @Test fun unknown_kind_on_disk_is_replaced_without_crashing() {
        val r = Rig(FakeFile("{\"v\":1,\"e\":[{\"k\":\"Z\",\"d\":\"c\",\"peer\":\"p\",\"at\":1}]}".toByteArray()))
        r.runAll()
        assertEquals(emptyList<TlEntry>(), r.store.entries.value)
        assertNull("replaced, as on the PC", r.file.bytes)
        assertTrue(r.log.any { it.startsWith("TIMELINE unreadable") })
        r.store.add(clipTextEntry("PC", "abc", at = now))
        r.runAll()
        assertEquals(1, decode(r.file.text!!).size)
    }

    @Test fun a_read_error_keeps_the_file_for_the_session() {
        val f = FakeFile(onDisk.toByteArray()).apply { readError = java.io.IOException("busy") }
        val r = Rig(f)
        r.runAll()
        r.store.add(clipTextEntry("PC", "abc", at = now))
        r.runAll()
        assertEquals(1, r.store.entries.value.size)
        assertEquals(onDisk, f.text)
        assertEquals(0, f.writes)
    }
}
