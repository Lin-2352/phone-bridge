package dev.phonebridge.app

import android.app.ActivityManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.net.wifi.WifiInfo
import android.os.Build
import android.os.PowerManager
import android.util.Log
import androidx.core.content.FileProvider
import java.io.File
import java.util.TimeZone
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream
import kotlinx.coroutines.delay
import uniffi.pb_ffi.logClearAll
import uniffi.pb_ffi.logDays
import uniffi.pb_ffi.logEvent
import uniffi.pb_ffi.logFlush
import uniffi.pb_ffi.logInit
import uniffi.pb_ffi.logSetEnabled
import uniffi.pb_ffi.logSetTzOffsetSecs
import uniffi.pb_ffi.logSizeBytes

/**
 * The phone's day-by-day log (one folder per day, written by the Rust core).
 * Metadata only: never clipboard text, notification title or body, or file
 * contents; file names only as an extension. Safe to call before [init]
 * (events are dropped) and from any thread.
 */
object AppLog {
    private const val TAG = "PhoneBridge"
    private const val CAP_BYTES = 30L * 1024 * 1024
    @Volatile private var ready = false
    private var tzReceiver: BroadcastReceiver? = null

    /** The log folder: app-specific external storage (readable over adb), else private storage. */
    fun root(ctx: Context): File = ctx.getExternalFilesDir("logs") ?: File(ctx.filesDir, "logs")

    /** Opens the log once per process; installs the crash hook and the time-zone watcher. */
    @Synchronized
    fun init(ctx: Context) {
        if (ready) return
        val app = ctx.applicationContext
        try {
            NativeInit.ensureLoaded()
            val dir = root(app).also { it.mkdirs() }
            if (!logInit(dir.absolutePath, CAP_BYTES.toULong())) return
            ready = true
            applyTz()
            logSetEnabled(Prefs(app).keepLogs)
            val prev = Thread.getDefaultUncaughtExceptionHandler()
            Thread.setDefaultUncaughtExceptionHandler { t, e ->
                runCatching {
                    // Class and first frame only: exception messages can carry user text.
                    val top = e.stackTrace.firstOrNull()
                    event("crash", "crash", "thread" to t.name, "type" to e.javaClass.name, "at" to top?.toString())
                    logFlush()
                }
                prev?.uncaughtException(t, e)
            }
            tzReceiver = object : BroadcastReceiver() {
                override fun onReceive(c: Context, i: Intent) = applyTz()
            }.also { app.registerReceiver(it, IntentFilter(Intent.ACTION_TIMEZONE_CHANGED)) }
            event("events", "app_start", "sdk" to Build.VERSION.SDK_INT)
        } catch (t: Throwable) {
            Log.e(TAG, "LOG init failed: $t")
        }
    }

    /** Day folders follow local time: also called again on a time-zone change. */
    fun applyTz() {
        if (!ready) return
        runCatching { logSetTzOffsetSecs(TimeZone.getDefault().getOffset(System.currentTimeMillis()) / 1000) }
    }

    fun setEnabled(on: Boolean) {
        if (ready) runCatching { logSetEnabled(on) }
    }

    fun event(stream: String, kind: String, vararg fields: Pair<String, Any?>) {
        if (!ready) return
        runCatching { logEvent(stream, kind, LogFields.json(*fields)) }
    }

    fun flush() {
        if (ready) runCatching { logFlush() }
    }

    fun sizeBytes(): Long = if (ready) runCatching { logSizeBytes().toLong() }.getOrDefault(0L) else 0L

    fun dayCount(): Int = if (ready) runCatching { logDays().size }.getOrDefault(0) else 0

    fun clearAll(): Boolean = ready && runCatching { logClearAll() }.getOrDefault(false)

    /** Zips the day folders into the cache and returns a share intent, or null when there is nothing to export. */
    fun exportIntent(ctx: Context): Intent? {
        flush()
        val files = LogFields.exportFiles(root(ctx))
        if (files.isEmpty()) return null
        val dir = File(ctx.cacheDir, "logs-export").also { it.mkdirs() }
        val zip = File(dir, "relay-logs.zip")
        ZipOutputStream(zip.outputStream().buffered()).use { z ->
            for ((name, f) in files) {
                z.putNextEntry(ZipEntry(name))
                f.inputStream().use { it.copyTo(z) }
                z.closeEntry()
            }
        }
        val uri = FileProvider.getUriForFile(ctx, "${ctx.packageName}.clips", zip)
        return Intent.createChooser(
            Intent(Intent.ACTION_SEND).setType("application/zip").putExtra(Intent.EXTRA_STREAM, uri)
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION),
            "Export logs",
        )
    }

    private var lastPerms: String? = null

    /** Logs the permission and access states when they differ from the last time (first call logs them too). */
    fun permissionsChanged(ctx: Context) {
        if (!ready) return
        runCatching {
            fun granted(p: String) = ctx.checkSelfPermission(p) == android.content.pm.PackageManager.PERMISSION_GRANTED
            val fields = arrayOf<Pair<String, Any?>>(
                "notifications" to granted(android.Manifest.permission.POST_NOTIFICATIONS),
                "camera" to granted(android.Manifest.permission.CAMERA),
                "pictures" to ClipImages.hasImageAccess(ctx),
                "notify_access" to hasNotifyAccess(ctx),
                "accessibility" to (ControlService.instance != null),
                "battery_unrestricted" to isBatteryUnrestricted(ctx),
            )
            val key = LogFields.json(*fields)
            if (key != lastPerms) {
                lastPerms = key
                event("events", "permissions", *fields)
            }
        }
    }

    // ---- samples ----

    private var lastNetwork = 0L

    /** One `network` line: radio facts only. SSID needs location access and is never logged; BSSID only when Android hands it over, hashed. */
    fun sampleNetwork(ctx: Context, force: Boolean = false) {
        if (!ready) return
        val now = android.os.SystemClock.elapsedRealtime()
        if (!force && now - lastNetwork < 5_000) return
        lastNetwork = now
        runCatching {
            val cm = ctx.getSystemService(ConnectivityManager::class.java)
            var wifi = false
            var cell = false
            var vpn = false
            var eth = false
            var wi: WifiInfo? = null
            for (n in cm.allNetworks) {
                val c = cm.getNetworkCapabilities(n) ?: continue
                if (c.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)) {
                    wifi = true
                    (c.transportInfo as? WifiInfo)?.let { wi = it }
                }
                if (c.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR)) cell = true
                if (c.hasTransport(NetworkCapabilities.TRANSPORT_VPN)) vpn = true
                if (c.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)) eth = true
            }
            val w = wi
            val freq = w?.frequency?.takeIf { it > 0 }
            event(
                "network", "net",
                "wifi" to wifi, "cell" to cell, "vpn" to vpn, "ethernet" to eth,
                "band" to freq?.let(LogFields::band),
                "freq_mhz" to freq,
                "rssi_dbm" to w?.rssi?.takeIf { it != -127 && it != Int.MIN_VALUE },
                "link_mbps" to w?.linkSpeed?.takeIf { it > 0 },
                "rx_mbps" to w?.rxLinkSpeedMbps?.takeIf { it > 0 },
                "tx_mbps" to w?.txLinkSpeedMbps?.takeIf { it > 0 },
                "bssid" to LogFields.hashBssid(w?.bssid),
            )
        }
    }

    /** One `perf` line: memory and thread count. */
    fun samplePerf(ctx: Context) {
        if (!ready) return
        runCatching {
            val rt = Runtime.getRuntime()
            val mi = ActivityManager.MemoryInfo().also { ctx.getSystemService(ActivityManager::class.java).getMemoryInfo(it) }
            event(
                "perf", "app",
                "heap_mb" to (rt.totalMemory() - rt.freeMemory()) / 1_048_576,
                "native_mb" to android.os.Debug.getNativeHeapAllocatedSize() / 1_048_576,
                "threads" to (File("/proc/self/task").list()?.size ?: 0),
                "avail_mb" to mi.availMem / 1_048_576,
                "low_mem" to mi.lowMemory,
            )
        }
    }

    /** Samples while the service runs: perf every 10 s with the screen on (60 s off), network every 60 s. Ends when the caller's scope is cancelled. */
    suspend fun sampleLoop(ctx: Context) {
        val pm = ctx.getSystemService(PowerManager::class.java)
        var since = 0L
        sampleNetwork(ctx, force = true)
        while (true) {
            val step = if (pm.isInteractive) 10_000L else 60_000L
            delay(step)
            since += step
            samplePerf(ctx)
            if (since >= 60_000L) {
                since = 0
                sampleNetwork(ctx, force = true)
            }
        }
    }
}
