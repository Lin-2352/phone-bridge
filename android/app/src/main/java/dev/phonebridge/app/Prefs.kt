package dev.phonebridge.app

import android.content.Context
import android.net.Uri

/** User settings. Share clipboard is off until the user turns it on. */
private const val NL = "\n"

class Prefs(context: Context) {
    private val sp = context.getSharedPreferences("settings", Context.MODE_PRIVATE)
    /** The app's own debuggable flag (the build config is not generated in this project). */
    private val debugBuild = context.applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE != 0

    init {
        migrateOnce()
    }

    /** Once per process, before anything reads a setting: older stored settings are brought to [PREFS_VERSION]. */
    private fun migrateOnce() {
        if (migrated) return
        synchronized(Prefs::class.java) {
            if (migrated) return
            val before = sp.all
            val from = prefsVersion(before)
            val (to, after) = migratePrefs(from, before)
            val edit = prefsDiff(before, after + (PREFS_VERSION_KEY to to))
            if (edit.put.isNotEmpty() || edit.remove.isNotEmpty()) {
                val e = sp.edit()
                edit.remove.forEach { e.remove(it) }
                for ((k, v) in edit.put) {
                    when (v) {
                        is Boolean -> e.putBoolean(k, v)
                        is Int -> e.putInt(k, v)
                        is Long -> e.putLong(k, v)
                        is Float -> e.putFloat(k, v)
                        is String -> e.putString(k, v)
                        is Set<*> -> e.putStringSet(k, v.filterIsInstance<String>().toSet())
                    }
                }
                e.commit() // all of it at once, with the version
            }
            migrated = true
        }
    }

    private companion object {
        @Volatile var migrated = false
    }

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

    /** Keep "Relay control" (accessibility) on after a PC session instead of switching it off. */
    var keepControlOn: Boolean
        get() = sp.getBoolean("keep_control_on", false)
        set(v) = sp.edit().putBoolean("keep_control_on", v).apply()

    /** Keyed fingerprint of the last clipboard text synced either way, so focus changes never
     *  re-send a clipboard that has not changed (survives restarts). See [ClipFingerprints]. */
    var lastClipMac: String?
        get() = sp.getString("last_clip_mac", null)
        set(v) = sp.edit().putString("last_clip_mac", v).apply()

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

    /** A PC was removed: what it was allowed goes with it, so pairing it again asks again. */
    fun dropAllowancesExcept(pairedIds: List<String>) {
        allowedCameraPcs = pruneAllowed(allowedCameraPcs, pairedIds)
        allowedControlPcs = pruneAllowed(allowedControlPcs, pairedIds)
        allowedScreenPcs = pruneAllowed(allowedScreenPcs, pairedIds)
        sp.edit().remove("allow_control").remove("allow_screen").apply()
    }

    /** The Keystore can no longer open this phone's identity (M3.7, B5): Relay stays off until it is reset. */
    var identityLost: Boolean
        get() = sp.getBoolean("identity_lost", false)
        set(v) = sp.edit().putBoolean("identity_lost", v).apply()

    /** "Phone Bridge stopped" was shown and not yet resolved (shown once, not every 15 min). */
    var stoppedNotified: Boolean
        get() = sp.getBoolean("stopped_notified", false)
        set(v) = sp.edit().putBoolean("stopped_notified", v).apply()

    /** Newest process-exit timestamp already looked at (user stops are detected once). */
    var lastExitSeen: Long
        get() = sp.getLong("last_exit_seen", 0L)
        set(v) = sp.edit().putLong("last_exit_seen", v).apply()

    /** Phone audio to the PC: skip Relay's own confirmation (never Android's capture dialog, never for the microphone). Off by default. */
    /** Mute the media stream while this phone's audio goes to a PC. */
    var audioSilencePhone: Boolean
        get() = sp.getBoolean("audio_silence_phone", false)
        set(v) = sp.edit().putBoolean("audio_silence_phone", v).apply()

    /** Whether "silence this phone" also covers earphones, wired or wireless. */
    /** Volume keys set the PC's level in every app during a silenced share, through Relay control (opt-in). */
    var audioKeysViaControl: Boolean
        get() = sp.getBoolean("audio_keys_via_control", false)
        set(v) = sp.edit().putBoolean("audio_keys_via_control", v).apply()

    var audioSilenceEarphones: Boolean
        get() = sp.getBoolean("audio_silence_earphones", true)
        set(v) = sp.edit().putBoolean("audio_silence_earphones", v).apply()

    /** Whether [audioHeldIndex] is the volume of the earphones rather than the speaker. */
    var audioHeldOnEarphones: Boolean
        get() = sp.getBoolean("audio_held_on_earphones", false)
        set(v) = sp.edit().putBoolean("audio_held_on_earphones", v).commit().let {}

    /** Muting was tried here and silenced the capture too. */
    var audioSilenceUnsupported: Boolean
        get() = sp.getBoolean("audio_silence_unsupported", false)
        set(v) = sp.edit().putBoolean("audio_silence_unsupported", v).apply()

    /** The PC's level the next silenced share starts at. */
    var audioPcLevel: Int
        get() = sp.getInt("audio_pc_level", dev.phonebridge.app.audio.PC_LEVEL_STEPS)
        set(v) = sp.edit().putInt("audio_pc_level", v).apply()

    /** The volume to give back when the share ends, -1 when none is held (survives a crash). */
    /** A held volume could not be restored (Android 17 refuses outside a service): do it at the next start or open. */
    var pendingVolumeRestore: Boolean
        get() = sp.getBoolean("pending_volume_restore", false)
        set(v) = sp.edit().putBoolean("pending_volume_restore", v).commit().let {}

    var audioHeldIndex: Int
        get() = sp.getInt("audio_held_index", -1)
        set(v) = sp.edit().putInt("audio_held_index", v).commit().let {}

    /** The one-time pairing is done: Relay may start the helper by itself. */
    var audioHelperPaired: Boolean
        get() = sp.getBoolean("audio_helper_paired", false)
        set(v) = sp.edit().putBoolean("audio_helper_paired", v).commit().let {}

    var audioHelperLastAttempt: Long
        get() = sp.getLong("audio_helper_last_attempt", 0)
        set(v) = sp.edit().putLong("audio_helper_last_attempt", v).apply()

    /** Set by earlier versions only: the phone's own volume, for a crash they left behind to be put right. -1 when there is none. */
    var audioSilencedFrom: Int
        get() = sp.getInt("audio_silenced_from", -1)
        set(v) = sp.edit().putInt("audio_silenced_from", v).commit().let {}

    /** Set by earlier versions only: where the volume bar, which set the PC's level then, was left. -1 once taken over. */
    var audioShareIndex: Int
        get() = sp.getInt("audio_share_index", -1)
        set(v) = sp.edit().putInt("audio_share_index", v).apply()

    /** What the last setup or start did, kept because the phone's log only lasts minutes. Never holds the secret. */
    var audioHelperLastResult: String
        get() = sp.getString("audio_helper_last_result", "").orEmpty()
        set(v) = sp.edit().putString("audio_helper_last_result", v).apply()

    var audioHelperLastConnect: Long
        get() = sp.getLong("audio_helper_last_connect", 0)
        set(v) = sp.edit().putLong("audio_helper_last_connect", v).apply()

    /** The audio helper's shared secret, as hex (debug builds only; see HelperSecret.kt). */
    var audioHelperSecretHex: String?
        get() = sp.getString("audio_helper_secret", null)
        set(v) = sp.edit().putString("audio_helper_secret", v).commit().let {}

    /** Every stored key, for the one-time helper cleanup. */
    val allKeys: Set<String> get() = sp.all.keys

    /** Removes [keys] from the settings file. */
    fun removeKeys(keys: Collection<String>) {
        sp.edit().apply { keys.forEach { remove(it) } }.commit()
    }

    /** The helper's leftovers were removed from a build without the helper. */
    var helperCleaned: Boolean
        get() = sp.getBoolean("helper_cleaned", false)
        set(v) = sp.edit().putBoolean("helper_cleaned", v).apply()

    /** Sound was captured while muted on this phone, so muting never needs checking again. */
    var audioMuteKeepsCapture: Boolean
        get() = sp.getBoolean("audio_mute_keeps_capture", false)
        set(v) = sp.edit().putBoolean("audio_mute_keeps_capture", v).apply()

    /** Set by earlier versions only: muted on top of the effect and not unmuted yet, for a crash they left behind. */
    var audioBypassMuted: Boolean
        get() = sp.getBoolean("audio_bypass_muted", false)
        set(v) = sp.edit().putBoolean("audio_bypass_muted", v).commit().let {}

    /** Relay muted the stream and has not given it back yet (survives a crash). */
    var audioMutedByRelay: Boolean
        get() = sp.getBoolean("audio_muted_by_relay", false)
        set(v) = sp.edit().putBoolean("audio_muted_by_relay", v).commit().let {}

    var audioSkipConfirmPhoneToPc: Boolean
        get() = sp.getBoolean("audio_skip_confirm_phone_to_pc", false)
        set(v) = sp.edit().putBoolean("audio_skip_confirm_phone_to_pc", v).apply()


    var autoAccept: Boolean
        get() = sp.getBoolean("auto_accept", true)
        set(v) = sp.edit().putBoolean("auto_accept", v).apply()

    /** 0 = follow system, 1 = light, 2 = dark. */
    var theme: Int
        get() = sp.getInt("theme", 0)
        set(v) = sp.edit().putInt("theme", v).apply()

    /** The glass look: 0 = Native glass (the default on first run), 1 = Relay glass. */
    var themeLook: Int
        get() = sp.getInt("theme_look", 0)
        set(v) = sp.edit().putInt("theme_look", v).apply()

    /** Ask for unlock before sensitive actions (M7.1). Default: on in release and qa, off in debug. */
    var unlockGate: Boolean
        get() = sp.getBoolean("unlock_gate", dev.phonebridge.app.security.gateDefault(debugBuild))
        set(v) = sp.edit().putBoolean("unlock_gate", v).apply()

    /** Solid panels instead of see-through ones. */
    var reduceTransparency: Boolean
        get() = sp.getBoolean("reduce_transparency", false)
        set(v) = sp.edit().putBoolean("reduce_transparency", v).apply()

    /** The manual lite switch: it can force lite on; automatic reasons still apply. */
    var liteForced: Boolean
        get() = sp.getBoolean("lite_forced", false)
        set(v) = sp.edit().putBoolean("lite_forced", v).apply()

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

    /** Android has connected Notification access or control at least once: its restricted-settings lock is off. */
    var specialAccessConnected: Boolean
        get() = sp.getBoolean("special_access_connected", false)
        set(v) = sp.edit().putBoolean("special_access_connected", v).apply()

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
        get() = sp.getStringSet("allow_camera_pcs", emptySet()).orEmpty().toSet()
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
