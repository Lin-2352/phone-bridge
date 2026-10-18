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
    if (!Prefs(ctx).enabled || Bridge.running.value) return
    try {
        ctx.startForegroundService(Intent(ctx, PbService::class.java))
        Log.i(TAG, "RECOVERY started service ($why)")
    } catch (t: Throwable) {
        // Android refuses background starts unless the app is exempt from
        // battery optimisation; Settings offers that.
        Log.i(TAG, "RECOVERY refused ($why): $t")
    }
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
