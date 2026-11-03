package dev.phonebridge.app

import android.content.Context
import android.net.Uri

/** User settings. Share clipboard is off until the user turns it on. */
private const val NL = "\n"

class Prefs(context: Context) {
    private val sp = context.getSharedPreferences("settings", Context.MODE_PRIVATE)

    /** Keep the day-by-day log (metadata only). On by default. */
    var keepLogs: Boolean
        get() = sp.getBoolean("keep_logs", true)
        set(v) = sp.edit().putBoolean("keep_logs", v).apply()

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

    /** Apply pictures copied on the PC to this phone's clipboard. Off by default. */
    var clipboardImages: Boolean
        get() = sp.getBoolean("clipboard_images", false)
        set(v) = sp.edit().putBoolean("clipboard_images", v).apply()

    /** Send each new screenshot to the PC's clipboard. Off by default; needs picture access. */
    var shareScreenshots: Boolean
        get() = sp.getBoolean("share_screenshots", false)
        set(v) = sp.edit().putBoolean("share_screenshots", v).apply()

    /** Fingerprint of the last clipboard text synced either way, so focus
     *  changes never re-send a clipboard that has not changed (survives restarts). */
    var lastClip: Int
        get() = sp.getInt("last_clip", 0)
        set(v) = sp.edit().putInt("last_clip", v).apply()

    /**
     * PCs (by peer id) whose control requests start without asking. "Always allow" in the question adds
     * only the PC that asked (as the camera list does). On = any PC in the list; setting it on from
     * Settings allows the PCs paired now, off clears the list. A value saved by an older version
     * ("allow_control": every PC) stays pending until [migrateAllowances] turns it into the paired PCs.
     */
    var allowedControlPcs: Set<String>
        get() = sp.getStringSet("allow_control_pcs", emptySet()).orEmpty().toSet()
        set(v) = sp.edit().putStringSet("allow_control_pcs", v).apply()

    var allowControl: Boolean
        get() = allowedControlPcs.isNotEmpty() || sp.getBoolean("allow_control", false)
        set(v) = setAllowAll("allow_control", "allow_control_pcs", v)

    private fun setAllowAll(legacyKey: String, setKey: String, on: Boolean) {
        val ids = Bridge.node?.peers()?.map { it.id }.orEmpty()
        val cur = sp.getStringSet(setKey, emptySet()).orEmpty()
        val e = sp.edit()
        when {
            !on -> { e.remove(legacyKey); e.putStringSet(setKey, emptySet()) }
            ids.isEmpty() -> e.putBoolean(legacyKey, true) // node off: migrated to the paired PCs at the next start
            else -> { e.remove(legacyKey); e.putStringSet(setKey, migrateGlobalAllow(true, cur, ids)) }
        }
        e.apply()
    }

    /** Node up, [pairedIds] known: old global "always allow" values become per-PC; removed PCs are dropped. */
    fun migrateAllowances(pairedIds: List<String>) {
        if (pairedIds.isEmpty()) return
        for ((legacy, key) in listOf("allow_control" to "allow_control_pcs", "allow_screen" to "allow_screen_pcs")) {
            val cur = sp.getStringSet(key, emptySet()).orEmpty()
            val next = pruneAllowed(migrateGlobalAllow(sp.getBoolean(legacy, false), cur, pairedIds), pairedIds)
            if (sp.contains(legacy) || next != cur) sp.edit().remove(legacy).putStringSet(key, next).apply()
        }
    }

    /** "Phone Bridge stopped" was shown and not yet resolved (shown once, not every 15 min). */
    var stoppedNotified: Boolean
        get() = sp.getBoolean("stopped_notified", false)
        set(v) = sp.edit().putBoolean("stopped_notified", v).apply()

    /** Newest process-exit timestamp already looked at (user stops are detected once). */
    var lastExitSeen: Long
        get() = sp.getLong("last_exit_seen", 0L)
        set(v) = sp.edit().putLong("last_exit_seen", v).apply()

    /** Phone audio to the PC: skip Relay's own confirmation (never Android's capture dialog, never for the microphone). Off by default. */
    var audioSkipConfirmPhoneToPc: Boolean
        get() = sp.getBoolean("audio_skip_confirm_phone_to_pc", false)
        set(v) = sp.edit().putBoolean("audio_skip_confirm_phone_to_pc", v).apply()

    /** PC audio on this phone: skip the PC's tray confirmation (the PC asks; kept here so Settings has one place). Off by default. */
    var audioSkipConfirmPcToPhone: Boolean
        get() = sp.getBoolean("audio_skip_confirm_pc_to_phone", false)
        set(v) = sp.edit().putBoolean("audio_skip_confirm_pc_to_phone", v).apply()

    var autoAccept: Boolean
        get() = sp.getBoolean("auto_accept", true)
        set(v) = sp.edit().putBoolean("auto_accept", v).apply()

    /** 0 = follow system, 1 = light, 2 = dark. */
    var theme: Int
        get() = sp.getInt("theme", 0)
        set(v) = sp.edit().putInt("theme", v).apply()

    /** Apps the user does not want mirrored to the PC (package names). */
    var notifyBlocked: Set<String>
        get() = sp.getStringSet("notify_blocked", emptySet()).orEmpty().toSet()
        set(v) = sp.edit().putStringSet("notify_blocked", v).apply()

    /** Apps the user allows although they are blocked by default (banking, authenticators, secret notifications). */
    var notifyAllowed: Set<String>
        get() = sp.getStringSet("notify_allowed", emptySet()).orEmpty().toSet()
        set(v) = sp.edit().putStringSet("notify_allowed", v).apply()

    /** Mirrored vs blocked notifications so far (Settings shows them). */
    var mirrorCounts: MirrorCounts
        get() = MirrorCounts(sp.getInt("mirror_sent", 0), sp.getInt("mirror_blocked", 0))
        set(v) = sp.edit().putInt("mirror_sent", v.sent).putInt("mirror_blocked", v.blocked).apply()

    /** Recently seen notifying apps, newest first, for the block list. */
    var notifySeen: List<String>
        get() = sp.getString("notify_seen", "").orEmpty().split(NL).filter { it.isNotEmpty() }
        set(v) = sp.edit().putString("notify_seen", v.joinToString(NL)).apply()

    /** Phone notifications are shown on paired PCs (once access is granted). */
    var notifyToPc: Boolean
        get() = sp.getBoolean("notify_to_pc", true)
        set(v) = sp.edit().putBoolean("notify_to_pc", v).apply()

    /** Ringer mode before a PC silenced a call, or -1: restored after the call. */
    var savedRinger: Int
        get() = sp.getInt("saved_ringer", -1)
        set(v) = sp.edit().putInt("saved_ringer", v).apply()

    /** PCs (by peer id) whose screen requests start without asking (Android's prompt still shows); like [allowedControlPcs]. */
    var allowedScreenPcs: Set<String>
        get() = sp.getStringSet("allow_screen_pcs", emptySet()).orEmpty().toSet()
        set(v) = sp.edit().putStringSet("allow_screen_pcs", v).apply()

    var allowScreen: Boolean
        get() = allowedScreenPcs.isNotEmpty() || sp.getBoolean("allow_screen", false)
        set(v) = setAllowAll("allow_screen", "allow_screen_pcs", v)

    /**
     * PCs (by peer id) that may use the camera without asking; Android's
     * camera permission still applies. The old global switch ("allow_camera")
     * is dropped, not turned into "every PC": nobody is allowed by it.
     */
    var allowedCameraPcs: Set<String>
        get() {
            if (sp.contains("allow_camera")) sp.edit().remove("allow_camera").apply()
            return sp.getStringSet("allow_camera_pcs", emptySet()).orEmpty().toSet()
        }
        set(v) = sp.edit().putStringSet("allow_camera_pcs", v).apply()

    /** Keep screen capture ready between shares (Android shows its cast icon). */
    var instantScreen: Boolean
        get() = sp.getBoolean("instant_screen", false)
        set(v) = sp.edit().putBoolean("instant_screen", v).apply()

    /** "Keep history": record the Timeline (metadata only). Off deletes it. On by default. */
    var keepHistory: Boolean
        get() = sp.getBoolean("keep_history", true)
        set(v) = sp.edit().putBoolean("keep_history", v).apply()

    /** When the paired PC was last in contact (wall-clock ms), 0 if never: Home's "Last seen". */
    var lastSeenMs: Long
        get() = sp.getLong("last_seen_ms", 0L)
        set(v) = sp.edit().putLong("last_seen_ms", v).apply()

    /** Name of the last paired PC, so Home can show it (dimmed) while Relay is off. Cleared once the node says nothing is paired. */
    var lastPcName: String?
        get() = sp.getString("last_pc_name", null)
        set(v) = sp.edit().putString("last_pc_name", v).apply()

    /** Folder the user picked for received files (SAF tree), or null for Downloads/Phone Bridge. */
    var saveTree: Uri?
        get() = sp.getString("save_tree", null)?.let(Uri::parse)
        set(v) = sp.edit().putString("save_tree", v?.toString()).apply()
}

/** "primary:Documents/Phone" -> "Documents/Phone" for display. */
fun treeLabel(tree: Uri?): String {
    tree ?: return "Downloads/$APP_NAME"
    val id = android.provider.DocumentsContract.getTreeDocumentId(tree)
    return id.substringAfter(':').ifEmpty { "Internal storage" }
}
