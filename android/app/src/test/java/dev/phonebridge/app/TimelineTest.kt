package dev.phonebridge.app

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
        for (bad in listOf("", "not json", "{", "[]", "{\"v\":2,\"e\":[]}", "{\"v\":1,\"e\":[{\"k\":\"Nope\",\"d\":\"ToPc\",\"peer\":\"p\",\"at\":1}]}",
            "{\"v\":1,\"e\":[{\"k\":\"File\",\"d\":\"ToPc\",\"peer\":\"p\"}]}", "{\"v\":1,\"e\":[{\"k\":\"ClipImage\",\"d\":\"ToPc\",\"peer\":\"p\",\"at\":1,\"w\":3}]}")) {
            assertEquals(bad, emptyList<TlEntry>(), decode(bad))
            assertNull(bad, decodeOrNull(bad))
        }
        assertTrue(decodeOrNull(encode(emptyList())) != null)
    }

    @Test fun home_recent_rows_show_the_summary_not_content() {
        val r = recentItem(fileEntry("Lin's PC", TlDir.ToPhone, "photo.jpg", 3_400_000, "content://x/1"))
        assertEquals(RecentItem("photo.jpg, 3.4 MB", "From Lin's PC", RecentItem.Kind.File, "content://x/1"), r)
        val t = recentItem(clipTextEntry("Lin's PC", "secret words"))
        assertEquals(RecentItem("Text, 12 characters", "Sent to Lin's PC", RecentItem.Kind.Clipboard), t)
        assertEquals(RecentItem.Kind.Link, recentItem(linkEntry("p", "https://a")).kind)
    }
}
