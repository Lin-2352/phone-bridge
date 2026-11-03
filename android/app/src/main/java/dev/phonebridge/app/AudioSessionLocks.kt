package dev.phonebridge.app

import android.content.Context
import android.net.wifi.WifiManager
import android.os.PowerManager

/**
 * Held while audio flows in either direction. Without them the radio naps between beacons and the
 * CPU may sleep with the screen off, and audio stalls for seconds. Same locks as ControlService.
 */
class AudioSessionLocks(ctx: Context) {
    private val app = ctx.applicationContext
    private var wifi: List<WifiManager.WifiLock> = emptyList()
    private var wake: PowerManager.WakeLock? = null

    @Synchronized
    fun acquire() {
        if (wake != null) return
        val wm = app.getSystemService(WifiManager::class.java)
        @Suppress("DEPRECATION")
        wifi = listOf(WifiManager.WIFI_MODE_FULL_LOW_LATENCY, WifiManager.WIFI_MODE_FULL_HIGH_PERF).mapNotNull { mode ->
            runCatching { wm.createWifiLock(mode, "PhoneBridge:audio").apply { setReferenceCounted(false); acquire() } }.getOrNull()
        }
        wake = runCatching {
            app.getSystemService(PowerManager::class.java)
                .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "PhoneBridge:audio")
                .apply { setReferenceCounted(false); acquire() }
        }.getOrNull()
    }

    @Synchronized
    fun release() {
        wifi.forEach { runCatching { if (it.isHeld) it.release() } }
        wifi = emptyList()
        runCatching { wake?.let { if (it.isHeld) it.release() } }
        wake = null
    }
}
