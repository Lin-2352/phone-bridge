package dev.phonebridge.app

import android.app.job.JobInfo
import android.app.job.JobParameters
import android.app.job.JobScheduler
import android.app.job.JobService
import android.content.BroadcastReceiver
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.os.PowerManager
import android.util.Log

private const val TAG = "PhoneBridge"
private const val JOB_ID = 4701

/**
 * Brings Phone Bridge back when the system killed it (low memory) and then
 * refused the automatic restart. Runs at most every 15 minutes, does nothing
 * when the service is already up or the user turned Phone Bridge off.
 */
class RecoveryJob : JobService() {
    override fun onStartJob(params: JobParameters?): Boolean {
        ensureRunning(this, "job")
        return false
    }

    override fun onStopJob(params: JobParameters?) = false
}

/** Start again after a reboot (an allowed foreground-service start). */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action == Intent.ACTION_BOOT_COMPLETED || intent.action == Intent.ACTION_MY_PACKAGE_REPLACED) {
            ensureRunning(context, intent.action ?: "boot")
        }
    }
}

fun ensureRunning(ctx: Context, why: String) {
    if (honourUserStop(ctx)) return
    if (!Prefs(ctx).enabled || Bridge.running.value) return
    try {
        ctx.startForegroundService(Intent(ctx, PbService::class.java))
        Log.i(TAG, "RECOVERY started service ($why)")
    } catch (t: Throwable) {
        // Android refuses background starts unless the app is exempt from
        // battery optimisation (Settings offers that). Say so once, with a
        // tap that starts it from the foreground.
        Log.i(TAG, "RECOVERY refused ($why): $t")
        notifyStopped(ctx)
    }
}

const val STOPPED_ID = 5

/** "Phone Bridge stopped. Tap to turn it on" (once until it runs again). */
fun notifyStopped(ctx: Context) {
    val prefs = Prefs(ctx)
    if (prefs.stoppedNotified) return
    prefs.stoppedNotified = true
    val nm = ctx.getSystemService(android.app.NotificationManager::class.java)
    nm.createNotificationChannel(
        android.app.NotificationChannel("events", "Files and clipboard", android.app.NotificationManager.IMPORTANCE_DEFAULT),
    )
    val start = android.app.PendingIntent.getActivity(
        ctx, 50, Intent(ctx, StarterActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
        android.app.PendingIntent.FLAG_IMMUTABLE,
    )
    val n = android.app.Notification.Builder(ctx, "events")
        .setSmallIcon(R.drawable.ic_stat_bridge)
        .setContentTitle("$APP_NAME is not running")
        .setContentText("Android stopped it in the background. Tap to turn it on")
        .setContentIntent(start)
        .setAutoCancel(true)
        .build()
    nm.notify(STOPPED_ID, n)
}

/**
 * The user stopped Phone Bridge from the system (realme "Active apps" Stop,
 * Settings force stop) since we last looked: treat it as turning it off, so
 * the tile, the app and recovery agree with the user. Package updates also
 * report "user requested" and are ignored. Returns true if it was stopped.
 */
fun honourUserStop(ctx: Context): Boolean {
    if (android.os.Build.VERSION.SDK_INT < 30) return false
    val prefs = Prefs(ctx)
    val exits = runCatching {
        ctx.getSystemService(android.app.ActivityManager::class.java)
            .getHistoricalProcessExitReasons(ctx.packageName, 0, 5)
    }.getOrNull().orEmpty()
    val newest = exits.maxOfOrNull { it.timestamp } ?: return false
    val seen = prefs.lastExitSeen
    prefs.lastExitSeen = maxOf(seen, newest)
    // First run with this check: only remember where history starts.
    if (seen == 0L) return false
    val stopped = exits.any {
        it.timestamp > seen &&
            it.reason == android.app.ApplicationExitInfo.REASON_USER_REQUESTED &&
            // What realme's "Active apps" Stop reports (verified on the
            // RMX3561); installs and other system stops say something else.
            it.description?.contains("by user request", ignoreCase = true) == true
    }
    if (stopped && prefs.enabled) {
        prefs.enabled = false
        cancelRecovery(ctx)
        ctx.getSystemService(android.app.NotificationManager::class.java).cancel(STOPPED_ID)
        refreshTile(ctx)
        Log.i(TAG, "RECOVERY user stopped $APP_NAME from the system: now off")
    }
    return stopped
}

fun scheduleRecovery(ctx: Context) {
    val js = ctx.getSystemService(JobScheduler::class.java)
    if (js.getPendingJob(JOB_ID) != null) return
    js.schedule(
        JobInfo.Builder(JOB_ID, ComponentName(ctx, RecoveryJob::class.java))
            .setPeriodic(15 * 60 * 1000L)
            .setPersisted(true)
            .build(),
    )
}

fun cancelRecovery(ctx: Context) {
    ctx.getSystemService(JobScheduler::class.java).cancel(JOB_ID)
}

fun isBatteryUnrestricted(ctx: Context): Boolean =
    ctx.getSystemService(PowerManager::class.java).isIgnoringBatteryOptimizations(ctx.packageName)
