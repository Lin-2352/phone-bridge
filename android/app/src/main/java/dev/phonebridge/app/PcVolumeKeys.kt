package dev.phonebridge.app

import android.content.Context
import android.media.VolumeProvider
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.os.Handler
import android.os.Looper
import android.util.Log
import dev.phonebridge.app.audio.PC_LEVEL_STEPS
import dev.phonebridge.app.audio.clampStep
import dev.phonebridge.app.audio.stepAfterKey

/**
 * Gives the volume keys to the PC's level during a silenced share: a media session playing remotely
 * gets them instead of the phone's media volume, as when casting. The stream stays muted, because a
 * press never reaches it, and the system's bar shows the PC's level.
 *
 * Android hands a session its volume presses through the session's callback handler, and drops them
 * when there is none, then puts the bar back after a second. So the session needs a callback even
 * with nothing to do in it.
 */
object PcVolumeKeys {
    private const val TAG = "PhoneBridgeAudio"
    private var session: MediaSession? = null
    private var provider: VolumeProvider? = null

    /** The key filter holds the keys: the cast bar then always stays on, as no key reaches the real volume. */
    @Volatile var heldByFilter = false

    /** [onStep] runs on the main thread for each change the keys make. */
    @Synchronized
    fun start(ctx: Context, step: Int, onStep: (Int) -> Unit) {
        if (session != null) return
        val levels = object : VolumeProvider(VOLUME_CONTROL_ABSOLUTE, PC_LEVEL_STEPS, clampStep(step)) {
            // Android also sends a 0 when a key is let go: only the bar is meant to show.
            override fun onAdjustVolume(direction: Int) {
                if (direction != 0) change(stepAfterKey(currentVolume, direction))
            }

            override fun onSetVolumeTo(volume: Int) = change(clampStep(volume))

            private fun change(next: Int) {
                Log.i(TAG, "AUDIO volume key reached Relay, pc level step=$next")
                currentVolume = next
                onStep(next)
            }
        }
        provider = levels
        session = MediaSession(ctx.applicationContext, "RelayPcLevel").apply {
            setCallback(object : MediaSession.Callback() {}, Handler(Looper.getMainLooper()))
            setPlaybackToRemote(levels)
            setPlaybackState(
                PlaybackState.Builder().setState(PlaybackState.STATE_PLAYING, 0L, 1f).build(),
            )
            isActive = true
        }
        Log.i(TAG, "AUDIO volume keys now set the PC's level, step=$step of $PC_LEVEL_STEPS")
    }

    /** Off, the keys go where they would without Relay: Android flips between the two bars if both are on. */
    @Synchronized
    fun setActive(on: Boolean) {
        if (!on && heldByFilter) return
        runCatching { session?.isActive = on }
    }

    /** A key the filter took: one step, with the system's bar shown at the PC's level. */
    @Synchronized
    fun press(ctx: Context, direction: Int) {
        val s = session ?: return
        runCatching {
            s.isActive = true
            android.media.session.MediaController(ctx.applicationContext, s.sessionToken)
                .adjustVolume(direction, android.media.AudioManager.FLAG_SHOW_UI)
        }
    }

    /** Whether a silenced share has the key session now. */
    val running: Boolean get() = session != null

    /**
     * Android gives a volume key to the first media session in its list that is playing. Another app's session
     * moves above this one each time it starts playing; a session goes back on top when it is set to play again.
     * A window that has its own media controller (an app in front that set one) still gets its keys first.
     */
    @Synchronized
    fun reclaim() {
        val s = session ?: return
        runCatching {
            s.setPlaybackState(PlaybackState.Builder().setState(PlaybackState.STATE_PAUSED, 0L, 0f).build())
            s.setPlaybackState(PlaybackState.Builder().setState(PlaybackState.STATE_PLAYING, 0L, 1f).build())
        }
    }

    /** The level moved some other way: the bar follows. */
    @Synchronized
    fun setStep(step: Int) {
        provider?.currentVolume = clampStep(step)
    }

    @Synchronized
    fun stop() {
        val s = session ?: return
        session = null
        provider = null
        runCatching {
            s.isActive = false
            s.release()
        }
        Log.i(TAG, "AUDIO volume keys back to the phone")
    }
}
