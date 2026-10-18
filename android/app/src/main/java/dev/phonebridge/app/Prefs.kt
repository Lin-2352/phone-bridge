package dev.phonebridge.app

import android.content.Context

/** User settings. Universal Clipboard is off until the user turns it on. */
class Prefs(context: Context) {
    private val sp = context.getSharedPreferences("settings", Context.MODE_PRIVATE)

    var clipboardSync: Boolean
        get() = sp.getBoolean("clipboard_sync", false)
        set(v) = sp.edit().putBoolean("clipboard_sync", v).apply()

    /** Fingerprint of the last clipboard text synced either way, so focus
     *  changes never re-send a clipboard that has not changed (survives restarts). */
    var lastClip: Int
        get() = sp.getInt("last_clip", 0)
        set(v) = sp.edit().putInt("last_clip", v).apply()

    var autoAccept: Boolean
        get() = sp.getBoolean("auto_accept", true)
        set(v) = sp.edit().putBoolean("auto_accept", v).apply()
}
