package dev.phonebridge.app

import android.content.Context
import android.net.Uri

/** User settings. Share clipboard is off until the user turns it on. */
class Prefs(context: Context) {
    private val sp = context.getSharedPreferences("settings", Context.MODE_PRIVATE)

    /** Phone Bridge on (service running, reachable) or off. The tile's state. */
    var enabled: Boolean
        get() = sp.getBoolean("enabled", true)
        set(v) = sp.edit().putBoolean("enabled", v).apply()

    var clipboardSync: Boolean
        get() = sp.getBoolean("clipboard_sync", false)
        set(v) = sp.edit().putBoolean("clipboard_sync", v).apply()

    /** Also sync text an app marked sensitive (passwords, codes). Off by default. */
    var sendSensitive: Boolean
        get() = sp.getBoolean("send_sensitive", false)
        set(v) = sp.edit().putBoolean("send_sensitive", v).apply()

    /** Fingerprint of the last clipboard text synced either way, so focus
     *  changes never re-send a clipboard that has not changed (survives restarts). */
    var lastClip: Int
        get() = sp.getInt("last_clip", 0)
        set(v) = sp.edit().putInt("last_clip", v).apply()

    /** PC control starts without asking. Off: each request asks on the phone. */
    var allowControl: Boolean
        get() = sp.getBoolean("allow_control", false)
        set(v) = sp.edit().putBoolean("allow_control", v).apply()

    /** "Phone Bridge stopped" was shown and not yet resolved (shown once, not every 15 min). */
    var stoppedNotified: Boolean
        get() = sp.getBoolean("stopped_notified", false)
        set(v) = sp.edit().putBoolean("stopped_notified", v).apply()

    /** Newest process-exit timestamp already looked at (user stops are detected once). */
    var lastExitSeen: Long
        get() = sp.getLong("last_exit_seen", 0L)
        set(v) = sp.edit().putLong("last_exit_seen", v).apply()

    var autoAccept: Boolean
        get() = sp.getBoolean("auto_accept", true)
        set(v) = sp.edit().putBoolean("auto_accept", v).apply()

    /** 0 = follow system, 1 = light, 2 = dark. */
    var theme: Int
        get() = sp.getInt("theme", 0)
        set(v) = sp.edit().putInt("theme", v).apply()

    /** Folder the user picked for received files (SAF tree), or null for Downloads/Phone Bridge. */
    var saveTree: Uri?
        get() = sp.getString("save_tree", null)?.let(Uri::parse)
        set(v) = sp.edit().putString("save_tree", v?.toString()).apply()
}

/** "primary:Documents/Phone" -> "Documents/Phone" for display. */
fun treeLabel(tree: Uri?): String {
    tree ?: return "Downloads/Phone Bridge"
    val id = android.provider.DocumentsContract.getTreeDocumentId(tree)
    return id.substringAfter(':').ifEmpty { "Internal storage" }
}
