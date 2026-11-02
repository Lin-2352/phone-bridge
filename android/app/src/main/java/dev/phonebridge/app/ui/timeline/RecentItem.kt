package dev.phonebridge.app.ui.timeline

import dev.phonebridge.app.TlDir
import dev.phonebridge.app.TlEntry
import dev.phonebridge.app.TlKind
import dev.phonebridge.app.summary

/** One line in a Recent list (Home's short one, later the Timeline tab). */
data class RecentItem(val title: String, val subtitle: String, val kind: Kind, val uri: String? = null) {
    enum class Kind { File, Clipboard, Link }
}

/** A timeline entry in the Recent list's shape: metadata only, as stored. */
fun recentItem(e: TlEntry) = RecentItem(
    title = summary(e),
    subtitle = if (e.dir == TlDir.ToPc) "Sent to ${e.peer}" else "From ${e.peer}",
    kind = when (e.kind) {
        TlKind.File -> RecentItem.Kind.File
        TlKind.Link -> RecentItem.Kind.Link
        else -> RecentItem.Kind.Clipboard
    },
    uri = e.uri,
)
