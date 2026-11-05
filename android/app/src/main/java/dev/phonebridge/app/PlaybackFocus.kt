package dev.phonebridge.app

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioManager
import androidx.core.content.ContextCompat
import dev.phonebridge.app.audio.FocusEvent
import dev.phonebridge.app.audio.PlayerAction
import dev.phonebridge.app.audio.focusEventOf
import dev.phonebridge.app.audio.playerReaction

/**
 * PC audio on the phone holds audio focus like any player, so it pauses for calls and ducks for
 * short sounds, and it ends when headphones are unplugged instead of moving to the speaker.
 */
class PlaybackFocus(private val ctx: Context, private val onAction: (PlayerAction) -> Unit) {
    private val am = ctx.getSystemService(AudioManager::class.java)
    private var request: AudioFocusRequest? = null

    private val noisy = object : BroadcastReceiver() {
        override fun onReceive(c: Context, i: Intent) {
            if (i.action == AudioManager.ACTION_AUDIO_BECOMING_NOISY) onAction(playerReaction(FocusEvent.NOISY))
        }
    }

    /** False when Android refuses focus (for example during a call): don't play. */
    fun start(): Boolean {
        val r = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
            .setAudioAttributes(
                AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA).setContentType(AudioAttributes.CONTENT_TYPE_MUSIC).build(),
            )
            .setOnAudioFocusChangeListener { code -> focusEventOf(code)?.let { onAction(playerReaction(it)) } }
            .build()
        if (am.requestAudioFocus(r) != AudioManager.AUDIOFOCUS_REQUEST_GRANTED) return false
        request = r
        ContextCompat.registerReceiver(ctx, noisy, IntentFilter(AudioManager.ACTION_AUDIO_BECOMING_NOISY), ContextCompat.RECEIVER_NOT_EXPORTED) // a system broadcast still arrives
        return true
    }

    fun stop() {
        request?.let { runCatching { am.abandonAudioFocusRequest(it) } }
        if (request != null) runCatching { ctx.unregisterReceiver(noisy) }
        request = null
    }
}
