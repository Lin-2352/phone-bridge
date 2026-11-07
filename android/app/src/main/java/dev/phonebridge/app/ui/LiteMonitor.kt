package dev.phonebridge.app.ui

import android.app.ActivityManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.Build
import android.os.PowerManager
import android.provider.Settings
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.compose.runtime.DisposableEffect

/** The phone's current lite signals, read from the system (the power-save state is kept live while the UI shows). */
@Composable
fun rememberLiteSignals(forced: Boolean, slowFrames: Boolean): LiteSignals {
    val context = LocalContext.current
    var powerSave by remember { mutableStateOf(powerSaveOn(context)) }
    DisposableEffect(context) {
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(c: Context, intent: Intent) {
                powerSave = powerSaveOn(c)
            }
        }
        context.registerReceiver(receiver, IntentFilter(PowerManager.ACTION_POWER_SAVE_MODE_CHANGED))
        onDispose { context.unregisterReceiver(receiver) }
    }
    return LiteSignals(
        forced = forced,
        lowRam = activityManager(context)?.isLowRamDevice == true,
        totalMemBytes = totalMemory(context),
        sdk = Build.VERSION.SDK_INT,
        powerSave = powerSave,
        animatorScale = animatorScale(context),
        slowFrames = slowFrames,
    )
}

private fun activityManager(context: Context) =
    context.getSystemService(Context.ACTIVITY_SERVICE) as? ActivityManager

private fun powerSaveOn(context: Context): Boolean =
    (context.getSystemService(Context.POWER_SERVICE) as? PowerManager)?.isPowerSaveMode == true

private fun totalMemory(context: Context): Long {
    val info = ActivityManager.MemoryInfo()
    activityManager(context)?.getMemoryInfo(info)
    return info.totalMem
}

/** The animator duration scale; 0 when the user turned animations off. */
private fun animatorScale(context: Context): Float =
    Settings.Global.getFloat(context.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f)
