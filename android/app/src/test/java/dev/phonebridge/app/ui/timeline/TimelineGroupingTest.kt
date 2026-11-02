package dev.phonebridge.app.ui.timeline

import dev.phonebridge.app.TlDir
import dev.phonebridge.app.TlEntry
import dev.phonebridge.app.TlKind
import java.time.Instant
import java.time.ZoneId
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class TimelineGroupingTest {
    private fun ms(iso: String) = Instant.parse(iso).toEpochMilli()
    private fun e(at: Long) = TlEntry(TlKind.ClipText, TlDir.ToPc, "p", at, chars = 1)
    private fun labels(g: List<Pair<String, List<TlEntry>>>) = g.map { it.first }

    @Test fun groups_today_yesterday_and_dates() {
        val zone = ZoneId.of("Asia/Kolkata") // +05:30
        val now = ms("2026-11-02T10:00:00Z") // 15:30 on Mon 2 Nov
        val list = listOf(
            e(ms("2026-11-02T09:00:00Z")),
            e(ms("2026-11-01T20:00:00Z")), // 01:30 on 2 Nov local: still Today
            e(ms("2026-11-01T17:00:00Z")), // 22:30 on 1 Nov: Yesterday
            e(ms("2026-10-27T05:00:00Z")),
            e(ms("2026-10-27T04:00:00Z")),
        )
        val g = groupByDay(list, now, zone)
        assertEquals(listOf("Today", "Yesterday", "Tue 27 Oct"), labels(g))
        assertEquals(listOf(2, 1, 2), g.map { it.second.size })
        // Order inside a group is the input order.
        assertEquals(list.take(2), g[0].second)
    }

    @Test fun late_evening_utc_is_next_local_day() {
        val now = ms("2026-11-02T12:00:00Z")
        val inUtc = groupByDay(listOf(e(ms("2026-11-01T23:30:00Z"))), now, ZoneId.of("UTC"))
        assertEquals(listOf("Yesterday"), labels(inUtc))
        val inTokyo = groupByDay(listOf(e(ms("2026-11-01T23:30:00Z"))), now, ZoneId.of("Asia/Tokyo")) // 08:30 on 2 Nov
        assertEquals(listOf("Today"), labels(inTokyo))
        val inLa = groupByDay(listOf(e(ms("2026-11-02T03:00:00Z"))), now, ZoneId.of("America/Los_Angeles")) // 20:00 on 1 Nov
        assertEquals(listOf("Yesterday"), labels(inLa))
    }

    @Test fun older_dates_use_short_english_names_and_single_digit_days() {
        val now = ms("2026-11-02T12:00:00Z")
        val g = groupByDay(listOf(e(ms("2026-10-02T12:00:00Z")), e(ms("2026-01-04T12:00:00Z"))), now, ZoneId.of("UTC"))
        assertEquals(listOf("Fri 2 Oct", "Sun 4 Jan"), labels(g))
    }

    @Test fun empty_input_and_a_repeating_day_do_not_crash() {
        val now = ms("2026-11-02T12:00:00Z")
        assertTrue(groupByDay(emptyList(), now, ZoneId.of("UTC")).isEmpty())
        // Out of order (clock went backwards): a duplicate header, no crash.
        val g = groupByDay(
            listOf(e(ms("2026-11-02T01:00:00Z")), e(ms("2026-11-01T01:00:00Z")), e(ms("2026-11-02T02:00:00Z"))),
            now, ZoneId.of("UTC"),
        )
        assertEquals(listOf("Today", "Yesterday", "Today"), labels(g))
    }

    @Test fun clock_is_hh_mm_in_the_zone() {
        assertEquals("15:30", clockText(ms("2026-11-02T10:00:00Z"), ZoneId.of("Asia/Kolkata")))
        assertEquals("05:07", clockText(ms("2026-11-02T05:07:00Z"), ZoneId.of("UTC")))
    }

    @Test fun empty_history_shows_empty_state_text() {
        assertEquals("History is off. Turn it on to see what you send and receive.", emptyText(false))
        assertEquals("Nothing yet. Files and clipboard you send or receive show up here.", emptyText(true))
    }
}
