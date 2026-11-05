package dev.phonebridge.app

import android.content.Context
import android.util.Log
import java.io.File
import java.security.KeyStore

/** The prefs only the audio helper used, all named `audio_helper_*`. */
fun helperPrefKeys(all: Set<String>): List<String> = all.filter { it.startsWith("audio_helper_") }.sorted()

/** Builds without the helper remove what an old install's helper setup left behind, once. */
fun helperCleanupDue(helperBuilt: Boolean, done: Boolean): Boolean = !helperBuilt && !done

/** Asking for audio waits for the helper only when it is offered and set up to silence the phone. */
fun helperAskWaits(offered: Boolean, mic: Boolean, paired: Boolean, silence: Boolean): Boolean =
    offered && !mic && paired && silence

/**
 * Release builds carry no audio helper. An install that once ran the helper's setup still holds its
 * stored adb identity and Keystore key; this removes them, as the helper's own "turn off" did.
 * The device-idle allow-list entry that setup added can only be removed over adb.
 */
object HelperCleanup {
    private const val TAG = "PhoneBridge"
    private const val ADB_IDENTITY = "adb_identity" // the helper's file in noBackupFilesDir
    private const val ADB_KEY_ALIAS = "relay_adb_identity" // the helper's Keystore wrapping key

    fun runOnce(ctx: Context) {
        val prefs = Prefs(ctx)
        if (!helperCleanupDue(HELPER_BUILT, prefs.helperCleaned)) return
        runCatching { File(ctx.noBackupFilesDir, ADB_IDENTITY).delete() }
        runCatching { KeyStore.getInstance("AndroidKeyStore").apply { load(null) }.deleteEntry(ADB_KEY_ALIAS) }
        prefs.removeKeys(helperPrefKeys(prefs.allKeys))
        prefs.helperCleaned = true
        Log.i(TAG, "helper leftovers removed")
    }
}
