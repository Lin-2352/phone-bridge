package dev.phonebridge.helper

import android.annotation.SuppressLint
import android.app.Application
import android.app.Instrumentation
import android.content.AttributionSource
import android.content.Context
import android.content.ContextWrapper
import android.content.pm.ApplicationInfo
import android.media.AudioFormat
import android.media.AudioRecord
import android.os.Build
import android.os.Looper
import android.os.Process
import android.util.Log
import androidx.annotation.RequiresApi
import java.io.DataInputStream
import java.io.File
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.security.SecureRandom
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.system.exitProcess

private const val TAG = "RelayHelper"
private const val REMOTE_SUBMIX = 8
private const val SHELL_PACKAGE = "com.android.shell"
private const val APK_CHECK_MS = 30_000L
private const val HANDSHAKE_MS = 2000

/**
 * Relay's audio helper: a small program started as the shell user with `app_process` from Relay's
 * own APK. It does one thing, capture the phone's output from REMOTE_SUBMIX for Relay, which also
 * takes the sound off the speaker and leaves the phone's real volume bar in charge of the level.
 * There is deliberately no way to ask it to do anything else. Recording exists only while Relay
 * holds the connection: if Relay goes away, the sound goes back to the speaker at once.
 *
 * The capture approach follows scrcpy's AudioDirectCapture and Workarounds (Apache-2.0,
 * Genymobile/scrcpy).
 *
 * Argument: the APK path this was started from. Environment: the shared secret, which only this
 * user can read, unlike the command line.
 */
object Main {
    private val capturing = AtomicBoolean(false)

    /** Android 11 only lets this capture with a shell window in front, so Relay uses its normal capture there. */
    @JvmStatic
    fun main(args: Array<String>) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) {
            Log.i(TAG, "needs Android 12 or later")
            exitProcess(0)
        }
        val secret = HelperProtocol.secretFromHex(System.getenv(HelperProtocol.SECRET_ENV))
        val apk = args.getOrNull(0)
        if (secret == null || apk == null) {
            System.err.println("usage: $HelperProtocol.SECRET_ENV=<secret> <apk path>")
            exitProcess(2)
        }
        val server = try {
            ServerSocket(HelperProtocol.PORT, 4, InetAddress.getLoopbackAddress())
        } catch (e: Exception) {
            Log.i(TAG, "port is taken, by another helper or another app: $e")
            exitProcess(0)
        }
        Looper.prepareMainLooper()
        Shell.prepare()
        watchApk(apk)
        Log.i(TAG, "helper ready, uid ${Process.myUid()}")
        while (true) {
            val client = server.accept()
            Thread({ serve(client, secret) }, "relay-helper-client").start()
        }
    }

    /** Relay was updated or removed: this copy must not outlive the APK it came from. */
    private fun watchApk(apk: String) {
        Thread({
            while (File(apk).exists()) Thread.sleep(APK_CHECK_MS)
            Log.i(TAG, "apk is gone, exiting")
            exitProcess(0)
        }, "relay-helper-apk").apply {
            isDaemon = true
            start()
        }
    }

    @RequiresApi(Build.VERSION_CODES.S)
    private fun serve(client: Socket, secret: ByteArray) {
        client.use { s ->
            try {
                s.soTimeout = HANDSHAKE_MS
                s.tcpNoDelay = true
                if (!handshake(s, secret)) {
                    Log.w(TAG, "refused a connection without the secret")
                    return
                }
                val input = DataInputStream(s.getInputStream())
                when (input.readByte()) {
                    HelperProtocol.CMD_PING -> s.getOutputStream().write(HelperProtocol.VERSION.toInt())
                    HelperProtocol.CMD_CAPTURE -> {
                        val raw = ByteArray(HelperProtocol.PARAMS_BYTES)
                        input.readFully(raw)
                        s.soTimeout = 0
                        capture(s, HelperProtocol.decodeParams(raw))
                    }
                }
            } catch (e: Exception) {
                Log.i(TAG, "client ended: $e")
            }
        }
    }

    /** Relay proves itself first, so a stranger gets nothing but a nonce. */
    private fun handshake(s: Socket, secret: ByteArray): Boolean {
        val helperNonce = ByteArray(HelperProtocol.NONCE_BYTES).also { SecureRandom().nextBytes(it) }
        val out = s.getOutputStream()
        out.write(helperNonce)
        val input = DataInputStream(s.getInputStream())
        val relayNonce = ByteArray(HelperProtocol.NONCE_BYTES).also { input.readFully(it) }
        val got = ByteArray(HelperProtocol.PROOF_BYTES).also { input.readFully(it) }
        if (!HelperProtocol.proofOk(secret, HelperProtocol.ROLE_RELAY, relayNonce, helperNonce, got)) return false
        out.write(HelperProtocol.proof(secret, HelperProtocol.ROLE_HELPER, relayNonce, helperNonce))
        return true
    }

    @RequiresApi(Build.VERSION_CODES.S)
    private fun capture(s: Socket, p: HelperProtocol.CaptureParams?) {
        val out = s.getOutputStream()
        if (p == null) {
            out.write(HelperProtocol.REPLY_FAILED.toInt())
            return
        }
        if (!capturing.compareAndSet(false, true)) {
            out.write(HelperProtocol.REPLY_BUSY.toInt())
            return
        }
        var rec: AudioRecord? = null
        try {
            rec = Shell.recorder(p)
            rec.startRecording()
            if (rec.recordingState != AudioRecord.RECORDSTATE_RECORDING) throw IllegalStateException("recorder did not start")
            out.write(HelperProtocol.REPLY_OK.toInt())
            Log.i(TAG, "capture started: ${p.sampleRate} Hz, ${p.channels} ch")
            // Relay closing the socket must stop recording even when no audio is flowing.
            val running = AtomicBoolean(true)
            val started = rec
            Thread({
                runCatching { while (s.getInputStream().read() >= 0) { /* nothing is expected */ } }
                running.set(false)
                runCatching { started.stop() }
            }, "relay-helper-eof").apply {
                isDaemon = true
                start()
            }
            val buf = ByteArray(HelperProtocol.frameBytes(p.sampleRate, p.channels))
            while (running.get()) {
                val n = rec.read(buf, 0, buf.size)
                if (n < 0) throw IllegalStateException("read error $n")
                if (n > 0) out.write(buf, 0, n)
            }
        } catch (e: Exception) {
            Log.i(TAG, "capture ended: $e")
            runCatching { if (rec == null) out.write(HelperProtocol.REPLY_FAILED.toInt()) }
        } finally {
            runCatching { rec?.release() }
            capturing.set(false)
            Log.i(TAG, "capture stopped")
        }
    }
}

/**
 * Android expects a Context from an installed app; this process is a bare program, so it presents
 * itself as the shell package, which is what it runs as.
 */
@RequiresApi(Build.VERSION_CODES.S)
@SuppressLint("PrivateApi", "DiscouragedPrivateApi", "BlockedPrivateApi", "SoonBlockedPrivateApi")
private object Shell {
    private val threadClass = Class.forName("android.app.ActivityThread")
    private val thread: Any = threadClass.getDeclaredConstructor().run {
        isAccessible = true
        newInstance()
    }
    private lateinit var context: Context

    private fun field(name: String) = threadClass.getDeclaredField(name).apply { isAccessible = true }

    @RequiresApi(Build.VERSION_CODES.S)
    private class ShellContext(base: Context) : ContextWrapper(base) {
        override fun getPackageName() = SHELL_PACKAGE
        override fun getOpPackageName() = SHELL_PACKAGE
        override fun getApplicationContext(): Context = this
        override fun getAttributionSource(): AttributionSource =
            AttributionSource.Builder(Process.SHELL_UID).setPackageName(SHELL_PACKAGE).build()
    }

    fun prepare() {
        field("sCurrentActivityThread").set(null, thread)
        field("mSystemThread").setBoolean(thread, true)
        attempt("configuration controller") {
            val c = Class.forName("android.app.ConfigurationController")
            val i = Class.forName("android.app.ActivityThreadInternal")
            val made = c.getDeclaredConstructor(i).run {
                isAccessible = true
                newInstance(thread)
            }
            field("mConfigurationController").set(thread, made)
        }
        attempt("app info") {
            val data = Class.forName("android.app.ActivityThread\$AppBindData")
            val bound = data.getDeclaredConstructor().run {
                isAccessible = true
                newInstance()
            }
            data.getDeclaredField("appInfo").apply { isAccessible = true }
                .set(bound, ApplicationInfo().apply { packageName = SHELL_PACKAGE })
            field("mBoundApplication").set(thread, bound)
        }
        val system = threadClass.getDeclaredMethod("getSystemContext").invoke(thread) as Context
        context = ShellContext(system)
        attempt("app context") {
            field("mInitialApplication").set(thread, Instrumentation.newApplication(Application::class.java, context))
        }
    }

    /** These only smooth over differences between phones: failing one is not an error. */
    private inline fun attempt(what: String, block: () -> Unit) {
        try {
            block()
        } catch (t: Throwable) {
            Log.i(TAG, "could not fill $what: $t")
        }
    }

    @SuppressLint("WrongConstant", "MissingPermission")
    fun recorder(p: HelperProtocol.CaptureParams): AudioRecord {
        val mask = if (p.channels == 1) AudioFormat.CHANNEL_IN_MONO else AudioFormat.CHANNEL_IN_STEREO
        val builder = AudioRecord.Builder()
            .setContext(context)
            .setAudioSource(REMOTE_SUBMIX)
            .setAudioFormat(
                AudioFormat.Builder()
                    .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                    .setSampleRate(p.sampleRate)
                    .setChannelMask(mask)
                    .build(),
            )
        val min = AudioRecord.getMinBufferSize(p.sampleRate, mask, AudioFormat.ENCODING_PCM_16BIT)
        if (min > 0) builder.setBufferSizeInBytes(8 * min)
        return builder.build()
    }
}
