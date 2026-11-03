package dev.phonebridge.app

import android.content.Context
import android.os.SystemClock
import android.util.Log
import dev.phonebridge.app.audio.AudioReason
import dev.phonebridge.helper.HelperProtocol
import java.io.DataInputStream
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.Socket
import java.security.SecureRandom
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong

private const val TAG = "PhoneBridgeAudio"
private const val ANSWER_TIMEOUT_MS = 1000
private const val JOIN_MS = 1000L

/** Relay's side of the audio helper ([dev.phonebridge.helper.Main]). */
object HelperAudio {
    /** Connects and proves both ways, whoever holds the port: a stranger is never read from. */
    internal fun connect(ctx: Context): Socket {
        val secret = Prefs(ctx).audioHelperSecret()
        val s = Socket()
        try {
            s.connect(InetSocketAddress(InetAddress.getLoopbackAddress(), HelperProtocol.PORT), ANSWER_TIMEOUT_MS)
            s.soTimeout = ANSWER_TIMEOUT_MS
            s.tcpNoDelay = true
            val input = DataInputStream(s.getInputStream())
            val helperNonce = ByteArray(HelperProtocol.NONCE_BYTES).also { input.readFully(it) }
            val relayNonce = ByteArray(HelperProtocol.NONCE_BYTES).also { SecureRandom().nextBytes(it) }
            s.getOutputStream().write(relayNonce + HelperProtocol.proof(secret, HelperProtocol.ROLE_RELAY, relayNonce, helperNonce))
            val got = ByteArray(HelperProtocol.PROOF_BYTES).also { input.readFully(it) }
            check(HelperProtocol.proofOk(secret, HelperProtocol.ROLE_HELPER, relayNonce, helperNonce, got)) {
                "whatever holds the port does not know the secret"
            }
            return s
        } catch (e: Exception) {
            runCatching { s.close() }
            throw e
        }
    }

    /** Asks a running helper to exit. Nothing to do when none is running. */
    fun quit(ctx: Context) {
        runCatching { connect(ctx).use { it.getOutputStream().write(HelperProtocol.CMD_QUIT.toInt()) } }
    }

    /** A real round trip: never remembered, since the helper can die at any time. */
    fun alive(ctx: Context): Boolean = try {
        connect(ctx).use { s ->
            s.getOutputStream().write(HelperProtocol.CMD_PING.toInt())
            s.getInputStream().read() == HelperProtocol.VERSION.toInt()
        }
    } catch (e: Exception) {
        Log.i(TAG, "AUDIO helper not answering: $e")
        false
    }
}

/** Reads the helper's PCM as 10 ms frames, with the same contract as [AudioStreamer]. */
class HelperCapture(
    private val ctx: Context,
    private val sampleRate: Int,
    private val channels: Int,
    private val onEnded: (AudioReason?) -> Unit,
    private val onFrame: (ByteArray, Int) -> Unit,
) : AudioCapture {
    private val stopped = AtomicBoolean(false)
    private val maxFrameGapMs = AtomicLong(0)
    private var socket: Socket? = null
    private var worker: Thread? = null

    override fun takeMaxFrameGapMs(): Long = maxFrameGapMs.getAndSet(0)

    override fun start(): AudioStartResult {
        val s = try {
            HelperAudio.connect(ctx)
        } catch (e: Exception) {
            Log.i(TAG, "AUDIO helper not reachable: $e")
            return AudioStartResult(AudioReason.UNAVAILABLE)
        }
        try {
            val params = HelperProtocol.encodeParams(HelperProtocol.CaptureParams(sampleRate, channels))
            s.getOutputStream().write(byteArrayOf(HelperProtocol.CMD_CAPTURE) + params)
            val reply = s.getInputStream().read().toByte()
            if (reply != HelperProtocol.REPLY_OK) {
                s.close()
                return AudioStartResult(if (reply == HelperProtocol.REPLY_BUSY) AudioReason.BUSY else AudioReason.UNAVAILABLE)
            }
            s.soTimeout = 0
        } catch (e: Exception) {
            Log.i(TAG, "AUDIO helper refused capture: $e")
            runCatching { s.close() }
            return AudioStartResult(AudioReason.UNAVAILABLE)
        }
        socket = s
        worker = Thread({ read(s) }, "pb-audio-helper").apply {
            priority = Thread.MAX_PRIORITY
            start()
        }
        return AudioStartResult(null)
    }

    private fun read(s: Socket) {
        val buf = ByteArray(HelperProtocol.frameBytes(sampleRate, channels))
        val input = DataInputStream(s.getInputStream())
        var lastFrameAt = 0L
        Log.i(TAG, "capture started: helper, $sampleRate Hz, $channels ch")
        try {
            while (!stopped.get()) {
                input.readFully(buf)
                val now = SystemClock.elapsedRealtime()
                if (lastFrameAt != 0L) maxFrameGapMs.accumulateAndGet(now - lastFrameAt, ::maxOf)
                lastFrameAt = now
                onFrame(buf, buf.size)
            }
        } catch (e: Exception) {
            // The helper died, or stop() closed the socket.
            if (stopped.compareAndSet(false, true)) {
                Log.i(TAG, "AUDIO helper capture ended by itself: $e")
                runCatching { s.close() }
                runCatching { onEnded(AudioReason.UNAVAILABLE) }
            }
        }
        Log.i(TAG, "capture worker exiting")
    }

    override fun stop() {
        stopped.set(true)
        runCatching { socket?.close() }
        val t = worker
        if (t != null && t !== Thread.currentThread()) {
            runCatching { t.join(JOIN_MS) }
        }
        worker = null
        socket = null
    }
}
