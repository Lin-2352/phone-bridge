package dev.phonebridge.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.BroadcastReceiver
import android.content.ClipData
import android.content.ClipboardManager
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.os.IBinder
import android.os.PersistableBundle
import android.provider.DocumentsContract
import android.provider.MediaStore
import android.util.Log
import android.webkit.MimeTypeMap
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import java.io.File
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import uniffi.pb_ffi.EventListener
import uniffi.pb_ffi.PbEvent
import uniffi.pb_ffi.PbNode
import uniffi.pb_ffi.startNode

private const val TAG = "PhoneBridge"
private const val PORT = 47100
private const val CH_STATUS = "status"
private const val CH_EVENTS = "events"
private const val ONGOING_ID = 1

/** One line in the Home screen's Recent list. */
data class RecentItem(val title: String, val subtitle: String, val kind: Kind, val uri: Uri? = null) {
    enum class Kind { File, Clipboard, Link }
}

/** Process-wide handle to the running node, shared by the service and the UI. */
object Bridge {
    @Volatile var node: PbNode? = null
    val listeners = CopyOnWriteArrayList<(PbEvent) -> Unit>()
    /** Compose-observable state for the Home screen. */
    val recent = mutableStateListOf<RecentItem>()
    val linked = mutableStateOf<String?>(null)
    /** True while the service (and node) runs. */
    val running = mutableStateOf(false)
    /** Long-lived scope for sends started from share/tile entry points. */
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    val pendingClips = java.util.concurrent.ConcurrentHashMap<Int, Pair<String, Boolean>>()

    fun addRecent(item: RecentItem) {
        android.os.Handler(android.os.Looper.getMainLooper()).post {
            recent.add(0, item)
            while (recent.size > 20) recent.removeAt(recent.size - 1)
        }
    }

    fun firstPeer() = node?.peers()?.firstOrNull()
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
 * no 6-hour limit. Android requires an ongoing notification; it is minimal
 * priority, silent and has no buttons (the Quick Settings tile turns us off).
 * Idle cost is network-driven (no polling): events arrive via [EventListener].
 */
class PbService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val nextId = AtomicInteger(0)
    /** Network events are handled here, one at a time, never on Rust's runtime threads. */
    private val events = java.util.concurrent.Executors.newSingleThreadExecutor { r -> Thread(r, "pb-events") }
    @Volatile private var destroyed = false

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel(CH_STATUS, "Connection status", NotificationManager.IMPORTANCE_MIN))
        nm.createNotificationChannel(NotificationChannel(CH_EVENTS, "Files and clipboard", NotificationManager.IMPORTANCE_DEFAULT))
        try {
            startForeground(ONGOING_ID, statusNotification(), ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)
        } catch (t: Throwable) {
            // A background restart (after a low-memory kill) may be refused on
            // Android 12+. Stop cleanly instead of crash-looping; RecoveryJob retries.
            Log.e(TAG, "SERVICE could not go foreground: $t")
            destroyed = true
            stopSelf()
            return
        }
        Bridge.running.value = true
        refreshTile(this)
        scheduleRecovery(this)
        scope.launch { start() }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            Prefs(this).enabled = false
            stopSelf()
            return START_NOT_STICKY
        }
        if (intent?.action == ACTION_REFRESH) updateStatus()
        return START_STICKY
    }

    /** Low memory: keep the bare minimum and leave a trace for diagnosis. */
    override fun onTrimMemory(level: Int) {
        super.onTrimMemory(level)
        if (level >= TRIM_MEMORY_RUNNING_LOW) {
            android.os.Handler(mainLooper).post {
                while (Bridge.recent.size > 5) Bridge.recent.removeAt(Bridge.recent.size - 1)
            }
        }
        Log.i(TAG, "TRIM level=$level")
    }

    override fun onDestroy() {
        destroyed = true
        val n = Bridge.node
        Bridge.node = null
        // Never block the main thread on the network: close in the background,
        // bounded so a stuck peer cannot keep the node alive.
        if (n != null) {
            Bridge.scope.launch {
                withTimeoutOrNull(3000) { n.shutdown() }
                n.destroy()
            }
        }
        scope.cancel()
        events.shutdown()
        Bridge.running.value = false
        refreshTile(this)
        super.onDestroy()
    }

    private suspend fun start() {
        if (Bridge.node != null) return
        try {
            NativeInit.initContext(applicationContext)
            val n = startNode(
                filesDir.absolutePath,
                Build.MODEL ?: "Android",
                PORT.toUShort(),
                KeystoreWrapper(),
                object : EventListener {
                    // Called on a Rust runtime thread: hand off and return at once.
                    override fun onEvent(event: PbEvent) {
                        try {
                            events.execute {
                                try {
                                    handle(event)
                                } catch (t: Throwable) {
                                    Log.e(TAG, "EVENT handling failed: $t")
                                }
                            }
                        } catch (_: Throwable) {
                            // Executor shut down: the service is going away.
                        }
                    }
                },
            )
            if (destroyed) {
                // Stopped while starting: do not leave an orphaned node holding the port.
                withTimeoutOrNull(3000) { n.shutdown() }
                n.destroy()
                return
            }
            val prefs = Prefs(this)
            n.setClipboardSync(prefs.clipboardSync)
            n.setAutoAccept(prefs.autoAccept)
            ControlService.instance?.let { n.setInputListener(it) }
            Bridge.node = n
            Bridge.linked.value = n.peers().firstOrNull()?.name
            updateStatus()
            Log.i(TAG, "SERVICE node started id=${n.peerId()}")
        } catch (e: Throwable) {
            // Keystore not ready, port busy...: do not sit in the foreground
            // pretending to work. RecoveryJob retries later.
            Log.e(TAG, "SERVICE failed to start: $e")
            android.os.Handler(mainLooper).post { stopSelf() }
        }
    }

    private fun handle(e: PbEvent) {
        when (e) {
            is PbEvent.Clipboard -> notifyClip(e.peerName, e.text, e.sensitive)
            is PbEvent.RemoteClipboard -> {
                // Share clipboard: writing the clipboard is allowed in the background.
                Prefs(this).lastClip = e.text.hashCode()
                writeClipboard(e.text, e.sensitive)
                Log.i(TAG, "CLIPSYNC received from ${e.peerName}${if (e.sensitive) " (sensitive)" else ""}")
                val title = if (e.sensitive) "Password or code" else e.text.take(80)
                Bridge.addRecent(RecentItem(title, "Copied on ${e.peerName}", RecentItem.Kind.Clipboard))
            }
            is PbEvent.FileReceived -> saveReceived(e)
            is PbEvent.SharedText -> {
                if (e.isLink) notifyLink(e.peerName, e.text) else writeClipboard(e.text, false)
                Bridge.addRecent(RecentItem(e.text.take(80), if (e.isLink) "Link from ${e.peerName}" else "Text from ${e.peerName}", if (e.isLink) RecentItem.Kind.Link else RecentItem.Kind.Clipboard))
            }
            is PbEvent.IncomingOffer -> notifyOffer(e)
            is PbEvent.TransferFailed -> notifySimple("Transfer from ${e.peerName} failed", e.reason)
            is PbEvent.Connected -> Log.i(TAG, "Connected: ${e.peerName}")
            is PbEvent.Disconnected -> Log.i(TAG, "Disconnected: ${e.peerName} (${e.reason})")
        }
        Bridge.listeners.forEach { it(e) }
    }

    private fun writeClipboard(text: String, sensitive: Boolean) {
        val clip = ClipData.newPlainText("Phone Bridge", text)
        if (sensitive) {
            clip.description.extras = PersistableBundle().apply { putBoolean("android.content.extra.IS_SENSITIVE", true) }
        }
        android.os.Handler(mainLooper).post { getSystemService(ClipboardManager::class.java).setPrimaryClip(clip) }
    }

    /** Into the folder the user picked, else Downloads/Phone Bridge. */
    private fun saveReceived(e: PbEvent.FileReceived) {
        val src = File(e.path)
        val mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(src.extension.lowercase()) ?: "application/octet-stream"
        val tree = Prefs(this).saveTree
        if (tree != null) {
            try {
                val dir = DocumentsContract.buildDocumentUriUsingTree(tree, DocumentsContract.getTreeDocumentId(tree))
                val uri = DocumentsContract.createDocument(contentResolver, dir, mime, e.name) ?: error("could not create file")
                contentResolver.openOutputStream(uri)!!.use { out -> src.inputStream().use { it.copyTo(out, 256 * 1024) } }
                src.delete()
                val label = treeLabel(tree)
                Log.i(TAG, "FILE saved ${e.name} (${e.size} bytes) from ${e.peerName} to $label")
                Bridge.addRecent(RecentItem(e.name, "From ${e.peerName}, saved to $label", RecentItem.Kind.File, uri))
                notifyFile(e.peerName, e.name, e.size, uri, mime, label)
                return
            } catch (t: Throwable) {
                // Folder removed or access revoked: fall back to Downloads.
                Log.e(TAG, "FILE save to chosen folder failed: $t")
            }
        }
        saveToDownloads(e, src, mime)
    }

    /** Move a received file into Downloads/Phone Bridge via MediaStore (no storage permission needed). */
    private fun saveToDownloads(e: PbEvent.FileReceived, src: File, mime: String) {
        val values = ContentValues().apply {
            put(MediaStore.Downloads.DISPLAY_NAME, e.name)
            put(MediaStore.Downloads.MIME_TYPE, mime)
            put(MediaStore.Downloads.RELATIVE_PATH, Environment.DIRECTORY_DOWNLOADS + "/Phone Bridge")
            put(MediaStore.Downloads.IS_PENDING, 1)
        }
        val uri = try {
            contentResolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
        } catch (t: Throwable) {
            null
        }
        if (uri == null) {
            src.delete()
            notifySimple("Could not save ${e.name}", "Storage is not available")
            return
        }
        try {
            contentResolver.openOutputStream(uri)!!.use { out -> src.inputStream().use { it.copyTo(out, 256 * 1024) } }
            contentResolver.update(uri, ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }, null, null)
            src.delete()
            Log.i(TAG, "FILE saved ${e.name} (${e.size} bytes) from ${e.peerName}")
            Bridge.addRecent(RecentItem(e.name, "From ${e.peerName}, saved to Downloads", RecentItem.Kind.File, uri))
            notifyFile(e.peerName, e.name, e.size, uri, mime, "Downloads/Phone Bridge")
        } catch (t: Throwable) {
            runCatching { contentResolver.delete(uri, null, null) }
            src.delete()
            notifySimple("Could not save ${e.name}", t.message ?: "")
        }
    }

    private fun notifyFile(from: String, name: String, size: ULong, uri: Uri, mime: String, folder: String) {
        val id = nextNotificationId()
        val open = PendingIntent.getActivity(
            this, id,
            Intent(Intent.ACTION_VIEW).setDataAndType(uri, mime).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK),
            PendingIntent.FLAG_IMMUTABLE,
        )
        val mb = size.toDouble() / 1_048_576
        val n = Notification.Builder(this, CH_EVENTS)
            .setSmallIcon(R.drawable.ic_stat_bridge)
            .setContentTitle("$from sent $name")
            .setContentText("%.1f MB, saved to %s".format(mb, folder))
            .setContentIntent(open)
            .setAutoCancel(true)
            .build()
        getSystemService(NotificationManager::class.java).notify(id, n)
    }

    private fun notifyLink(from: String, link: String) {
        val id = nextNotificationId()
        val open = PendingIntent.getActivity(
            this, id, Intent(Intent.ACTION_VIEW, Uri.parse(link)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK), PendingIntent.FLAG_IMMUTABLE,
        )
        val n = Notification.Builder(this, CH_EVENTS)
            .setSmallIcon(R.drawable.ic_stat_bridge)
            .setContentTitle("Link from $from")
            .setContentText(link)
            .setContentIntent(open)
            .setAutoCancel(true)
            .build()
        getSystemService(NotificationManager::class.java).notify(id, n)
    }

    private fun notifyOffer(e: PbEvent.IncomingOffer) {
        val id = nextNotificationId()
        fun action(accept: Boolean) = PendingIntent.getBroadcast(
            this, id * 2 + if (accept) 1 else 0,
            Intent(this, OfferReceiver::class.java).putExtra("offer", e.id.toLong()).putExtra("accept", accept).putExtra("nid", id),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val mb = e.size.toDouble() / 1_048_576
        val n = Notification.Builder(this, CH_EVENTS)
            .setSmallIcon(R.drawable.ic_stat_bridge)
            .setContentTitle("${e.peerName} wants to send ${e.name}")
            .setContentText("%.1f MB".format(mb))
            .addAction(Notification.Action.Builder(null, "Decline", action(false)).build())
            .addAction(Notification.Action.Builder(null, "Accept", action(true)).build())
            .setAutoCancel(true)
            .build()
        getSystemService(NotificationManager::class.java).notify(id, n)
    }

    private fun notifyClip(from: String, text: String, sensitive: Boolean) {
        Log.i(TAG, "CLIP from $from (${text.length} chars${if (sensitive) ", sensitive" else ""})")
        val id = nextNotificationId()
        Bridge.pendingClips[id] = text to sensitive
        // Swiped-away clips are never claimed: keep only the newest few.
        while (Bridge.pendingClips.size > 10) Bridge.pendingClips.keys.minOrNull()?.let { Bridge.pendingClips.remove(it) }
        val copy = PendingIntent.getBroadcast(
            this, id, Intent(this, CopyReceiver::class.java).putExtra("id", id),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val n = Notification.Builder(this, CH_EVENTS)
            .setSmallIcon(R.drawable.ic_stat_bridge)
            .setContentTitle("Clipboard from $from")
            .setContentText(if (sensitive) "Sensitive text (hidden)" else text)
            .setVisibility(if (sensitive) Notification.VISIBILITY_SECRET else Notification.VISIBILITY_PRIVATE)
            .setAutoCancel(true)
            .addAction(Notification.Action.Builder(null, "Copy", copy).build())
            .setContentIntent(copy)
            .build()
        getSystemService(NotificationManager::class.java).notify(id, n)
    }

    /** Ids 100..139, reused: at most 40 of our notifications exist at once. */
    private fun nextNotificationId() = 100 + Math.floorMod(nextId.getAndIncrement(), 40)

    private fun notifySimple(title: String, text: String) {
        val n = Notification.Builder(this, CH_EVENTS)
            .setSmallIcon(R.drawable.ic_stat_bridge)
            .setContentTitle(title)
            .setContentText(text)
            .setAutoCancel(true)
            .build()
        getSystemService(NotificationManager::class.java).notify(nextNotificationId(), n)
    }

    /** One quiet line: who we are linked with. No buttons; tap opens the app. */
    private fun statusNotification(): Notification {
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
        val peer = Bridge.linked.value
        return Notification.Builder(this, CH_STATUS)
            .setSmallIcon(R.drawable.ic_stat_bridge)
            .setContentTitle(if (peer != null) "Linked with $peer" else "Not paired yet")
            .setOngoing(true)
            .setShowWhen(false)
            .setContentIntent(open)
            .build()
    }

    private fun updateStatus() {
        getSystemService(NotificationManager::class.java).notify(ONGOING_ID, statusNotification())
        refreshTile(this)
    }

    companion object {
        const val ACTION_STOP = "dev.phonebridge.app.STOP"
        private const val ACTION_REFRESH = "dev.phonebridge.app.REFRESH"

        /** Pairing changed: refresh the status line and the tile. */
        fun refresh(ctx: Context) {
            if (Bridge.running.value) ctx.startService(Intent(ctx, PbService::class.java).setAction(ACTION_REFRESH))
        }
    }
}

/** Writes a received clip to the clipboard when the user taps "Copy". */
class CopyReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val id = intent.getIntExtra("id", -1)
        val (text, sensitive) = Bridge.pendingClips.remove(id) ?: return
        val clip = ClipData.newPlainText("Phone Bridge", text)
        if (sensitive) {
            clip.description.extras = PersistableBundle().apply { putBoolean("android.content.extra.IS_SENSITIVE", true) }
        }
        context.getSystemService(ClipboardManager::class.java).setPrimaryClip(clip)
        context.getSystemService(NotificationManager::class.java).cancel(id)
        Log.i(TAG, "COPIED clip $id to clipboard")
    }
}

/** Accept/Decline buttons on an incoming-file notification. */
class OfferReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val offer = intent.getLongExtra("offer", -1)
        val accept = intent.getBooleanExtra("accept", false)
        Bridge.node?.answerOffer(offer.toULong(), accept)
        context.getSystemService(NotificationManager::class.java).cancel(intent.getIntExtra("nid", 0))
        Log.i(TAG, "OFFER $offer answered accept=$accept")
    }
}
