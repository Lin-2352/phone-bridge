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
import dev.phonebridge.app.widget.RelayWidget

private const val TAG = "PhoneBridge"
// Negative: WorkManager (used by the Glance widget) numbers its JobScheduler jobs from 0 upwards and
// would in time reuse a positive id, replacing this job with its own.
private const val JOB_ID = -4701
private const val LEGACY_JOB_ID = 4701

/**
 * Brings Phone Bridge back when the system killed it (low memory) and then
 * refused the automatic restart. Runs at most every 15 minutes, does nothing
 * when the service is already up or the user turned Phone Bridge off.
 */
class RecoveryJob : JobService() {
    override fun onStartJob(params: JobParameters?): Boolean {
        ensureRunning(this, "job")
        HelperStarter.maintain(this)
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
    // A fresh process (the job or boot woke it): the widget may still show the dead process's "Connected".
    if (!Bridge.running.value) RelayWidget.update(ctx)
    if (honourUserStop(ctx)) {
        // A forced stop can land mid-share, and no service will run to put the volume back.
        PhoneSpeaker.restore(ctx)
        return
    }
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
        // Killed without onDestroy (OEM task manager): the widget may still show the old state.
        RelayWidget.update(ctx)
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
    val exits = exitRecords(ctx) ?: return false
    val v = userStopVerdict(prefs.lastExitSeen, exits)
    prefs.lastExitSeen = v.newSeen
    val stopped = v.stopped
    if (stopped && prefs.enabled) {
        prefs.enabled = false
        cancelRecovery(ctx)
        ctx.getSystemService(android.app.NotificationManager::class.java).cancel(STOPPED_ID)
        refreshTile(ctx)
        RelayWidget.update(ctx)
        Log.i(TAG, "RECOVERY user stopped $APP_NAME from the system: now off")
    }
    return stopped
}

/**
 * The user turned Relay on themselves (tile, panel, buttons, notification): any earlier system "Stop"
 * is history now. Without this the next [honourUserStop] reads that old exit as new and switches Relay off again.
 */
fun consumeUserStop(ctx: Context) {
    if (android.os.Build.VERSION.SDK_INT < 30) return
    val exits = exitRecords(ctx) ?: return
    val prefs = Prefs(ctx)
    prefs.lastExitSeen = userStopVerdict(prefs.lastExitSeen, exits, consume = true).newSeen
}

@androidx.annotation.RequiresApi(30)
private fun exitRecords(ctx: Context): List<ExitRecord>? {
    val exits = runCatching {
        ctx.getSystemService(android.app.ActivityManager::class.java)
            .getHistoricalProcessExitReasons(ctx.packageName, 0, 5)
    }.getOrNull().orEmpty()
    if (exits.isEmpty()) return null
    return exits.map {
        ExitRecord(
            it.timestamp,
            it.reason == android.app.ApplicationExitInfo.REASON_USER_REQUESTED &&
                // What realme's "Active apps" Stop reports (verified on the
                // RMX3561); installs and other system stops say something else.
                it.description?.contains("by user request", ignoreCase = true) == true,
        )
    }
}

class ExitRecord(val timestamp: Long, val byUserRequest: Boolean)
class UserStopVerdict(val newSeen: Long, val stopped: Boolean)

/**
 * [seen]: newest exit already looked at (0 = never checked). Returns the new mark and whether a
 * by-user-request exit is newer than [seen]. [consume]: only advance the mark, never report a stop.
 */
fun userStopVerdict(seen: Long, exits: List<ExitRecord>, consume: Boolean = false): UserStopVerdict {
    val newest = exits.maxOfOrNull { it.timestamp } ?: return UserStopVerdict(seen, false)
    val newSeen = maxOf(seen, newest)
    // First run with this check: only remember where history starts.
    if (seen == 0L || consume) return UserStopVerdict(newSeen, false)
    return UserStopVerdict(newSeen, exits.any { it.timestamp > seen && it.byUserRequest })
}

fun scheduleRecovery(ctx: Context) {
    val js = ctx.getSystemService(JobScheduler::class.java)
    cancelLegacyRecovery(js)
    if (js.getPendingJob(JOB_ID) != null) return
    js.schedule(
        JobInfo.Builder(JOB_ID, ComponentName(ctx, RecoveryJob::class.java))
            .setPeriodic(15 * 60 * 1000L)
            .setPersisted(true)
            .build(),
    )
}

fun cancelRecovery(ctx: Context) {
    val js = ctx.getSystemService(JobScheduler::class.java)
    js.cancel(JOB_ID)
    cancelLegacyRecovery(js)
}

/** The job as older versions scheduled it; only if it is still ours (WorkManager may own that id now). */
private fun cancelLegacyRecovery(js: JobScheduler) {
    if (js.getPendingJob(LEGACY_JOB_ID)?.service?.className == RecoveryJob::class.java.name) js.cancel(LEGACY_JOB_ID)
}

fun isBatteryUnrestricted(ctx: Context): Boolean =
    ctx.getSystemService(PowerManager::class.java).isIgnoringBatteryOptimizations(ctx.packageName)
