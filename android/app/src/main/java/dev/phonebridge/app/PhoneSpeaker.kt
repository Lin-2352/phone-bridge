package dev.phonebridge.app

import android.content.Context
import android.media.AudioManager
import android.util.Log
import dev.phonebridge.app.audio.SpeakerGuard

/**
 * "Silence this phone while streaming". Mutes the media stream for the session and always gives it
 * back: on Stop, on a dropped link, and on the next start after a crash (the flag in [Prefs]).
 * If muting silences the capture too, it unmutes at once, remembers that, and says so: never a
 * toggle that quietly sends nothing.
 */
object PhoneSpeaker {
    private const val TAG = "PhoneBridgeAudio"
    private var guard: SpeakerGuard? = null

    private fun audio(ctx: Context) = ctx.getSystemService(AudioManager::class.java)

    /** At session start. Only a stream this call muted is ever unmuted later. */
    @Synchronized
    fun begin(ctx: Context, frameMs: Int) {
        val prefs = Prefs(ctx)
        if (!prefs.audioSilencePhone || prefs.audioSilenceUnsupported) return
        val am = audio(ctx)
        if (am.isStreamMute(AudioManager.STREAM_MUSIC)) return
        prefs.audioMutedByRelay = true
        guard = SpeakerGuard(frameMs)
        runCatching { am.adjustStreamVolume(AudioManager.STREAM_MUSIC, AudioManager.ADJUST_MUTE, 0) }
            .onFailure { Log.w(TAG, "AUDIO mute refused: $it") }
        Log.i(TAG, "AUDIO phone speaker muted for the session")
    }

    /**
     * Every captured frame, from the capture thread. Returns true when the capture just proved
     * to die with the mute, so the owner can tell the user. A volume key unmutes the stream:
     * mute it again so the setting holds for the whole session.
     */
    @Synchronized
    fun onFrame(ctx: Context, silent: Boolean): Boolean {
        val g = guard ?: return false
        val am = audio(ctx)
        if (!am.isStreamMute(AudioManager.STREAM_MUSIC)) {
            runCatching { am.adjustStreamVolume(AudioManager.STREAM_MUSIC, AudioManager.ADJUST_MUTE, 0) }
            return false
        }
        if (!g.onFrame(silent, am.isMusicActive)) return false
        Log.i(TAG, "AUDIO muting silences the capture on this phone: unmuted, toggle disabled")
        Prefs(ctx).audioSilenceUnsupported = true
        guard = null
        unmute(ctx)
        return true
    }

    /** At session end, and at every service start for the crash case. */
    @Synchronized
    fun restore(ctx: Context) {
        guard = null
        if (Prefs(ctx).audioMutedByRelay) unmute(ctx)
    }

    private fun unmute(ctx: Context) {
        runCatching { audio(ctx).adjustStreamVolume(AudioManager.STREAM_MUSIC, AudioManager.ADJUST_UNMUTE, 0) }
            .onFailure { Log.w(TAG, "AUDIO unmute refused: $it") }
        Prefs(ctx).audioMutedByRelay = false
        Log.i(TAG, "AUDIO phone speaker restored")
    }
}
