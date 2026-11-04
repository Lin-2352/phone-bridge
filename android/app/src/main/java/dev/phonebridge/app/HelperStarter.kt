package dev.phonebridge.app

import android.Manifest
import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.RemoteInput
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.provider.Settings
import android.util.Log
import dev.phonebridge.app.audio.HELPER_OFFERED
import dev.phonebridge.app.audio.HelperFacts
import dev.phonebridge.app.audio.HelperStep
import dev.phonebridge.app.audio.helperErrorText
import dev.phonebridge.app.audio.helperRenewDue
import dev.phonebridge.app.audio.helperShellTail
import dev.phonebridge.app.audio.helperStartCommand
import dev.phonebridge.app.audio.helperStep
import dev.phonebridge.app.audio.needsPairingAgain
import dev.phonebridge.app.audio.pairingCode
import dev.phonebridge.app.audio.turnWirelessOffAfter
import dev.phonebridge.helper.HelperProtocol

private const val TAG = "PhoneBridgeAudio"
private const val ADB_WIFI = "adb_wifi_enabled"
private const val ALIVE_WAIT_MS = 3000L
private const val ALIVE_POLL_MS = 200L
private const val PAIR_WINDOW_MS = 10 * 60_000L
private const val PAIR_NOTICE_ID = 14
internal const val REPLY_KEY = "code"
private const val CHANNEL = "control"

/**
 * Keeps the audio helper running with no chore for the user. After one pairing, Relay turns Wireless
 * debugging on for a moment, starts the helper through its own adb connection, and puts Wireless
 * debugging back. It does this after a reboot or an update, or when a PC asks and the helper is gone.
 */
object HelperStarter {
    private val startLock = Any()

    fun step(ctx: Context, asked: Boolean = false): HelperStep {
        val p = Prefs(ctx)
        return helperStep(
            HelperFacts(
                paired = p.audioHelperPaired,
                sdk = Build.VERSION.SDK_INT,
                wifi = onWifi(ctx),
                alive = HelperAudio.alive(ctx),
                canToggle = canToggle(ctx),
                msSinceAttempt = System.currentTimeMillis() - p.audioHelperLastAttempt,
            ),
            asked,
        )
    }

    private fun onWifi(ctx: Context): Boolean {
        val cm = ctx.getSystemService(ConnectivityManager::class.java)
        return cm.getNetworkCapabilities(cm.activeNetwork)?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true
    }

    /** Blocking. Returns true when the helper answers afterwards. */
    fun ensure(ctx: Context, asked: Boolean = false): Boolean = synchronized(startLock) {
        when (val s = step(ctx, asked)) {
            HelperStep.NOTHING -> true
            HelperStep.START -> start(ctx)
            else -> {
                if (asked) Log.i(TAG, "AUDIO helper not started: $s")
                false
            }
        }
    }

    /** From the recovery job: bring the helper back, and keep the adb key from expiring. */
    fun maintain(ctx: Context) {
        if (!HELPER_OFFERED) return
        Thread({
            synchronized(startLock) {
                val p = Prefs(ctx)
                if (!ensure(ctx) && step(ctx) != HelperStep.NOTHING) return@Thread
                if (helperRenewDue(System.currentTimeMillis() - p.audioHelperLastConnect)) {
                    runCatching { withWireless(ctx) { AdbBridge.shell(ctx, listOf("true")) } }
                        .onSuccess { p.audioHelperLastConnect = System.currentTimeMillis() }
                        .onFailure { Log.i(TAG, "AUDIO adb key renewal failed: ${it.javaClass.simpleName}") }
                }
            }
        }, "pb-helper-maintain").start()
    }

    private fun <T> withWireless(ctx: Context, block: () -> T): T {
        val cr = ctx.contentResolver
        val wasOn = Settings.Global.getInt(cr, ADB_WIFI, 0) == 1
        if (!wasOn) Settings.Global.putInt(cr, ADB_WIFI, 1)
        try {
            return block()
        } finally {
            if (turnWirelessOffAfter(wasOn)) Settings.Global.putInt(cr, ADB_WIFI, 0)
        }
    }

    private fun startCommand(ctx: Context) = helperStartCommand(
        HelperProtocol.secretToHex(Prefs(ctx).audioHelperSecret()),
        ctx.applicationInfo.sourceDir,
    )

    private fun start(ctx: Context): Boolean {
        val p = Prefs(ctx)
        p.audioHelperLastAttempt = System.currentTimeMillis()
        val began = SystemClock.elapsedRealtime()
        val devOptions = Settings.Global.getInt(ctx.contentResolver, Settings.Global.DEVELOPMENT_SETTINGS_ENABLED, 0)
        try {
            withWireless(ctx) { AdbBridge.shell(ctx, listOf(startCommand(ctx))) }
            p.audioHelperLastConnect = System.currentTimeMillis()
        } catch (e: Exception) {
            record(ctx, "start failed, developer options $devOptions", e)
            return false
        }
        return awaitAlive(ctx).also {
            record(ctx, "start took ${SystemClock.elapsedRealtime() - began} ms, developer options $devOptions, alive=$it")
        }
    }

    private fun record(ctx: Context, what: String, e: Throwable? = null) {
        val p = Prefs(ctx)
        val text = if (e == null) what else "$what: ${helperErrorText(e, HelperProtocol.secretToHex(p.audioHelperSecret()))}"
        p.audioHelperLastResult = text
        Log.i(TAG, "AUDIO helper $text")
    }

    private fun canToggle(ctx: Context) =
        ctx.checkSelfPermission(Manifest.permission.WRITE_SECURE_SETTINGS) == PackageManager.PERMISSION_GRANTED

    /** Checked after Wireless debugging went back off: that is the state it has to survive. */
    private fun awaitAlive(ctx: Context): Boolean {
        val until = SystemClock.elapsedRealtime() + ALIVE_WAIT_MS
        while (SystemClock.elapsedRealtime() < until) {
            if (HelperAudio.alive(ctx)) return true
            Thread.sleep(ALIVE_POLL_MS)
        }
        return false
    }

    // One-time setup.

    private var stopLooking: (() -> Unit)? = null
    private val main = Handler(Looper.getMainLooper())

    /** Already paired with Wireless debugging on: no code needed. Otherwise, pairing from the start. */
    fun beginSetup(ctx: Context) {
        val app = ctx.applicationContext
        cancelLooking()
        val on = Settings.Global.getInt(app.contentResolver, ADB_WIFI, 0) == 1
        if (!AdbBridge.hasIdentity(app) || !on) {
            beginPairing(app)
            return
        }
        notify(app, "Connecting...", "This takes a few seconds.")
        Thread({
            val failure = completeSetup(app)
            when {
                failure == null -> Unit
                needsPairingAgain(failure) -> main.post { beginPairing(app) }
                else -> notifyNotFinished(app)
            }
        }, "pb-helper-setup").start()
    }

    private fun developerPage() = Intent(Settings.ACTION_APPLICATION_DEVELOPMENT_SETTINGS)
        .putExtra(":settings:fragment_args_key", "toggle_adb_wireless")
        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)

    /** Opens Android's page and waits for its "Pair with code" dialog to appear. */
    private fun beginPairing(app: Context) {
        val open = PendingIntent.getActivity(app, 71, developerPage(), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        app.getSystemService(NotificationManager::class.java).notify(
            PAIR_NOTICE_ID,
            notice(app, "Turn on Wireless debugging", "Then tap \"Pair device with pairing code\". $APP_NAME will ask for the code here.")
                .setContentIntent(open)
                .setAutoCancel(true)
                .build(),
        )
        stopLooking = AdbBridge.findPairingPort(app) { port ->
            main.post {
                cancelLooking()
                askForCode(app, port)
            }
        }
        main.postDelayed({
            cancelLooking()
            notify(app, "Setup timed out", "$APP_NAME stopped waiting for the pairing dialog. Tap Set up in its Settings to try again.")
        }, PAIR_WINDOW_MS)
        runCatching { app.startActivity(developerPage()) }.onFailure { Log.i(TAG, "AUDIO developer options not opened: $it") }
    }

    private fun cancelLooking() {
        stopLooking?.let { runCatching { it() } }
        stopLooking = null
        main.removeCallbacksAndMessages(null)
    }

    private fun notice(ctx: Context, title: String, text: String) = Notification.Builder(ctx, CHANNEL)
        .setSmallIcon(R.drawable.ic_stat_bridge)
        .setContentTitle(title)
        .setContentText(text)
        .setStyle(Notification.BigTextStyle().bigText(text))

    private fun notify(ctx: Context, title: String, text: String) {
        ctx.getSystemService(NotificationManager::class.java)
            .notify(PAIR_NOTICE_ID, notice(ctx, title, text).setAutoCancel(true).build())
    }

    private fun askForCode(ctx: Context, port: Int) {
        val reply = PendingIntent.getBroadcast(
            ctx, 70,
            Intent(ctx, HelperPairReceiver::class.java).putExtra("port", port),
            PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val input = RemoteInput.Builder(REPLY_KEY).setLabel("6-digit code").build()
        val action = Notification.Action.Builder(null, "Enter code", reply).addRemoteInput(input).build()
        // Inline only: opening any screen would take Android's pairing dialog away, and the code with it.
        val n = notice(ctx, "Enter the pairing code", "Tap Enter code below and type the 6 digits Android is showing. Leave its dialog open. No button? Drag this notification down.")
            .addAction(action)
            .setOngoing(true)
            .build()
        ctx.getSystemService(NotificationManager::class.java).notify(PAIR_NOTICE_ID, n)
    }

    internal fun onCode(ctx: Context, port: Int, typed: String?) {
        val code = pairingCode(typed.orEmpty())
        if (code == null || port <= 0) {
            if (port > 0) askForCode(ctx, port)
            return
        }
        notify(ctx, "Pairing...", "This takes a few seconds.")
        Thread({ finishSetup(ctx, port, code) }, "pb-helper-setup").start()
    }

    /** Pairs, gives Relay what it needs to restart the helper alone, and starts it. */
    private fun finishSetup(ctx: Context, port: Int, code: String) {
        try {
            AdbBridge.pair(ctx, port, code)
        } catch (e: Exception) {
            record(ctx, "pairing failed", e)
            notify(ctx, "Pairing didn't work", "Check the code and that Android's dialog was still open, then try again from $APP_NAME's Settings.")
            return
        }
        if (completeSetup(ctx) != null) notifyNotFinished(ctx)
    }

    private fun notifyNotFinished(ctx: Context) {
        notify(ctx, "Setup didn't finish", "Pairing worked, but $APP_NAME couldn't connect. Keep Wireless debugging on and tap Set up again. You won't need a new code.")
    }

    /** With a key adb accepts, takes the grant and starts the helper. Returns what went wrong, if anything. */
    private fun completeSetup(ctx: Context): Exception? {
        val p = Prefs(ctx)
        val began = SystemClock.elapsedRealtime()
        try {
            val pkg = ctx.packageName
            val answer = AdbBridge.shell(
                ctx,
                listOf(
                    "pm grant $pkg ${Manifest.permission.WRITE_SECURE_SETTINGS}",
                    "dumpsys deviceidle whitelist +$pkg",
                    startCommand(ctx),
                ),
            )
            check(canToggle(ctx)) { "the shell ran but Android did not give the grant. It said: ${helperShellTail(answer)}" }
            p.audioHelperPaired = true
            p.audioHelperLastConnect = System.currentTimeMillis()
        } catch (e: Exception) {
            record(ctx, "setup after pairing failed after ${SystemClock.elapsedRealtime() - began} ms", e)
            return e
        }
        val alive = awaitAlive(ctx)
        record(ctx, "setup finished in ${SystemClock.elapsedRealtime() - began} ms, alive=$alive")
        // The user turned it on only for this.
        runCatching { Settings.Global.putInt(ctx.contentResolver, ADB_WIFI, 0) }
        if (alive) {
            notify(ctx, "Phone audio helper is on", "Wireless debugging is off again. $APP_NAME turns it on for a moment when the helper needs starting.")
        } else {
            notify(ctx, "The helper didn't start", "$APP_NAME tries again by itself. Until then, sharing uses Android's capture.")
        }
        return null
    }

    /** Off, and Relay gives up its access: everything it can undo from here. */
    fun turnOff(ctx: Context) {
        Thread({
            synchronized(startLock) {
                val p = Prefs(ctx)
                HelperAudio.quit(ctx)
                val pkg = ctx.packageName
                runCatching {
                    withWireless(ctx) {
                        AdbBridge.shell(ctx, listOf("dumpsys deviceidle whitelist -$pkg", "pm revoke $pkg ${Manifest.permission.WRITE_SECURE_SETTINGS}"))
                    }
                }.onFailure { Log.i(TAG, "AUDIO could not revoke the grant: ${it.javaClass.simpleName}") }
                AdbBridge.forget(ctx)
                p.audioHelperPaired = false
                p.audioHelperLastAttempt = 0
                Log.i(TAG, "AUDIO helper turned off")
            }
        }, "pb-helper-off").start()
    }
}

/** The pairing code typed into the notification. */
class HelperPairReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val typed = RemoteInput.getResultsFromIntent(intent)?.getCharSequence(REPLY_KEY)?.toString()
        HelperStarter.onCode(context.applicationContext, intent.getIntExtra("port", -1), typed)
    }
}
