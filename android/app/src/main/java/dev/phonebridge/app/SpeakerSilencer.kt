package dev.phonebridge.app

import android.content.Context
import android.media.AudioManager
import android.media.audiofx.AudioEffect
import android.media.audiofx.DynamicsProcessing
import android.media.audiofx.Visualizer
import android.util.Log
import dev.phonebridge.app.audio.mixIsAudible
import dev.phonebridge.app.audio.stepsBack

private const val TAG = "PhoneBridgeAudio"
private const val EFFECT_PRIORITY = Int.MAX_VALUE
private const val SILENCE_DB = -96f
private const val STEREO = 2

/** The volume is back before the effect lets go. */
private const val SETTLE_MS = 100L

/**
 * A second layer under [PhoneSpeaker]'s mute: an effect on the output mix that silences media apps.
 * It lets the real volume bar work in an app that keeps the keys, as the stream can be unmuted while
 * that app's sound is covered. Captured sound is taken before the effect, and alarms, calls and
 * notifications play on other outputs (both heard on a realme RMX3561).
 *
 * The effect reaches one output only, so a game on another isn't covered: that is why the mute is the
 * main layer. A meter ahead of the effect tells them apart: a game's sound is captured while that
 * output plays nothing. Best effort: Android may refuse either, and the mute still holds.
 */
object SpeakerSilencer {
    private var effect: DynamicsProcessing? = null
    private var mix: Visualizer? = null
    private val reading = Visualizer.MeasurementPeakRms()

    /** The meter's last reading in millibels, for the log. */
    val lastPeakMb: Int get() = reading.mPeak

    private fun newEffect(): DynamicsProcessing {
        val config = DynamicsProcessing.Config.Builder(
            DynamicsProcessing.VARIANT_FAVOR_FREQUENCY_RESOLUTION, STEREO, false, 0, false, 0, false, 0, false,
        ).build()
        val fx = DynamicsProcessing(EFFECT_PRIORITY, 0, config)
        fx.setInputGainAllChannelsTo(SILENCE_DB)
        fx.enabled = true
        val lost = { Log.i(TAG, "AUDIO another app took the silencing effect: the mute still holds") }
        fx.setControlStatusListener(AudioEffect.OnControlStatusChangeListener { _, granted -> if (!granted) lost() })
        fx.setEnableStatusListener(AudioEffect.OnEnableStatusChangeListener { _, enabled -> if (!enabled) lost() })
        return fx
    }

    private fun openMixMeter(): Visualizer? = try {
        Visualizer(0).apply {
            measurementMode = Visualizer.MEASUREMENT_MODE_PEAK_RMS
            enabled = true
        }
    } catch (e: Exception) {
        Log.i(TAG, "AUDIO no meter on the output mix: $e")
        null
    }

    @Synchronized
    fun start() {
        if (effect != null) return
        // Made before the effect, so it sits ahead of it in the chain and hears what the effect removes.
        val meter = openMixMeter()
        try {
            val fx = newEffect()
            if (fx.hasControl() && fx.enabled) {
                effect = fx
                mix = meter
                Log.i(TAG, "AUDIO silencing effect on, under the mute, meter=${meter != null}")
                return
            }
            Log.i(TAG, "AUDIO silencing effect refused: control=${fx.hasControl()} enabled=${fx.enabled}")
            runCatching { fx.release() }
        } catch (e: Exception) {
            Log.i(TAG, "AUDIO silencing effect refused: $e")
        }
        runCatching { meter?.release() }
    }

    /**
     * A new meter: it reports the peak of a long stretch behind it, and the last sound must not be
     * taken for the next one. Android keeps a meter ahead of the effect whichever is made first.
     */
    @Synchronized
    fun freshMeter() {
        if (effect == null) return
        runCatching { mix?.release() }
        mix = openMixMeter()
    }

    /** Whether the mute can safely come off for sound the effect covers: it must be there, and be seen to work. */
    @Synchronized
    fun canCover(): Boolean = effect != null && mix != null

    /** Whether the output the effect is on is playing anything. */
    @Synchronized
    fun mixIsAudible(): Boolean {
        val meter = mix ?: return false
        return runCatching {
            meter.getMeasurementPeakRms(reading) == Visualizer.SUCCESS && mixIsAudible(reading.mPeak)
        }.getOrDefault(false)
    }

    /** At session end, and at every service start for the crash case. Does nothing if no share was silenced. */
    @Synchronized
    fun stop(ctx: Context) {
        restoreOlderShare(ctx.applicationContext)
        runCatching { mix?.release() }
        mix = null
        effect?.let { fx ->
            Thread.sleep(SETTLE_MS)
            runCatching {
                fx.enabled = false
                fx.release()
            }
        }
        effect = null
    }

    /** A version before this silenced differently: a share it left behind in a crash is put right once. */
    private fun restoreOlderShare(app: Context) {
        val p = Prefs(app)
        if (p.audioBypassMuted) {
            p.audioBypassMuted = false
            PhoneSpeaker.setStreamMuted(app, false)
        }
        val saved = p.audioSilencedFrom
        if (saved >= 0) {
            val am = app.getSystemService(AudioManager::class.java)
            PhoneSpeaker.stepIndex(app, stepsBack(am.getStreamVolume(AudioManager.STREAM_MUSIC), saved))
            p.audioSilencedFrom = -1
            Log.i(TAG, "AUDIO phone's own volume back to $saved, reads ${am.getStreamVolume(AudioManager.STREAM_MUSIC)}")
        }
    }
}
