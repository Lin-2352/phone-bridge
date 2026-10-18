package dev.phonebridge.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.BroadcastReceiver
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.os.PersistableBundle
import android.util.Log
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import uniffi.pb_ffi.EventListener
import uniffi.pb_ffi.PbEvent
import uniffi.pb_ffi.PbNode
import uniffi.pb_ffi.startNode

private const val TAG = "PhoneBridge"
private const val PORT = 47100
private const val CH_STATUS = "status"
private const val CH_CLIP = "clipboard"
private const val ONGOING_ID = 1

/** Process-wide handle to the running node, shared by the service and the UI. */
object Bridge {
    @Volatile var node: PbNode? = null
    val listeners = CopyOnWriteArrayList<(PbEvent) -> Unit>()
    /** Last received clipboard texts by notification id (cleared when copied). */
    val pendingClips = java.util.concurrent.ConcurrentHashMap<Int, Pair<String, Boolean>>()
}

/** Hands the Application context to Rust (iroh needs it for system network config). */
object NativeInit {
    init {
        System.loadLibrary("pb_ffi")
    }

    @JvmStatic
    external fun initContext(context: Context)
}

/**
 * Keeps the node reachable while the UI is closed. `connectedDevice` FGS type:
 * no 6-hour limit; one low-priority ongoing notification with a Stop action.
 * Idle cost is network-driven (no polling): events arrive via [EventListener].
 */
class PbService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val nextId = AtomicInteger(100)

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel(CH_STATUS, "Connection status", NotificationManager.IMPORTANCE_MIN))
        nm.createNotificationChannel(NotificationChannel(CH_CLIP, "Received clipboard", NotificationManager.IMPORTANCE_DEFAULT))
        val ongoing = statusNotification("Ready")
        if (Build.VERSION.SDK_INT >= 29) {
            startForeground(ONGOING_ID, ongoing, ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)
        } else {
            startForeground(ONGOING_ID, ongoing)
        }
        scope.launch { start() }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            stopSelf()
            return START_NOT_STICKY
        }
        return START_STICKY
    }

    override fun onDestroy() {
        val n = Bridge.node
        Bridge.node = null
        runBlocking { n?.shutdown() }
        // Release the Rust object (and its UDP socket) now instead of at GC time.
        n?.destroy()
        scope.cancel()
        super.onDestroy()
    }

    private suspend fun start() {
        if (Bridge.node != null) return
        try {
            NativeInit.initContext(applicationContext)
            Bridge.node = startNode(
                filesDir.absolutePath,
                Build.MODEL ?: "Android",
                PORT.toUShort(),
                KeystoreWrapper(),
                object : EventListener {
                    override fun onEvent(event: PbEvent) = handle(event)
                },
            )
            Log.i(TAG, "SERVICE node started id=${Bridge.node?.peerId()}")
        } catch (e: Throwable) {
            Log.e(TAG, "SERVICE failed to start: $e")
        }
    }

    private fun handle(e: PbEvent) {
        when (e) {
            is PbEvent.Clipboard -> notifyClip(e)
            is PbEvent.Connected -> Log.i(TAG, "Connected: ${e.peerName}")
            is PbEvent.Disconnected -> Log.i(TAG, "Disconnected: ${e.peerName} (${e.reason})")
        }
        Bridge.listeners.forEach { it(e) }
    }

    private fun notifyClip(e: PbEvent.Clipboard) {
        Log.i(TAG, if (e.sensitive) "CLIP from ${e.peerName} (sensitive)" else "CLIP from ${e.peerName}: ${e.text}")
        val id = nextId.incrementAndGet()
        Bridge.pendingClips[id] = e.text to e.sensitive
        val copy = PendingIntent.getBroadcast(
            this, id,
            Intent(this, CopyReceiver::class.java).putExtra("id", id),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val n = Notification.Builder(this, CH_CLIP)
            .setSmallIcon(R.drawable.ic_stat_bridge)
            .setContentTitle("Clipboard from ${e.peerName}")
            .setContentText(if (e.sensitive) "Sensitive text (hidden)" else e.text)
            .setVisibility(if (e.sensitive) Notification.VISIBILITY_SECRET else Notification.VISIBILITY_PRIVATE)
            .setAutoCancel(true)
            .addAction(Notification.Action.Builder(null, "Copy", copy).build())
            .setContentIntent(copy)
            .build()
        getSystemService(NotificationManager::class.java).notify(id, n)
    }

    private fun statusNotification(text: String): Notification {
        val stop = PendingIntent.getService(
            this, 0, Intent(this, PbService::class.java).setAction(ACTION_STOP), PendingIntent.FLAG_IMMUTABLE,
        )
        val open = PendingIntent.getActivity(
            this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE,
        )
        return Notification.Builder(this, CH_STATUS)
            .setSmallIcon(R.drawable.ic_stat_bridge)
            .setContentTitle("Phone Bridge")
            .setContentText(text)
            .setOngoing(true)
            .setContentIntent(open)
            .addAction(Notification.Action.Builder(null, "Stop", stop).build())
            .build()
    }

    companion object {
        const val ACTION_STOP = "dev.phonebridge.app.STOP"
    }
}

/** Writes a received clip to the clipboard when the user taps "Copy". */
class CopyReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val id = intent.getIntExtra("id", -1)
        val (text, sensitive) = Bridge.pendingClips.remove(id) ?: return
        val clip = ClipData.newPlainText("Phone Bridge", text)
        if (sensitive) {
            clip.description.extras = PersistableBundle().apply {
                putBoolean("android.content.extra.IS_SENSITIVE", true)
            }
        }
        context.getSystemService(ClipboardManager::class.java).setPrimaryClip(clip)
        context.getSystemService(NotificationManager::class.java).cancel(id)
        Log.i(TAG, "COPIED clip $id to clipboard")
    }
}
