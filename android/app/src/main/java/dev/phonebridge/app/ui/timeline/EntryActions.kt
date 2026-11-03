package dev.phonebridge.app.ui.timeline

import dev.phonebridge.app.APP_NAME
import dev.phonebridge.app.TlDir
import dev.phonebridge.app.TlEntry
import dev.phonebridge.app.TlKind
import dev.phonebridge.app.humanSize
import java.time.Instant
import java.time.ZoneId

/** Start of every "nothing to open" line for content the timeline deliberately does not store. */
val APP_KEEPS = "$APP_NAME keeps only"

/** What a timeline row can do. The timeline stores metadata only, so text, links and pictures cannot be copied or reopened. */
enum class EntryAction { Open, ShareAgain, CopyName, Details, Delete }

fun entryActions(e: TlEntry): List<EntryAction> = buildList {
    if (isOpenable(e)) {
        add(EntryAction.Open)
        add(EntryAction.ShareAgain)
    }
    if (e.kind == TlKind.File && !e.name.isNullOrEmpty()) add(EntryAction.CopyName)
    add(EntryAction.Details)
    add(EntryAction.Delete)
}

/** The message for tapping a row that cannot open, or null when a tap opens it. */
fun tapHint(e: TlEntry): String? = when {
    isOpenable(e) -> null
    e.kind == TlKind.File ->
        if (e.dir == TlDir.ToPc) "Sent files are not kept here, so they cannot be reopened."
        else "This file cannot be reopened from here. Find it in your Files app."
    e.kind == TlKind.ClipText -> "$APP_KEEPS the length of copied text, never the text itself."
    e.kind == TlKind.Link -> "$APP_KEEPS the length of a link, never the address."
    e.kind == TlKind.ClipImage -> "$APP_KEEPS the size of a picture, never the picture."
    else -> "$APP_KEEPS the app name of a notification, never its content."
}

private fun kindName(k: TlKind) = when (k) {
    TlKind.File -> "File"
    TlKind.ClipText -> "Copied text"
    TlKind.ClipImage -> "Picture"
    TlKind.Link -> "Link"
    TlKind.Notification -> "Notification"
}

/** The stored facts of an entry, label then value, for the Details dialog. Only what the timeline keeps. */
fun detailLines(e: TlEntry, zone: ZoneId): List<Pair<String, String>> = buildList {
    add("Type" to if (e.kind == TlKind.File) "File" else kindName(e.kind))
    add("Direction" to if (e.dir == TlDir.ToPc) "Sent to ${e.peer}" else "From ${e.peer}")
    val t = Instant.ofEpochMilli(e.at).atZone(zone)
    add("When" to "%04d-%02d-%02d %02d:%02d".format(t.year, t.monthValue, t.dayOfMonth, t.hour, t.minute))
    e.name?.let { add("Name" to it) }
    e.app?.let { add("App" to it) }
    e.size?.let { add("Size" to humanSize(it)) }
    if (e.width != null && e.height != null) add("Dimensions" to "${e.width} \u00d7 ${e.height}")
    e.chars?.let { add("Characters" to it.toString()) }
}

/** A calendar-day key for remembering which days the user opened or closed; a new day is a new key, whatever its label. */
fun dayKey(at: Long, zone: ZoneId): String = Instant.ofEpochMilli(at).atZone(zone).toLocalDate().toEpochDay().toString()
