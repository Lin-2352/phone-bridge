package dev.phonebridge.app.ui.timeline

import dev.phonebridge.app.TlEntry
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId

private val WEEK = arrayOf("Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun")
private val MONTH = arrayOf("Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec")

/** "Today", "Yesterday", else "Mon 27 Oct" (English names, whatever the device language). */
private fun dayLabel(day: LocalDate, today: LocalDate): String = when (today.toEpochDay() - day.toEpochDay()) {
    0L -> "Today"
    1L -> "Yesterday"
    else -> "${WEEK[day.dayOfWeek.value - 1]} ${day.dayOfMonth} ${MONTH[day.monthValue - 1]}"
}

private fun localDay(at: Long, zone: ZoneId) = Instant.ofEpochMilli(at).atZone(zone).toLocalDate()

/**
 * Runs of one local day, in the given order, labelled as on the PC. The
 * input is newest first, so entries of one day are adjacent and make one
 * group; if the clock went backwards and a day repeats, it simply gets a
 * second header.
 */
fun groupByDay(entries: List<TlEntry>, now: Long, zone: ZoneId): List<Pair<String, List<TlEntry>>> {
    val today = localDay(now, zone)
    val out = ArrayList<Triple<LocalDate, String, MutableList<TlEntry>>>()
    for (e in entries) {
        val day = localDay(e.at, zone)
        val last = out.lastOrNull()
        if (last != null && last.first == day) last.third.add(e) else out.add(Triple(day, dayLabel(day, today), mutableListOf(e)))
    }
    return out.map { it.second to it.third }
}

/** Local "14:05". */
fun clockText(at: Long, zone: ZoneId): String {
    val t = Instant.ofEpochMilli(at).atZone(zone)
    return "%02d:%02d".format(t.hour, t.minute)
}

/** What the tab says when there is nothing to list. */
fun emptyText(historyOn: Boolean): String =
    if (historyOn) "Nothing yet. Files and clipboard you send or receive show up here."
    else "History is off. Turn it on to see what you send and receive."
