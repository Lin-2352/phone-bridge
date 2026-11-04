package dev.phonebridge.app

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.ApplicationInfo
import android.media.AudioManager
import android.media.audiofx.DynamicsProcessing
import android.util.Log
import androidx.core.content.ContextCompat
import dev.phonebridge.app.audio.stepForIndex

private const val TAG = "PhoneBridgeAudio"
private const val ACTION = "dev.phonebridge.debug.SILENCER"
private const val VOLUME_CHANGED = "android.media.VOLUME_CHANGED_ACTION"
private const val EFFECT_PRIORITY = Int.MAX_VALUE
private const val SILENCE_DB = -96f
private const val STEREO = 2
private const val PEAK_EVERY_FRAMES = 300

/**
 * Trial, debug builds only: silence the speaker with an effect on the output mix instead of muting.
 * The volume index then moves as usual, so the phone's real bar works, and it sets the PC's level.
 * Driven over adb, never by the normal share path, until the phone shows it silences the speaker,
 * leaves the capture alone and spares alarms. Silencing that fails must be visible: every step logs.
 */
object SpeakerSilencer {
    private var effect: DynamicsProcessing? = null
    private var listening = false
    private var measuring = false
    private var frames = 0
    private var peak = 0

    /** A share must not start muting underneath the trial. */
    @Volatile var active = false
        private set

    fun registerDebug(ctx: Context) {
        val app = ctx.applicationContext
        if (app.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE == 0) return
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(c: Context, i: Intent) {
                when (i.getStringExtra("do")) {
                    "on" -> on(app)
                    "off" -> off(app)
                    "measure" -> measuring = true
                }
            }
        }
        ContextCompat.registerReceiver(app, receiver, IntentFilter(ACTION), ContextCompat.RECEIVER_EXPORTED)
    }

    private val volume = object : BroadcastReceiver() {
        override fun onReceive(c: Context, i: Intent) = followVolume(c)
    }

    private fun followVolume(ctx: Context) {
        val am = ctx.getSystemService(AudioManager::class.java)
        val index = am.getStreamVolume(AudioManager.STREAM_MUSIC)
        val step = stepForIndex(index, am.getStreamMaxVolume(AudioManager.STREAM_MUSIC))
        PhoneSpeaker.setLevelStep(step)
        Log.i(TAG, "AUDIO silencer index=$index pc level step=$step")
    }

    @Synchronized
    private fun on(app: Context) {
        measuring = true
        PhoneSpeaker.restore(app) // The trial needs the speaker unmuted, or it proves nothing.
        active = true
        try {
            val config = DynamicsProcessing.Config.Builder(
                DynamicsProcessing.VARIANT_FAVOR_FREQUENCY_RESOLUTION, STEREO, false, 0, false, 0, false, 0, false,
            ).build()
            val fx = DynamicsProcessing(EFFECT_PRIORITY, 0, config)
            fx.setInputGainAllChannelsTo(SILENCE_DB)
            fx.enabled = true
            effect = fx
            Log.i(TAG, "AUDIO silencer on: control=${fx.hasControl()} enabled=${fx.enabled} gain=${fx.getInputGainByChannelIndex(0)} session=0")
        } catch (e: Exception) {
            Log.i(TAG, "AUDIO silencer refused: $e")
            active = false
            return
        }
        if (!listening) {
            ContextCompat.registerReceiver(app, volume, IntentFilter(VOLUME_CHANGED), ContextCompat.RECEIVER_NOT_EXPORTED)
            listening = true
        }
        followVolume(app)
    }

    @Synchronized
    private fun off(app: Context) {
        effect?.let { runCatching { it.enabled = false; it.release() } }
        effect = null
        active = false
        if (listening) {
            listening = false
            runCatching { app.unregisterReceiver(volume) }
        }
        PhoneSpeaker.setLevelStep(dev.phonebridge.app.audio.PC_LEVEL_STEPS)
        Log.i(TAG, "AUDIO silencer off")
    }

    /** The loudest captured sample, before any level: whether the effect also reaches the capture. Capture thread. */
    fun notePeak(buf: ByteArray, len: Int) {
        if (!measuring) return
        var i = 0
        while (i + 1 < len) {
            val v = (buf[i].toInt() and 0xff) or (buf[i + 1].toInt() shl 8)
            peak = maxOf(peak, kotlin.math.abs(v))
            i += 2
        }
        if (++frames >= PEAK_EVERY_FRAMES) {
            Log.i(TAG, "AUDIO silencer active=$active captured_peak=$peak")
            frames = 0
            peak = 0
        }
    }
}
