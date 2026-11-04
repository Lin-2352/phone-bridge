package dev.phonebridge.app

import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.media.projection.MediaProjection
import android.util.Log
import androidx.compose.runtime.mutableStateOf
import dev.phonebridge.app.audio.AudioDirection
import dev.phonebridge.app.audio.AudioReason
import dev.phonebridge.app.audio.AudioSessions
import dev.phonebridge.app.audio.AudioSource
import dev.phonebridge.app.audio.EndCause
import dev.phonebridge.app.audio.endNotice
import dev.phonebridge.app.audio.refusalNotice
import dev.phonebridge.app.audio.resolveEnd
import dev.phonebridge.app.audio.sessionNoticeText
import java.util.concurrent.atomic.AtomicReference
import uniffi.pb_ffi.AudioSink
import uniffi.pb_ffi.PbAudioEnd
import uniffi.pb_ffi.PbAudioOut
import uniffi.pb_ffi.PbEvent

private const val TAG = "PhoneBridgeAudio"
const val AUDIO_ASK_ID = 10
const val AUDIO_SHARE_ID = 11
const val AUDIO_PLAY_ID = 12
const val AUDIO_NOTICE_ID = 13

/** A PC asks for this phone's audio (`PbEvent.AudioRequest`). */
class AudioAsk(
    val id: ULong,
    val peer: String,
    val source: AudioSource,
    val sampleRate: Int,
    val channels: Int,
)

/**
 * The phone's audio sessions: phone to PC (capture, consented here) and PC to phone (playback,
 * consented on the PC). State changes come from the main thread (consent activity), the events
 * thread (node events) or the capture worker (source ended); methods are synchronized.
 * A session ends on Stop (either side), a link drop, or the sharing source ending.
 */
object AudioShare {
    val sessions = AudioSessions()

    /** The PC this phone's audio is going to, while it is (Home's card and the Stop button read it). */
    val sharingWith = mutableStateOf<String?>(null)

    /** The PC whose audio plays on this phone, while it does. */
    val playingFrom = mutableStateOf<String?>(null)

    /** Either direction's PC, for the "Connected" answer. */
    fun activePeer(): String? = sharingWith.value ?: playingFrom.value

    private val startedAt = LongArray(AudioDirection.entries.size)
    private val peerOf = arrayOfNulls<String>(AudioDirection.entries.size)

    private val out = AtomicReference<PbAudioOut?>(null)
    @Volatile private var player: AudioPlayer? = null
    private var streamer: AudioCapture? = null
    private var tone: Thread? = null
    private var encoder: OpusEncoder? = null

    /** Sends one capture frame to the PC: as Opus when this phone can, else as it is. */
    private fun send(o: PbAudioOut, enc: OpusEncoder?, buf: ByteArray, len: Int, channels: Int) {
        SpeakerSilencer.notePeak(buf, len)
        PhoneSpeaker.applyLevel(buf, len, channels)
        if (enc != null) enc.encode(buf, len) { o.sendFrame(it) } else o.sendFrame(buf.copyOf(len))
    }
    private var locks: AudioSessionLocks? = null

    /** Debug builds only, set over adb: send a test tone instead of asking and capturing. */
    @Volatile var debugTone = false
    private var captureSource: AudioSource? = null

    /** Set when our own source ended; read when the node reports the end. */
    @Volatile private var endOverride: EndCause? = null

    /** Frames of a PC to phone session; dropped until the player exists. */
    val sink = object : AudioSink {
        private var seen = 0L

        override fun onFrame(frame: ByteArray) {
            val p = player
            p?.push(frame)
            if (++seen % 1500 == 0L && p != null) { // about every 10 s
                Log.i(TAG, "AUDIO playing ${Bridge.node?.audioRxStats()} buffered_ms=${p.bufferedMs} dropped_ms=${p.droppedMs} underruns=${p.underruns}")
            }
        }
    }

    /** Before the projection token is created (Android 14): claim the direction and enter the service type. */
    @Synchronized
    fun prepare(a: AudioAsk): AudioReason? {
        if (!sessions.tryBegin(AudioDirection.PHONE_TO_PC)) return AudioReason.BUSY
        val svc = PbService.instance
        if (svc == null || !svc.enterAudio(a.source)) {
            sessions.end(AudioDirection.PHONE_TO_PC)
            return AudioReason.UNAVAILABLE
        }
        captureSource = a.source
        return null
    }

    /** Claims the direction for the helper, which needs no service type: nothing runs in this process but a socket. */
    @Synchronized
    fun prepareHelper(): AudioReason? =
        if (sessions.tryBegin(AudioDirection.PHONE_TO_PC)) null else AudioReason.BUSY

    /**
     * Start capturing (after [prepare] and, for playback, the projection; or after [prepareHelper]
     * with [helper]) and tell the PC.
     */
    @Synchronized
    fun launch(ctx: Context, a: AudioAsk, projection: MediaProjection?, helper: Boolean = false) {
        endOverride = null
        val app = ctx.applicationContext
        var frames = 0L
        val enc = OpusEncoder.create(a.sampleRate, a.channels)
        encoder = enc
        val deliver = { buf: ByteArray, len: Int ->
            val o = out.get()
            if (o != null) send(o, enc, buf, len, a.channels)
            if (++frames % (10_000 / AudioStreamer.FRAME_MS) == 0L) {
                Log.i(TAG, "AUDIO sharing captured_frames=$frames max_frame_gap_ms=${streamer?.takeMaxFrameGapMs()} encoder_dropped=${enc?.droppedFrames} ${o?.statsLine()}")
            }
        }
        val s: AudioCapture = if (helper) {
            HelperCapture(app, a.sampleRate, a.channels, onEnded = { reason -> sourceEnded(reason) }, onFrame = deliver)
        } else {
            AudioStreamer(
                app, a.source, projection, a.sampleRate, a.channels,
                onEnded = { reason -> sourceEnded(reason) },
                onLevel = { silent ->
                    if (PhoneSpeaker.onFrame(app, silent)) {
                        notice(app, "This phone can't stay silent while sharing", "Muting it also silences what Relay captures, so the speaker stays on.")
                    }
                },
                onFrame = deliver,
            )
        }
        val r = s.start()
        if (!r.ok) {
            s.stop()
            abort(a, r.reason ?: AudioReason.UNAVAILABLE, teardown = true)
            return
        }
        streamer = s
        val o = Bridge.node?.answerAudioAccept(a.id, a.sampleRate.toUInt(), a.channels.toUInt(), if (enc != null) 1u else 0u)
        if (o == null) {
            // The question timed out or the PC left meanwhile.
            teardownCapture()
            return
        }
        out.set(o)
        // The helper's route already keeps the speaker silent, and the real volume bar sets the level.
        if (a.source == AudioSource.PLAYBACK && !helper && !SpeakerSilencer.active) PhoneSpeaker.begin(app, AudioStreamer.FRAME_MS)
        began(AudioDirection.PHONE_TO_PC, a.peer)
        notifySession(ctx, AudioDirection.PHONE_TO_PC, a.source, a.peer)
        Log.i(TAG, "AUDIO phone to PC started source=${a.source} helper=$helper")
    }

    /** Debug builds only ([debugTone]): the real path from accept onward, with a tone as the source. */
    @Synchronized
    fun launchTone(ctx: Context, a: AudioAsk) {
        if (!sessions.tryBegin(AudioDirection.PHONE_TO_PC)) return
        val enc = OpusEncoder.create(a.sampleRate, a.channels)
        val o = Bridge.node?.answerAudioAccept(a.id, a.sampleRate.toUInt(), a.channels.toUInt(), if (enc != null) 1u else 0u)
        if (o == null) {
            enc?.release()
            sessions.end(AudioDirection.PHONE_TO_PC)
            return
        }
        out.set(o)
        PhoneSpeaker.begin(ctx.applicationContext, AudioStreamer.FRAME_MS)
        began(AudioDirection.PHONE_TO_PC, a.peer)
        notifySession(ctx, AudioDirection.PHONE_TO_PC, a.source, a.peer)
        tone = Thread({
            val bytes = a.sampleRate * 2 * a.channels * AudioStreamer.FRAME_MS / 1000
            val samples = bytes / 2 / a.channels
            val buf = ByteArray(bytes)
            var n = 0L
            var frames = 0L
            val start = android.os.SystemClock.elapsedRealtime()
            try {
                while (!Thread.currentThread().isInterrupted) {
                    for (i in 0 until samples) {
                        val v = (kotlin.math.sin(n++ % 109 / 109.0 * 2 * Math.PI) * 800).toInt()
                        for (c in 0 until a.channels) {
                            val at = (i * a.channels + c) * 2
                            buf[at] = v.toByte()
                            buf[at + 1] = (v shr 8).toByte()
                        }
                    }
                    send(o, enc, buf, bytes, a.channels)
                    frames++
                    if (frames % (10_000 / AudioStreamer.FRAME_MS) == 0L) Log.i(TAG, "AUDIO tone frames=$frames ${o.statsLine()}")
                    val due = start + frames * AudioStreamer.FRAME_MS
                    val wait = due - android.os.SystemClock.elapsedRealtime()
                    if (wait > 0) Thread.sleep(wait)
                }
            } catch (_: Exception) {
            }
            enc?.release()
        }, "pb-audio-tone").apply { start() }
        Log.i(TAG, "AUDIO test tone to the PC started")
    }

    /** Could not start: the PC gets the reason, the user gets what to do next. */
    @Synchronized
    fun abort(a: AudioAsk, reason: AudioReason, teardown: Boolean) {
        Bridge.node?.answerAudioRefuse(a.id, reason.code.toUByte())
        // Only when this request claimed the direction: a BUSY refusal must not touch the running session.
        if (teardown) teardownCapture()
        val ctx = PbService.instance ?: return
        if (reason != AudioReason.DECLINED) notice(ctx, "Audio was not shared", refusalNotice(reason, a.source))
        Log.i(TAG, "AUDIO request refused reason=$reason")
    }

    /** Stop pressed in the app or the notification. */
    @Synchronized
    fun stop(direction: AudioDirection) {
        val node = Bridge.node
        if (node == null || !node.stopAudio(direction.wire.toUByte())) ended(direction, EndCause.LOCAL_STOP)
    }

    @Synchronized
    fun stopAll() {
        for (d in AudioDirection.entries) {
            if (!sessions.isActive(d)) continue
            stop(d)
            // The node is going away: its "ended" event may never arrive, and the recorder must not outlive the service.
            if (sessions.isActive(d)) ended(d, EndCause.LOCAL_STOP)
        }
    }

    /** The capture ended by itself (app blocks capture, projection closed, read error). Worker thread. */
    private fun sourceEnded(reason: AudioReason?) {
        endOverride = if (reason == AudioReason.APP_BLOCKS_CAPTURE) EndCause.APP_BLOCKS_CAPTURE else EndCause.SOURCE_ENDED
        Log.i(TAG, "AUDIO capture ended by itself: $reason")
        stop(AudioDirection.PHONE_TO_PC)
    }

    @Synchronized
    fun onEvent(e: PbEvent) {
        when (e) {
            is PbEvent.AudioEnded -> {
                val d = AudioDirection.fromWire(e.direction.toInt()) ?: return
                val cause = if (d == AudioDirection.PHONE_TO_PC) resolveEnd(causeOf(e.cause), endOverride) else causeOf(e.cause)
                ended(d, cause)
            }
            is PbEvent.AudioStarted -> if (e.direction.toInt() == AudioDirection.PC_TO_PHONE.wire) startPlayback(e)
            is PbEvent.AudioAskEnded -> {
                PbService.instance?.getSystemService(NotificationManager::class.java)?.cancel(AUDIO_ASK_ID)
                AudioConsentActivity.closeFor(e.id)
            }
            is PbEvent.AudioRefused -> {
                val ctx = PbService.instance ?: return
                val src = if (e.source.toInt() == 1) AudioSource.MICROPHONE else AudioSource.PLAYBACK
                notice(ctx, "Audio was not shared", refusalNotice(AudioReason.fromCode(e.reason.toInt()), src))
            }
            else -> {}
        }
    }

    private fun startPlayback(e: PbEvent.AudioStarted) {
        val ctx = PbService.instance ?: return
        if (!sessions.tryBegin(AudioDirection.PC_TO_PHONE)) return
        val p = AudioPlayer(e.sampleRate.toInt(), e.channels.toInt())
        if (!p.start()) {
            sessions.end(AudioDirection.PC_TO_PHONE)
            notice(ctx, "PC audio did not start", "Relay could not open this phone's audio output. Close other apps that use it, then ask again from the PC.")
            Bridge.node?.stopAudio(AudioDirection.PC_TO_PHONE.wire.toUByte())
            return
        }
        player = p
        began(AudioDirection.PC_TO_PHONE, e.peerName)
        notifySession(ctx, AudioDirection.PC_TO_PHONE, AudioSource.PLAYBACK, e.peerName)
        Log.i(TAG, "AUDIO PC to phone started")
    }

    private fun ended(d: AudioDirection, cause: EndCause) {
        val ctx = PbService.instance
        finished(d)
        when (d) {
            AudioDirection.PHONE_TO_PC -> {
                val src = captureSource
                teardownCapture()
                ctx?.getSystemService(NotificationManager::class.java)?.cancel(AUDIO_SHARE_ID)
                Log.i(TAG, "AUDIO phone to PC ended ($cause, source=$src)")
            }
            AudioDirection.PC_TO_PHONE -> {
                player?.let {
                    Log.i(TAG, "AUDIO playback ended: dropped ${it.droppedMs} ms, ${it.underruns} underruns")
                    it.stop()
                    it.release()
                }
                player = null
                sessions.end(d)
                ctx?.getSystemService(NotificationManager::class.java)?.cancel(AUDIO_PLAY_ID)
                Log.i(TAG, "AUDIO PC to phone ended ($cause)")
            }
        }
        if (ctx != null) endNotice(cause, d)?.let { notice(ctx, "Audio stopped", it) }
    }

    /** A session is up: Home shows it until [finished]. */
    private fun began(d: AudioDirection, peer: String) {
        val app = PbService.instance
        if (app != null) {
            val l = locks ?: AudioSessionLocks(app).also { locks = it }
            l.acquire()
        }
        startedAt[d.ordinal] = System.currentTimeMillis()
        peerOf[d.ordinal] = peer
        if (d == AudioDirection.PHONE_TO_PC) sharingWith.value = peer else playingFrom.value = peer
    }

    /** The session is over: leave Home and write the Timeline entry (who, which way, how long; never audio). */
    private fun finished(d: AudioDirection) {
        val peer = peerOf[d.ordinal]
        val began = startedAt[d.ordinal]
        peerOf[d.ordinal] = null
        startedAt[d.ordinal] = 0
        if (d == AudioDirection.PHONE_TO_PC) sharingWith.value = null else playingFrom.value = null
        if (sharingWith.value == null && playingFrom.value == null) locks?.release()
        if (peer == null || began == 0L) return
        val dir = if (d == AudioDirection.PHONE_TO_PC) TlDir.ToPc else TlDir.ToPhone
        Timeline.add(audioEntry(peer, dir, audioTitle(dir), (System.currentTimeMillis() - began) / 1000))
    }

    private fun teardownCapture() {
        val src = captureSource
        tone?.interrupt()
        tone = null
        PbService.instance?.let { PhoneSpeaker.restore(it) }
        streamer?.stop()
        streamer = null
        // After the capture thread stopped: it is the encoder's only user.
        encoder?.release()
        encoder = null
        out.set(null)
        captureSource = null
        endOverride = null
        if (src != null) PbService.instance?.leaveAudio(src)
        sessions.end(AudioDirection.PHONE_TO_PC)
    }

    private fun causeOf(c: PbAudioEnd): EndCause = when (c) {
        is PbAudioEnd.PeerStopped -> EndCause.PEER_STOPPED
        is PbAudioEnd.LocalStop -> EndCause.LOCAL_STOP
        is PbAudioEnd.LinkLost -> EndCause.LINK_LOST
        is PbAudioEnd.Violation -> EndCause.VIOLATION
        is PbAudioEnd.WriteFailed -> EndCause.WRITE_FAILED
    }

    private fun notice(ctx: Context, title: String, text: String) {
        val n = Notification.Builder(ctx, "events")
            .setSmallIcon(R.drawable.ic_stat_bridge)
            .setContentTitle(title)
            .setContentText(text)
            .setStyle(Notification.BigTextStyle().bigText(text))
            .setAutoCancel(true)
            .build()
        ctx.getSystemService(NotificationManager::class.java).notify(AUDIO_NOTICE_ID, n)
    }

    /** The persistent indicator with Stop, for every active session. */
    private fun notifySession(ctx: Context, d: AudioDirection, source: AudioSource, peer: String) {
        val stop = PendingIntent.getBroadcast(
            ctx, 60 + d.wire, Intent(ctx, AudioReceiver::class.java).putExtra("dir", d.wire),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val n = Notification.Builder(ctx, "control")
            .setSmallIcon(R.drawable.ic_stat_bridge)
            .setContentTitle(sessionNoticeText(d, source))
            .setContentText(if (d == AudioDirection.PHONE_TO_PC) "Sent to $peer" else "From $peer")
            .setOngoing(true)
            .addAction(Notification.Action.Builder(null, "Stop", stop).build())
            .build()
        ctx.getSystemService(NotificationManager::class.java)
            .notify(if (d == AudioDirection.PHONE_TO_PC) AUDIO_SHARE_ID else AUDIO_PLAY_ID, n)
    }
}

/** "Stop" on an audio notification. */
class AudioReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val d = AudioDirection.fromWire(intent.getIntExtra("dir", -1)) ?: return
        AudioShare.stop(d)
        // A fresh process (the old one died) has nothing to stop; the stale bar must still go.
        context.getSystemService(NotificationManager::class.java)
            .cancel(if (d == AudioDirection.PHONE_TO_PC) AUDIO_SHARE_ID else AUDIO_PLAY_ID)
    }
}
