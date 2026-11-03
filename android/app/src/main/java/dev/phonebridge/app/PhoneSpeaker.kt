package dev.phonebridge.app

import android.content.Context
import android.media.AudioManager
import android.util.Log
import dev.phonebridge.app.audio.SpeakerAction
import dev.phonebridge.app.audio.SpeakerGuard

/**
 * "Silence this phone while streaming". Mutes the media stream during a share and always gives it
 * back: on Stop, on a dropped link, and on the next start after a crash (the flag in [Prefs]). The
 * setting applies at once, also in the middle of a stream. If muting proves to silence the capture
 * too ([SpeakerGuard]), it unmutes, remembers that, and says so: never a toggle that quietly sends
 * nothing.
 */
object PhoneSpeaker {
    private const val TAG = "PhoneBridgeAudio"

    /** System calls cost milliseconds: the capture thread makes them this rarely. */
    private const val SYSTEM_CHECK_FRAMES = 50

    private var guard: SpeakerGuard? = null
    private var sharing = false
    private var frameMs = 10
    private var frames = 0
    private var musicActive = false

    private fun audio(ctx: Context) = ctx.getSystemService(AudioManager::class.java)

    private fun adjust(ctx: Context, how: Int) {
        runCatching { audio(ctx).adjustStreamVolume(AudioManager.STREAM_MUSIC, how, 0) }
            .onFailure { Log.w(TAG, "AUDIO speaker change refused: $it") }
    }

    /** A share of this phone's audio started. */
    @Synchronized
    fun begin(ctx: Context, frameMs: Int) {
        sharing = true
        this.frameMs = frameMs
        apply(ctx)
    }

    /** The setting changed: takes effect now if a share is running. */
    @Synchronized
    fun settingChanged(ctx: Context) {
        if (!sharing) return
        val p = Prefs(ctx)
        if (p.audioSilencePhone && !p.audioSilenceUnsupported) apply(ctx) else stopMuting(ctx)
    }

    private fun apply(ctx: Context) {
        val p = Prefs(ctx)
        if (!p.audioSilencePhone) {
            Log.i(TAG, "AUDIO phone speaker stays on: the setting is off")
            return
        }
        if (p.audioSilenceUnsupported) {
            Log.i(TAG, "AUDIO phone speaker stays on: muting silenced the capture before")
            return
        }
        val am = audio(ctx)
        if (am.isStreamMute(AudioManager.STREAM_MUSIC)) {
            // The user's own mute: not ours to undo later.
            Log.i(TAG, "AUDIO phone speaker was already muted")
            return
        }
        p.audioMutedByRelay = true
        guard = SpeakerGuard(frameMs)
        adjust(ctx, AudioManager.ADJUST_MUTE)
        Log.i(TAG, "AUDIO phone speaker muted, stream reads muted=${am.isStreamMute(AudioManager.STREAM_MUSIC)}")
    }

    /**
     * Every captured frame, from the capture thread. Returns true when the capture just proved to
     * die with the mute, so the owner can tell the user.
     */
    @Synchronized
    fun onFrame(ctx: Context, silent: Boolean): Boolean {
        val g = guard ?: return false
        if (++frames >= SYSTEM_CHECK_FRAMES) {
            frames = 0
            val am = audio(ctx)
            musicActive = am.isMusicActive
            // A volume key unmutes the stream: the setting holds for the whole share.
            if (!g.isChecking && !am.isStreamMute(AudioManager.STREAM_MUSIC)) adjust(ctx, AudioManager.ADJUST_MUTE)
        }
        when (g.onFrame(silent, musicActive)) {
            SpeakerAction.NONE -> return false
            SpeakerAction.UNMUTE_TO_CHECK -> {
                Log.i(TAG, "AUDIO silence while muted: unmuting to check the capture")
                adjust(ctx, AudioManager.ADJUST_UNMUTE)
                return false
            }
            SpeakerAction.MUTE_AGAIN -> {
                Log.i(TAG, "AUDIO the content is just quiet: muting again")
                adjust(ctx, AudioManager.ADJUST_MUTE)
                return false
            }
            SpeakerAction.GIVE_UP -> {
                Log.i(TAG, "AUDIO muting silences the capture on this phone: speaker stays on")
                Prefs(ctx).audioSilenceUnsupported = true
                stopMuting(ctx)
                return true
            }
        }
    }

    private fun stopMuting(ctx: Context) {
        guard = null
        if (Prefs(ctx).audioMutedByRelay) unmute(ctx)
    }

    /** At session end, and at every service start for the crash case. */
    @Synchronized
    fun restore(ctx: Context) {
        sharing = false
        stopMuting(ctx)
    }

    private fun unmute(ctx: Context) {
        adjust(ctx, AudioManager.ADJUST_UNMUTE)
        Prefs(ctx).audioMutedByRelay = false
        Log.i(TAG, "AUDIO phone speaker restored")
    }
}
