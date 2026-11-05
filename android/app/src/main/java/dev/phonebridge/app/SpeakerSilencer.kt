package dev.phonebridge.app

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.media.AudioManager
import android.media.audiofx.AudioEffect
import android.media.audiofx.DynamicsProcessing
import android.util.Log
import androidx.core.content.ContextCompat
import dev.phonebridge.app.audio.mustLiftSilencer
import dev.phonebridge.app.audio.shareStartIndex
import dev.phonebridge.app.audio.stepForIndex
import dev.phonebridge.app.audio.stepsBack

private const val TAG = "PhoneBridgeAudio"
private const val VOLUME_CHANGED = "android.media.VOLUME_CHANGED_ACTION"
private const val EFFECT_PRIORITY = Int.MAX_VALUE
private const val SILENCE_DB = -96f
private const val STEREO = 2

/** The effect takes hold before the volume moves, and the volume is down before it lets go. */
private const val SETTLE_MS = 100L

/**
 * Silences the speaker with an effect on the output mix instead of muting, so nothing fights the
 * volume keys: the phone's real bar moves as usual and sets the PC's level. Captured sound is taken
 * before the effect, and alarms, calls and notifications play on other outputs (both heard on a
 * realme RMX3561). During a share the bar is the PC's: the phone's own volume is saved first and put
 * back after, also on the next start after a crash. While Relay is dead the effect is gone, so sound
 * can come back at the share's level until the service restarts. Nothing can prevent that gap.
 */
object SpeakerSilencer {
    @Volatile private var effect: DynamicsProcessing? = null
    private var listening = false
    @Volatile private var lifted = false

    /** An alarm, ringtone or emergency alert plays: let it through, then silence again (gain only,
     *  so the effect stays enabled and nothing reads as "taken over by another app"). */
    private val playback = object : AudioManager.AudioPlaybackCallback() {
        override fun onPlaybackConfigChanged(configs: MutableList<android.media.AudioPlaybackConfiguration>) {
            // Main thread: no lock here (start() holds it through a settle), just the current effect.
            val lift = mustLiftSilencer(configs.map { it.audioAttributes.usage })
            val fx = effect ?: return
            if (lift == lifted) return
            lifted = lift
            runCatching { fx.setInputGainAllChannelsTo(if (lift) 0f else SILENCE_DB) }
            Log.i(TAG, "AUDIO silencer ${if (lift) "lifted for an alarm or ringtone" else "back on"}")
        }
    }

    private val volume = object : BroadcastReceiver() {
        override fun onReceive(c: Context, i: Intent) = followVolume(c)
    }

    private fun followVolume(ctx: Context) {
        val am = ctx.getSystemService(AudioManager::class.java)
        val index = am.getStreamVolume(AudioManager.STREAM_MUSIC)
        val step = stepForIndex(index, am.getStreamMaxVolume(AudioManager.STREAM_MUSIC))
        PhoneSpeaker.setLevelStep(step)
        Prefs(ctx).audioShareIndex = index
        Log.i(TAG, "AUDIO silencer index=$index pc level step=$step")
    }

    private fun newEffect(onLost: () -> Unit): DynamicsProcessing {
        val config = DynamicsProcessing.Config.Builder(
            DynamicsProcessing.VARIANT_FAVOR_FREQUENCY_RESOLUTION, STEREO, false, 0, false, 0, false, 0, false,
        ).build()
        val fx = DynamicsProcessing(EFFECT_PRIORITY, 0, config)
        fx.setInputGainAllChannelsTo(SILENCE_DB)
        fx.enabled = true
        fx.setControlStatusListener(AudioEffect.OnControlStatusChangeListener { _, granted -> if (!granted) onLost() })
        fx.setEnableStatusListener(AudioEffect.OnEnableStatusChangeListener { _, enabled -> if (!enabled) onLost() })
        return fx
    }

    /** False when Android refuses the effect. [onLost]: another app took it over later. */
    @Synchronized
    fun start(ctx: Context, onLost: () -> Unit): Boolean {
        val fx = try {
            newEffect(onLost)
        } catch (e: Exception) {
            Log.i(TAG, "AUDIO silencer refused: $e")
            return false
        }
        if (!fx.hasControl() || !fx.enabled) {
            Log.i(TAG, "AUDIO silencer refused: control=${fx.hasControl()} enabled=${fx.enabled}")
            runCatching { fx.release() }
            return false
        }
        effect = fx
        lifted = false
        val app = ctx.applicationContext
        val am = app.getSystemService(AudioManager::class.java)
        am.registerAudioPlaybackCallback(playback, null)
        // The callback reports changes only: an alarm already ringing must be let through now.
        runCatching { playback.onPlaybackConfigChanged(am.activePlaybackConfigurations) }
        val p = Prefs(app)
        val own = am.getStreamVolume(AudioManager.STREAM_MUSIC)
        // Saved before anything moves: a crash from here on still gets it back.
        p.audioSilencedFrom = own
        Thread.sleep(SETTLE_MS)
        val start = shareStartIndex(p.audioShareIndex, am.getStreamMaxVolume(AudioManager.STREAM_MUSIC))
        PhoneSpeaker.stepIndex(app, start - own)
        ContextCompat.registerReceiver(app, volume, IntentFilter(VOLUME_CHANGED), ContextCompat.RECEIVER_NOT_EXPORTED)
        listening = true
        followVolume(app)
        Log.i(TAG, "AUDIO speaker silenced by an effect, phone's own volume $own, share starts at $start")
        return true
    }

    /** At session end, and at every service start for the crash case. Does nothing if no share was silenced. */
    @Synchronized
    fun stop(ctx: Context) {
        val app = ctx.applicationContext
        // First, so the walk back isn't taken for the user moving the PC's level.
        if (listening) {
            listening = false
            runCatching { app.unregisterReceiver(volume) }
        }
        val p = Prefs(app)
        val saved = p.audioSilencedFrom
        if (saved >= 0) {
            val am = app.getSystemService(AudioManager::class.java)
            PhoneSpeaker.stepIndex(app, stepsBack(am.getStreamVolume(AudioManager.STREAM_MUSIC), saved))
            p.audioSilencedFrom = -1
            Log.i(TAG, "AUDIO phone's own volume back to $saved, reads ${am.getStreamVolume(AudioManager.STREAM_MUSIC)}")
        }
        effect?.let { fx ->
            runCatching { app.getSystemService(AudioManager::class.java).unregisterAudioPlaybackCallback(playback) }
            Thread.sleep(SETTLE_MS)
            runCatching {
                fx.enabled = false
                fx.release()
            }
        }
        effect = null
    }
}
