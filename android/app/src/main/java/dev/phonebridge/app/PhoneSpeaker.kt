package dev.phonebridge.app

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.media.AudioManager
import android.os.Build
import android.util.Log
import dev.phonebridge.app.audio.PC_LEVEL_STEPS
import dev.phonebridge.app.audio.PcLevel
import dev.phonebridge.app.audio.SpeakerAction
import dev.phonebridge.app.audio.SpeakerGuard
import dev.phonebridge.app.audio.keyStep
import dev.phonebridge.app.audio.undoStep

/**
 * "Silence this phone while streaming". Mutes the media stream during a share and always gives it
 * back: on Stop, on a dropped link, and on the next start after a crash (the flags in [Prefs]). The
 * setting applies at once, also in the middle of a stream. If muting proves to silence the capture
 * too ([SpeakerGuard]), it unmutes, remembers that, and says so: never a toggle that quietly sends
 * nothing.
 *
 * The volume keys set the PC's level while the phone is silent. A foreground video app sends the
 * keys straight to the phone's volume, and a raise also unmutes it, so no app can catch them first.
 * The phone's volume is therefore held at [HELD_INDEX] under the mute: a press that unmutes for an
 * instant is near silence instead of a burst of the old volume, and it is undone when Android
 * reports it, with the press counted as a step of [PcLevel]. Everything moves the volume in relative
 * steps: this phone ignores setting a value while another app is in front.
 */
object PhoneSpeaker {
    private const val TAG = "PhoneBridgeAudio"

    /** System calls cost milliseconds: the capture thread makes them this rarely. */
    private const val SYSTEM_CHECK_FRAMES = 50
    private const val HELD_INDEX = 1
    private const val VOLUME_CHANGED = "android.media.VOLUME_CHANGED_ACTION"
    private const val MUTE_CHANGED = "android.media.STREAM_MUTE_CHANGED_ACTION"
    private const val EXTRA_STREAM = "android.media.EXTRA_VOLUME_STREAM_TYPE"
    private const val EXTRA_VALUE = "android.media.EXTRA_VOLUME_STREAM_VALUE"
    private const val EXTRA_PREV = "android.media.EXTRA_PREV_VOLUME_STREAM_VALUE"

    private var guard: SpeakerGuard? = null
    private var sharing = false
    private var frameMs = 10
    private var frames = 0
    private var musicActive = false
    private var listening = false
    private var pinned = false
    private val level = PcLevel(PC_LEVEL_STEPS)

    /** Android reports every change of the stream, so a press is undone in milliseconds. */
    private val changes = object : BroadcastReceiver() {
        override fun onReceive(ctx: Context, intent: Intent) {
            if (intent.action == VOLUME_CHANGED) {
                if (intent.getIntExtra(EXTRA_STREAM, -1) != AudioManager.STREAM_MUSIC) return
                val index = intent.getIntExtra(EXTRA_VALUE, HELD_INDEX)
                onIndex(ctx, intent.getIntExtra(EXTRA_PREV, index), index)
            }
            holdMute(ctx)
        }
    }

    private fun audio(ctx: Context) = ctx.getSystemService(AudioManager::class.java)

    private fun adjust(ctx: Context, how: Int) {
        runCatching { audio(ctx).adjustStreamVolume(AudioManager.STREAM_MUSIC, how, 0) }
            .onFailure { Log.w(TAG, "AUDIO speaker change refused: $it") }
    }

    /** [count] relative steps of the phone's volume, up when positive. */
    private fun stepIndex(ctx: Context, count: Int) {
        val how = if (count > 0) AudioManager.ADJUST_RAISE else AudioManager.ADJUST_LOWER
        repeat(kotlin.math.abs(count)) { adjust(ctx, how) }
    }

    /** The captured sound at the level the PC should hear. Capture thread. */
    fun applyLevel(buf: ByteArray, len: Int, channels: Int) = level.apply(buf, len, channels)

    /** The PC's level, set directly by the [SpeakerSilencer] trial. */
    fun setLevelStep(step: Int) {
        level.step = step
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
        if (listening) return
        val am = audio(ctx)
        level.step = p.audioPcLevel
        if (am.isStreamMute(AudioManager.STREAM_MUSIC)) {
            // The user's own mute: not ours to undo later, and its volume can't be read to restore.
            pinned = false
            listen(ctx)
            Log.i(TAG, "AUDIO phone speaker was already muted, pc level step=${level.step}")
            return
        }
        // Saved before anything changes: a crash from here on still gets it back.
        val index = am.getStreamVolume(AudioManager.STREAM_MUSIC)
        p.audioHeldIndex = index
        p.audioMutedByRelay = true
        pinned = true
        guard = SpeakerGuard(frameMs, proven = p.audioMuteKeepsCapture)
        // Muted first: lowering does not unmute, so the walk down is not heard.
        adjust(ctx, AudioManager.ADJUST_MUTE)
        stepIndex(ctx, HELD_INDEX - index)
        adjust(ctx, AudioManager.ADJUST_MUTE)
        listen(ctx)
        Log.i(TAG, "AUDIO phone speaker muted, held at $HELD_INDEX from ${p.audioHeldIndex}, pc level step=${level.step}")
    }

    private fun listen(ctx: Context) {
        val filter = IntentFilter(VOLUME_CHANGED).apply { addAction(MUTE_CHANGED) }
        val app = ctx.applicationContext
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            app.registerReceiver(changes, filter, Context.RECEIVER_NOT_EXPORTED)
        } else {
            app.registerReceiver(changes, filter)
        }
        listening = true
    }

    /** The phone's volume moved. Not the undo of an earlier press: this press is a PC level step. */
    @Synchronized
    private fun onIndex(ctx: Context, prev: Int, index: Int) {
        if (!listening || !pinned || guard?.isChecking == true) return
        val step = keyStep(HELD_INDEX, prev, index)
        if (step != 0) {
            level.adjust(step)
            Prefs(ctx).audioPcLevel = level.step
            Log.i(TAG, "AUDIO pc level step=${level.step} of $PC_LEVEL_STEPS")
        }
        stepIndex(ctx, undoStep(HELD_INDEX, index))
    }

    @Synchronized
    private fun holdMute(ctx: Context) {
        if (!listening || guard?.isChecking == true) return
        if (!audio(ctx).isStreamMute(AudioManager.STREAM_MUSIC)) adjust(ctx, AudioManager.ADJUST_MUTE)
    }

    /**
     * Every captured frame, from the capture thread. Returns true when the capture just proved to
     * die with the mute, so the owner can tell the user.
     */
    @Synchronized
    fun onFrame(ctx: Context, silent: Boolean): Boolean {
        val g = guard ?: return false
        val wasProven = g.proven
        if (++frames >= SYSTEM_CHECK_FRAMES) {
            frames = 0
            val am = audio(ctx)
            musicActive = am.isMusicActive
            // A missed report must not leave the phone playing for the rest of the share.
            if (!g.isChecking && !am.isStreamMute(AudioManager.STREAM_MUSIC)) adjust(ctx, AudioManager.ADJUST_MUTE)
        }
        val action = g.onFrame(silent, musicActive)
        if (g.proven && !wasProven) {
            Prefs(ctx).audioMuteKeepsCapture = true
            Log.i(TAG, "AUDIO sound captured through the mute: never unmuting to check again")
        }
        when (action) {
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
        pinned = false
        if (listening) {
            listening = false
            runCatching { ctx.applicationContext.unregisterReceiver(changes) }
        }
        // The phone plays again, so nothing may lower what the PC hears.
        level.step = PC_LEVEL_STEPS
        if (Prefs(ctx).audioMutedByRelay) giveBack(ctx)
    }

    /** At session end, and at every service start for the crash case. */
    @Synchronized
    fun restore(ctx: Context) {
        sharing = false
        stopMuting(ctx)
    }

    private fun giveBack(ctx: Context) {
        val p = Prefs(ctx)
        val index = p.audioHeldIndex
        adjust(ctx, AudioManager.ADJUST_UNMUTE)
        if (index >= 0) {
            val am = audio(ctx)
            stepIndex(ctx, index - am.getStreamVolume(AudioManager.STREAM_MUSIC))
        }
        p.audioMutedByRelay = false
        p.audioHeldIndex = -1
        Log.i(TAG, "AUDIO phone speaker restored to $index, reads ${audio(ctx).getStreamVolume(AudioManager.STREAM_MUSIC)}")
    }
}
