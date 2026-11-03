package dev.phonebridge.app.ui.timeline

import dev.phonebridge.app.TlDir
import dev.phonebridge.app.clipImageEntry
import dev.phonebridge.app.clipTextEntry
import dev.phonebridge.app.fileEntry
import dev.phonebridge.app.linkEntry
import dev.phonebridge.app.notificationEntry
import java.time.ZoneId
import java.time.ZoneOffset
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class EntryActionsTest {
    private val at = 1_800_000_000_000L
    private val received = fileEntry("PC", TlDir.ToPhone, "a.pdf", 10, "content://x/1", at)
    private val oldReceived = fileEntry("PC", TlDir.ToPhone, "b.pdf", 10, "file:///x/b.pdf", at)
    private val sent = fileEntry("PC", TlDir.ToPc, "c.pdf", 10, null, at)

    @Test fun a_received_file_can_be_opened_shared_again_copied_detailed_and_deleted() {
        assertEquals(
            listOf(EntryAction.Open, EntryAction.ShareAgain, EntryAction.CopyName, EntryAction.Details, EntryAction.Delete),
            entryActions(received),
        )
        assertNull(tapHint(received))
    }

    @Test fun a_sent_file_keeps_only_name_details_and_delete() {
        assertEquals(listOf(EntryAction.CopyName, EntryAction.Details, EntryAction.Delete), entryActions(sent))
        assertEquals("Sent files are not kept here, so they cannot be reopened.", tapHint(sent))
    }

    @Test fun a_file_with_an_old_style_uri_says_why() {
        assertEquals(listOf(EntryAction.CopyName, EntryAction.Details, EntryAction.Delete), entryActions(oldReceived))
        assertTrue(tapHint(oldReceived)!!.contains("cannot be reopened"))
    }

    @Test fun text_links_pictures_and_notifications_keep_no_content_so_only_details_and_delete() {
        val kept = listOf(
            clipTextEntry("PC", "hello", at = at) to "text",
            linkEntry("PC", "https://x.y", at = at) to "link",
            clipImageEntry("PC", TlDir.ToPc, 100, 2 to 2, at) to "picture",
            notificationEntry("PC", "Mail", at) to "notification",
        )
        for ((e, what) in kept) {
            assertEquals(listOf(EntryAction.Details, EntryAction.Delete), entryActions(e))
            assertTrue("$what: ${tapHint(e)}", tapHint(e)!!.startsWith("$APP_KEEPS"))
        }
        assertTrue(tapHint(kept[0].first)!!.contains("text"))
        assertTrue(tapHint(kept[1].first)!!.contains("link"))
    }

    @Test fun details_list_what_is_stored_and_nothing_else() {
        val zone = ZoneOffset.UTC
        val d = detailLines(received, zone).toMap()
        assertEquals("File", d["Type"])
        assertEquals("From PC", d["Direction"])
        assertEquals("a.pdf", d["Name"])
        assertEquals("10 B", d["Size"])
        assertTrue(d.containsKey("When"))
        val t = detailLines(clipTextEntry("PC", "hello", at = at), zone).toMap()
        assertEquals("5", t["Characters"])
        assertEquals("Sent to PC", t["Direction"])
        assertTrue(t.keys.none { it == "Name" || it == "Text" })
    }

    @Test fun a_day_key_changes_at_midnight_not_with_the_label() {
        val zone = ZoneId.of("UTC")
        val d1 = 1_700_000_000_000L // some day
        val sameDay = d1 + 60_000
        val nextDay = d1 + 24 * 3600_000L
        assertEquals(dayKey(d1, zone), dayKey(sameDay, zone))
        assertTrue(dayKey(d1, zone) != dayKey(nextDay, zone))
    }
}
