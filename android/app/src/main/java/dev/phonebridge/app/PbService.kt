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
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import dev.phonebridge.app.widget.RelayWidget
import dev.phonebridge.app.widget.widgetConnected
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
private const val CH_PROGRESS = "progress"
private const val PROGRESS_IN_ID = 2
private const val PROGRESS_OUT_ID = 3
private const val CH_CONTROL = "control"
const val CONTROL_ASK_ID = 4
const val SCREEN_ASK_ID = 6
const val SCREEN_SHARE_ID = 7
const val CAMERA_ASK_ID = 8
const val CAMERA_SHARE_ID = 9

/** Process-wide handle to the running node, shared by the service and the UI. */
object Bridge {
    @Volatile var node: PbNode? = null
    val listeners = CopyOnWriteArrayList<(PbEvent) -> Unit>()
    /** Compose-observable state for the Home screen. */
    val linked = mutableStateOf<String?>(null)
    /** True while the service (and node) runs. */
    val running = mutableStateOf(false)
    /** When a start was requested (uptime ms), 0 if none is pending. The
     *  service starts asynchronously; until it is up, the UI shows "starting". */
    @Volatile var startingSince = 0L

    /** What the UI should show: running, or a start requested in the last 10 s. */
    fun shownOn(): Boolean =
        running.value || (startingSince != 0L && android.os.SystemClock.uptimeMillis() - startingSince < 10_000)
    /** Long-lived scope for sends started from share/tile entry points. */
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    /** Clips waiting for "Copy", oldest first (notification ids wrap, so order is by insertion). */
    val pendingClips = java.util.LinkedHashMap<Int, Pair<String, Boolean>>()
    /** Pairing code check in progress (survives screen rotation). */
    val sas = mutableStateOf<String?>(null)
    val pairMessage = mutableStateOf<String?>(null)
    /** The node is up (until then the UI says "Starting", not "Not paired"). */
    val nodeReady = mutableStateOf(false)
    /** Name of the PC controlling this phone right now, or null. */
    val controlledBy = mutableStateOf<String?>(null)
    /** Mirrors [Prefs.allowControl] for the UI. */
    val allowControl = mutableStateOf(false)
    /** Sessions from the PC open right now (Connected minus Disconnected events); 0 when the node stops. */
    val liveSessions = mutableIntStateOf(0)
    /** When the PC was last in contact (wall-clock ms), 0 if never; persisted as [Prefs.lastSeenMs]. */
    val lastSeen = mutableLongStateOf(0L)

    /** The PC opened or closed a session: count it and stamp the contact time. */
    fun pcContact(ctx: Context, opened: Boolean) {
        liveSessions.intValue = if (opened) liveSessions.intValue + 1 else (liveSessions.intValue - 1).coerceAtLeast(0)
        stampSeen(ctx)
        // Notification, tile and widget say "Connected" or "last seen" from this; redraw all three.
        val svc = PbService.instance
        if (svc != null) svc.updateStatus() else { refreshTile(ctx); RelayWidget.update(ctx) }
    }

    /** The newest contact stamp: this process's, or the saved one (a fresh process starts at 0). */
    fun lastSeenAny(ctx: Context): Long = maxOf(lastSeen.longValue, Prefs(ctx).lastSeenMs)

    /**
     * The one status answer for notification, tile, panel and widget (see [relayStatus]).
     * [serviceUp]: true from inside the service, which is running although [running] is set a moment later.
     */
    fun status(ctx: Context, serviceUp: Boolean = false): RelayStatus {
        val up = running.value || serviceUp
        return relayStatus(up, nodeReady.value, linked.value, isConnected(), lastSeenAny(ctx), System.currentTimeMillis(), controlledBy.value)
    }

    /** A PC session is live right now (Home's "Connected"). */
    fun isConnected(): Boolean =
        widgetConnected(running.value, linked.value, liveSessions.intValue, controlledBy.value, ScreenShare.sharingWith.value, CameraShare.usedBy.value, AudioShare.activePeer())

    /** The PC was in contact just now. */
    fun stampSeen(ctx: Context) {
        val now = System.currentTimeMillis()
        lastSeen.longValue = now
        Prefs(ctx).lastSeenMs = now
    }

    /** Allow PC control without asking, or ask each time; off also ends a running session. */
    fun setAllowControl(ctx: Context, on: Boolean) {
        val prefs = Prefs(ctx)
        prefs.allowControl = on
        allowControl.value = prefs.allowControl
        // Which PC may skip the question is decided here, per PC (askControl); the node always asks us.
        node?.setAllowControl(false)
        if (!on) node?.stopControl()
    }

    /** The previous node's shutdown; a new node waits for it so the ports are free. */
    @Volatile var stopping: kotlinx.coroutines.Job? = null

    fun firstPeer() = node?.peers()?.firstOrNull()

    /** The node, waiting up to [ms] for a service that is just starting. */
    suspend fun awaitNode(ms: Long = 10_000): PbNode? {
        repeat((ms / 100).toInt()) {
            node?.let { return it }
            kotlinx.coroutines.delay(100)
        }
        return node
    }

    /** Pairing, step 1: join with the scanned code; the code check shows next. */
    suspend fun join(code: String) {
        val n = awaitNode() ?: run { pairMessage.value = "$APP_NAME is off"; return }
        try {
            val s = n.joinPairing(code.trim())
            Log.i(TAG, "SAS shown (${s.length} digits)")
            AppLog.event("events", "pair_step", "step" to "code_shown")
            sas.value = s
        } catch (e: Throwable) {
            Log.i(TAG, "Pairing failed: ${e.message}")
            AppLog.event("events", "pair_step", "step" to "join_failed", "type" to e.javaClass.simpleName)
            pairMessage.value = "Pairing failed: ${e.message}"
        }
    }

    /** Pairing, step 2: the user compared the codes. */
    suspend fun confirm(ctx: Context, accept: Boolean) {
        sas.value = null
        AppLog.event("events", "pair_step", "step" to if (accept) "confirmed" else "rejected")
        val n = node ?: run { pairMessage.value = "$APP_NAME stopped. Turn it on and pair again"; return }
        try {
            val name = n.confirmPairing(accept)
            Log.i(TAG, "PAIRED with $name")
            AppLog.event("events", "pair_step", "step" to "paired")
            linked.value = name
            // Pairing is contact too: Home says "Last seen just now", not "Not seen yet".
            stampSeen(ctx)
            PbService.refresh(ctx)
        } catch (e: Throwable) {
            Log.i(TAG, "Pairing not completed: ${e.message}")
            AppLog.event("events", "pair_step", "step" to "not_completed", "type" to e.javaClass.simpleName)
            pairMessage.value = "Pairing not completed"
        }
    }
}

/** Hands the Application context to Rust (iroh needs it for system network config). */
object NativeInit {
    init {
        System.loadLibrary("pb_ffi")
    }

    /** Touching this object loads the native library (the log needs it before the node starts). */
    fun ensureLoaded() = Unit

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
    private var netCallback: android.net.ConnectivityManager.NetworkCallback? = null
    private var rediscoverJob: kotlinx.coroutines.Job? = null

    /** New Wi-Fi or new address: find the paired PC again (debounced). */
    private fun rediscoverSoon() {
        rediscoverJob?.cancel()
        rediscoverJob = scope.launch {
            kotlinx.coroutines.delay(2000)
            val n = Bridge.node ?: return@launch
            runCatching { n.rediscover() }
            Log.i(TAG, "DISCOVERY probe sent")
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        instance = this
        // Foreground first: the start deadline is short, and the log, timeline and stale-notice work below is not needed for it.
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel(CH_STATUS, "Connection status", NotificationManager.IMPORTANCE_MIN))
        nm.createNotificationChannel(NotificationChannel(CH_EVENTS, "Files and clipboard", NotificationManager.IMPORTANCE_DEFAULT))
        nm.createNotificationChannel(NotificationChannel(CH_PROGRESS, "Transfer progress", NotificationManager.IMPORTANCE_LOW))
        nm.createNotificationChannel(NotificationChannel(CH_CONTROL, "Control requests", NotificationManager.IMPORTANCE_HIGH))
        try {
            startForeground(ONGOING_ID, statusNotification(), ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)
        } catch (t: Throwable) {
            // A background restart (after a low-memory kill) may be refused on
            // Android 12+. Stop cleanly instead of crash-looping; RecoveryJob retries.
            AppLog.init(this)
            Log.e(TAG, "SERVICE could not go foreground: $t")
            AppLog.event("events", "service_refused", "type" to t.javaClass.simpleName)
            Bridge.startingSince = 0L
            destroyed = true
            stopSelf()
            return
        }
        AppLog.init(this)
        AppLog.event("events", "service_start")
        Timeline.init(this)
        PhoneSpeaker.restore(this) // a crash while streaming must not leave the phone muted
        Prefs(this).audioHelperSecret()
        HelperStarter.maintain(this) // after a reboot or an update: nothing for the user to do
        SpeakerSilencer.registerDebug(this)
        // A new process shares nothing yet: drop the old process's ongoing notifications.
        clearStaleSharing()
        // Bars from a process that was killed mid-transfer never end by themselves.
        nm.cancel(PROGRESS_IN_ID)
        nm.cancel(PROGRESS_OUT_ID)
        Bridge.running.value = true
        Bridge.startingSince = 0L
        // Running now: an earlier system "Stop" must not read as new and switch it off again.
        consumeUserStop(this)
        Prefs(this).stoppedNotified = false
        getSystemService(NotificationManager::class.java).cancel(STOPPED_ID)
        refreshTile(this)
        RelayWidget.update(this)
        scheduleRecovery(this)
        scope.launch { start() }
        scope.launch { AppLog.sampleLoop(this@PbService) }
        // "<PC> not reachable, last seen 5 min ago" only moves when redrawn: do it while the PC is away.
        scope.launch {
            while (true) {
                kotlinx.coroutines.delay(STATUS_REDRAW_MS)
                if (Bridge.nodeReady.value && !Bridge.isConnected()) updateStatus()
            }
        }
        AppLog.permissionsChanged(this)
        val cb = object : android.net.ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: android.net.Network) {
                AppLog.sampleNetwork(this@PbService)
                rediscoverSoon()
            }
            override fun onLinkPropertiesChanged(network: android.net.Network, lp: android.net.LinkProperties) {
                AppLog.sampleNetwork(this@PbService)
                rediscoverSoon()
            }
            override fun onLost(network: android.net.Network) = AppLog.sampleNetwork(this@PbService)
        }
        runCatching { getSystemService(android.net.ConnectivityManager::class.java).registerDefaultNetworkCallback(cb) }
            .onSuccess { netCallback = cb }
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
        // Memory is really short while running (not just our own UI going to
        // the background, level 20+): screen sharing steps down.
        if (level == android.content.ComponentCallbacks2.TRIM_MEMORY_RUNNING_LOW ||
            level == android.content.ComponentCallbacks2.TRIM_MEMORY_RUNNING_CRITICAL
        ) {
            ScreenShare.capture?.memoryPressure()
        }
        super.onTrimMemory(level)
        Log.i(TAG, "TRIM level=$level")
    }

    override fun onDestroy() {
        if (instance === this) instance = null
        AppLog.event("events", "service_stop")
        AppLog.flush()
        ScreenshotWatcher.stop(this)
        ScreenShare.stop(userStopped = true)
        // The webcam goes with the service: its camera, encoder and "in use" notice must not outlive "Relay is off".
        if (CameraShare.capture != null || CameraShare.usedBy.value != null) CameraShare.stop(userStopped = true)
        AudioShare.stopAll()
        // The speaker comes back even if the stop path above could not reach it (instance is already null).
        PhoneSpeaker.restore(this)
        ControlService.instance?.endForServiceStop()
        val n = synchronized(Bridge) {
            destroyed = true
            Bridge.node.also { Bridge.node = null }
        }
        // Never block the main thread on the network: close in the background,
        // bounded so a stuck peer cannot keep the node alive.
        val previous = Bridge.stopping
        Bridge.stopping = Bridge.scope.launch {
            previous?.join()
            if (n != null) {
                withTimeoutOrNull(3000) { n.shutdown() }
                n.destroy()
            }
            // Events still queued (a file that just arrived) are handled; the
            // next start (which empties incoming/) waits for this.
            events.shutdown()
            runCatching { kotlinx.coroutines.runInterruptible { events.awaitTermination(60, java.util.concurrent.TimeUnit.SECONDS) } }
        }
        Bridge.nodeReady.value = false
        Bridge.liveSessions.intValue = 0
        cancelProgress(true)
        cancelProgress(false)
        netCallback?.let { cb -> runCatching { getSystemService(android.net.ConnectivityManager::class.java).unregisterNetworkCallback(cb) } }
        netCallback = null
        scope.cancel()
        Bridge.running.value = false
        Bridge.startingSince = 0L
        refreshTile(this)
        RelayWidget.update(this)
        // Quitting right after a transfer: its timeline entry is still waiting
        // for the delayed write. Write it now, bounded (at most 500 ms).
        Timeline.flush(500)
        super.onDestroy()
    }

    private suspend fun start() {
        if (Bridge.node != null) return
        // Turned off and on again quickly: the old node still holds the ports.
        Bridge.stopping?.join()
        // A new node has no sessions yet (the old node's last events are handled by now).
        Bridge.liveSessions.intValue = 0
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
            val prefs = Prefs(this)
            n.setClipboardSync(prefs.clipboardSync)
            n.setClipboardImages(prefs.clipboardImages)
            n.setAutoAccept(prefs.autoAccept)
            prefs.migrateAllowances(n.peers().map { it.id })
            n.setAllowControl(false) // per-PC "Always allow" is applied in askControl
            n.setNotifyEnabled(prefs.notifyToPc)
            n.setNotifyHandler(NotifyActions)
            n.setAudioSink(AudioShare.sink)
            Bridge.allowControl.value = prefs.allowControl
            ControlService.instance?.let { n.setInputListener(it) }
            // Published under the same lock onDestroy takes, so a stop racing
            // this start either sees the node or makes us drop it.
            val orphan = synchronized(Bridge) {
                if (destroyed) true else { Bridge.node = n; false }
            }
            if (orphan) {
                // Our scope is being cancelled: close on the long-lived one, and
                // let the next start wait for it (the ports must be free).
                val previous = Bridge.stopping
                Bridge.stopping = Bridge.scope.launch {
                    previous?.join()
                    withTimeoutOrNull(3000) { n.shutdown() }
                    n.destroy()
                }
                return
            }
            // The accessibility service may have connected while we started.
            ControlService.instance?.let { n.setInputListener(it) }
            // So the node knows what is already on the phone (sync with PCs).
            NotifyListener.instance?.seedAll()
            // A call silenced from the PC ended while Phone Bridge was not running.
            if (NotifyListener.live.values.none { it.notification.category == android.app.Notification.CATEGORY_CALL }) {
                restoreRinger(this)
            }
            Bridge.linked.value = n.peers().firstOrNull()?.name
            Bridge.nodeReady.value = true
            AppLog.event("events", "node_ready", "paired" to n.peers().size)
            ScreenshotWatcher.sync(this)
            updateStatus()
            Log.i(TAG, "SERVICE node started id=${n.peerId()}")
            runCatching { EncoderCaps.get() }.onSuccess { c ->
                Log.i(TAG, "SCREEN encoder caps ${c.encoderName} codecs=${c.codecs} vbr=${c.vbr} qpMax=${c.qpMax} align=${c.alignW}x${c.alignH}")
            }
            rediscoverSoon()
        } catch (e: Throwable) {
            // Keystore not ready, port busy...: do not sit in the foreground
            // pretending to work. RecoveryJob retries later.
            Log.e(TAG, "SERVICE failed to start: $e")
            AppLog.event("events", "node_failed", "type" to e.javaClass.simpleName)
            android.os.Handler(mainLooper).post { stopSelf() }
        }
    }

    /** Question id -> notification id still on screen for it (file offers get a new id each). */
    private val askNotes = java.util.concurrent.ConcurrentHashMap<ULong, Int>()

    /** Answered (from a notification button or a window): AskEnded will not come for it. */
    fun askAnswered(id: ULong) { askNotes.remove(id) }

    /** The PC gave up on question `id` before it was answered (or the time ran out). */
    private fun askEnded(id: ULong) {
        val note = askNotes.remove(id) ?: return
        getSystemService(NotificationManager::class.java).cancel(note)
        when (note) {
            CONTROL_ASK_ID -> ControlRequestActivity.answeredElsewhere(id.toLong())
            SCREEN_ASK_ID -> ScreenConsentActivity.withdrawn(id)
            CAMERA_ASK_ID -> CameraConsentActivity.withdrawn(id)
        }
        Log.i(TAG, "ASK $id ended without an answer")
    }

    private fun handle(e: PbEvent) {
        // Late events from a node that is shutting down: no progress bar or
        // offer that nothing can update or answer any more. A file that
        // already arrived is still saved.
        if (destroyed && (e is PbEvent.TransferProgress || e is PbEvent.IncomingOffer || e is PbEvent.ControlRequest)) return
        when (e) {
            is PbEvent.ScreenRequest -> askScreen(e)
            is PbEvent.CameraRequest -> askCamera(e)
            is PbEvent.AudioRequest -> askAudio(e)
            is PbEvent.AskEnded -> askEnded(e.id)
            is PbEvent.AudioAskEnded, is PbEvent.AudioStarted, is PbEvent.AudioEnded, is PbEvent.AudioRefused -> AudioShare.onEvent(e)
            is PbEvent.Clipboard -> notifyClip(e.peerName, e.text, e.sensitive)
            is PbEvent.RemoteClipboard -> {
                // Share clipboard: writing the clipboard is allowed in the background.
                Prefs(this).lastClip = e.text.hashCode()
                writeClipboard(e.text, e.sensitive)
                Log.i(TAG, "CLIPSYNC received from ${e.peerName}${if (e.sensitive) " (sensitive)" else ""}")
                Timeline.add(clipTextEntry(e.peerName, e.text, TlDir.ToPhone))
            }
            is PbEvent.RemoteImage -> {
                // Share copied images: a picture copied on the PC goes on the clipboard.
                // Size and dimensions first: applying moves the file.
                val src = File(e.path)
                val size = src.length()
                val dims = runCatching { java.io.DataInputStream(src.inputStream()).use { d -> ByteArray(24).also { d.readFully(it) } } }
                    .getOrNull()?.let(::pngDims)
                ClipImages.applyReceived(this, e.path)
                Log.i(TAG, "CLIPIMG received from ${e.peerName}")
                Timeline.add(clipImageEntry(e.peerName, TlDir.ToPhone, size, dims))
            }
            is PbEvent.FileReceived -> {
                cancelProgress(true)
                saveReceived(e)
            }
            is PbEvent.SharedText -> {
                if (e.isLink) notifyLink(e.peerName, e.text) else {
                    writeClipboard(e.text, false)
                    // The clipboard was just replaced: say so (F14).
                    notifyInfo(this, "Text from ${e.peerName} copied", "It is on this phone's clipboard. Paste it anywhere")
                }
                Timeline.add(if (e.isLink) linkEntry(e.peerName, e.text, TlDir.ToPhone) else clipTextEntry(e.peerName, e.text, TlDir.ToPhone))
            }
            is PbEvent.IncomingOffer -> notifyOffer(e)
            is PbEvent.ControlRequest -> askControl(e)
            is PbEvent.TransferFailed -> {
                cancelProgress(true)
                if (isCancelled(e.reason)) notifySimple("Transfer cancelled", e.reason.replaceFirstChar { it.uppercase() })
                else notifySimple("Transfer from ${e.peerName} failed", e.reason)
            }
            is PbEvent.TransferProgress -> notifyProgress(e)
            is PbEvent.Connected -> {
                Log.i(TAG, "Connected: ${e.peerName}")
                Bridge.pcContact(this, opened = true)
            }
            is PbEvent.Disconnected -> {
                Log.i(TAG, "Disconnected: ${e.peerName} (${e.reason})")
                Bridge.pcContact(this, opened = false)
            }
        }
        Bridge.listeners.forEach { it(e) }
    }

    private fun writeClipboard(text: String, sensitive: Boolean) {
        val clip = ClipData.newPlainText("$APP_NAME", text)
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
            var created: Uri? = null
            try {
                val dir = DocumentsContract.buildDocumentUriUsingTree(tree, DocumentsContract.getTreeDocumentId(tree))
                val uri = DocumentsContract.createDocument(contentResolver, dir, mime, e.name) ?: error("could not create file")
                created = uri
                contentResolver.openOutputStream(uri)!!.use { out -> src.inputStream().use { it.copyTo(out, 256 * 1024) } }
                src.delete()
                val label = treeLabel(tree)
                Log.i(TAG, "FILE saved ext=${LogFields.extOnly(e.name)} (${e.size} bytes)")
                Timeline.add(fileEntry(e.peerName, TlDir.ToPhone, e.name, e.size.toLong(), uri.toString()))
                notifyFile(e.peerName, e.name, e.size, uri, mime, label)
                return
            } catch (t: Throwable) {
                // Folder removed, access revoked or full: fall back to Downloads,
                // leaving no half-written copy behind.
                Log.e(TAG, "FILE save to chosen folder failed: $t")
                created?.let { runCatching { DocumentsContract.deleteDocument(contentResolver, it) } }
            }
        }
        saveToDownloads(e, src, mime)
    }

    /** Move a received file into Downloads/Phone Bridge via MediaStore (no storage permission needed). */
    private fun saveToDownloads(e: PbEvent.FileReceived, src: File, mime: String) {
        val values = ContentValues().apply {
            put(MediaStore.Downloads.DISPLAY_NAME, e.name)
            put(MediaStore.Downloads.MIME_TYPE, mime)
            put(MediaStore.Downloads.RELATIVE_PATH, Environment.DIRECTORY_DOWNLOADS + "/$APP_NAME")
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
            Log.i(TAG, "FILE saved ext=${LogFields.extOnly(e.name)} (${e.size} bytes)")
            Timeline.add(fileEntry(e.peerName, TlDir.ToPhone, e.name, e.size.toLong(), uri.toString()))
            notifyFile(e.peerName, e.name, e.size, uri, mime, "Downloads/$APP_NAME")
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
            // The sender gives up after 2 minutes; so does the question.
            .setTimeoutAfter(120_000)
            .setAutoCancel(true)
            .build()
        getSystemService(NotificationManager::class.java).notify(id, n)
        askNotes[e.id] = id
    }

    private fun notifyClip(from: String, text: String, sensitive: Boolean) {
        Log.i(TAG, "CLIP from $from (${text.length} chars${if (sensitive) ", sensitive" else ""})")
        val id = nextNotificationId()
        synchronized(Bridge.pendingClips) {
            Bridge.pendingClips.remove(id)
            Bridge.pendingClips[id] = text to sensitive
            // Swiped-away clips are never claimed: keep only the newest few.
            while (Bridge.pendingClips.size > 10) Bridge.pendingClips.remove(Bridge.pendingClips.keys.first())
        }
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

    /**
     * The PC asks to control this phone and "Let my PC control this phone" is
     * off: show the question (the app's accessibility service lets it open a
     * window from the background) and a notification as the fallback. The
     * request times out after 30 s on the Rust side.
     */
    /** A PC asks to see this screen: resume within the grace, else ask. */
    private fun askScreen(e: PbEvent.ScreenRequest) {
        if (e.resume && ScreenShare.resume(e.id)) return
        val peerId = uniquePeerId(Bridge.node?.peers()?.map { it.name to it.id }.orEmpty(), e.peerName)
        if (ScreenShare.armed != null && allowedFor(peerId, Prefs(this).allowedScreenPcs)) {
            val a = ScreenAsk(
                e.id, e.peerName, e.maxLongSide.toInt(), e.sharp, e.dim,
                List(e.codecs.size) { e.codecs[it].toInt() and 0xff },
            )
            android.os.Handler(mainLooper).post { ScreenShare.startArmed(a) }
            Log.i(TAG, "SCREEN request from ${e.peerName}: instant")
            return
        }
        askNotes[e.id] = SCREEN_ASK_ID
        val ask = Intent(this, ScreenConsentActivity::class.java)
            .putExtra("id", e.id.toLong())
            .putExtra("peer", e.peerName)
            .putExtra("peerId", peerId)
            .putExtra("at", android.os.SystemClock.elapsedRealtime())
            .putExtra("maxLong", e.maxLongSide.toInt())
            .putExtra("sharp", e.sharp)
            .putExtra("dim", e.dim)
            .putExtra("codecs", IntArray(e.codecs.size) { e.codecs[it].toInt() and 0xff })
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        val locked = getSystemService(android.app.KeyguardManager::class.java).isKeyguardLocked
        // Always posted: Android may refuse to open the window from the
        // background (the accessibility service gives the exemption and is
        // off after a force-stop). The window cancels it when it opens.
        run {
            val n = Notification.Builder(this, CH_CONTROL)
                .setSmallIcon(R.drawable.ic_stat_bridge)
                .setContentTitle(askTitle("share your screen", e.peerName, locked))
                .setContentText(askBody(locked))
                .setContentIntent(PendingIntent.getActivity(this, 43, ask, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT))
                .setTimeoutAfter(30_000)
                .setAutoCancel(true)
                .build()
            getSystemService(NotificationManager::class.java).notify(SCREEN_ASK_ID, n)
        }
        runCatching { startActivity(ask) }.onFailure { Log.i(TAG, "SCREEN ask window not allowed: $it") }
        Log.i(TAG, "SCREEN request from ${e.peerName} resume=${e.resume}")
    }

    /** A PC asks to use this camera: resume within the grace, else ask. */
    private fun askCamera(e: PbEvent.CameraRequest) {
        if (e.resume && CameraShare.resume(e.id)) return
        askNotes[e.id] = CAMERA_ASK_ID
        val ask = Intent(this, CameraConsentActivity::class.java)
            .putExtra("id", e.id.toLong())
            .putExtra("peer", e.peerName)
            .putExtra("peerId", uniquePeerId(Bridge.node?.peers()?.map { it.name to it.id }.orEmpty(), e.peerName))
            .putExtra("at", android.os.SystemClock.elapsedRealtime())
            .putExtra("lens", e.lens.toInt())
            .putExtra("width", e.width.toInt())
            .putExtra("height", e.height.toInt())
            .putExtra("codecs", IntArray(e.codecs.size) { e.codecs[it].toInt() and 0xff })
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        val locked = getSystemService(android.app.KeyguardManager::class.java).isKeyguardLocked
        run {
            val n = Notification.Builder(this, CH_CONTROL)
                .setSmallIcon(R.drawable.ic_stat_bridge)
                .setContentTitle(askTitle("use your camera", e.peerName, locked))
                .setContentText(askBody(locked))
                .setContentIntent(PendingIntent.getActivity(this, 44, ask, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT))
                .setTimeoutAfter(30_000)
                .setAutoCancel(true)
                .build()
            getSystemService(NotificationManager::class.java).notify(CAMERA_ASK_ID, n)
        }
        runCatching { startActivity(ask) }.onFailure { Log.i(TAG, "CAMERA ask window not allowed: $it") }
        Log.i(TAG, "CAMERA request from ${e.peerName} resume=${e.resume}")
    }

    /**
     * A PC asks for this phone's audio. Playback: Relay's dialog (unless the setting skips it), then
     * Android's capture dialog. Microphone: Relay's dialog every time. The window does the steps; the
     * notification is the fallback when Android refuses to open it from the background.
     */
    private fun askAudio(e: PbEvent.AudioRequest) {
        val mic = e.source.toInt() == 1
        // Starting the helper takes seconds and the PC waits for up to 40: never on the events thread.
        if (!mic && Prefs(this).audioHelperPaired && Prefs(this).audioSilencePhone) {
            Thread({
                HelperStarter.ensure(this, asked = true)
                answerAudio(e, mic)
            }, "pb-audio-ask").start()
        } else {
            answerAudio(e, mic)
        }
    }

    private fun answerAudio(e: PbEvent.AudioRequest, mic: Boolean) {
        if (AudioShare.debugTone && applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE != 0) {
            val src = dev.phonebridge.app.audio.AudioSource.PLAYBACK
            AudioShare.launchTone(this, AudioAsk(e.id, e.peerName, src, e.sampleRate.toInt(), e.channels.toInt()))
            return
        }
        if (!mic) {
            val prefs = Prefs(this)
            val kind = dev.phonebridge.app.audio.chooseCapture(
                dev.phonebridge.app.audio.AudioSource.PLAYBACK, prefs.audioSilencePhone, HelperAudio.alive(this),
            )
            if (dev.phonebridge.app.audio.startsWithoutWindow(kind, prefs.audioSkipConfirmPhoneToPc)) {
                val a = AudioAsk(e.id, e.peerName, dev.phonebridge.app.audio.AudioSource.PLAYBACK, e.sampleRate.toInt(), e.channels.toInt())
                val reason = AudioShare.prepareHelper()
                if (reason != null) AudioShare.abort(a, reason, teardown = false) else AudioShare.launch(this, a, null, helper = true)
                return
            }
        }
        val ask = Intent(this, AudioConsentActivity::class.java)
            .putExtra("id", e.id.toLong())
            .putExtra("peer", e.peerName)
            .putExtra("source", e.source.toInt())
            .putExtra("rate", e.sampleRate.toInt())
            .putExtra("channels", e.channels.toInt())
            .putExtra("at", android.os.SystemClock.elapsedRealtime())
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        val locked = getSystemService(android.app.KeyguardManager::class.java).isKeyguardLocked
        val n = Notification.Builder(this, CH_CONTROL)
            .setSmallIcon(R.drawable.ic_stat_bridge)
            .setContentTitle(askTitle(if (mic) "hear your microphone" else "hear this phone's audio", e.peerName, locked))
            .setContentText(askBody(locked))
            .setContentIntent(PendingIntent.getActivity(this, 45, ask, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT))
            .setTimeoutAfter(30_000)
            .setAutoCancel(true)
            .build()
        getSystemService(NotificationManager::class.java).notify(AUDIO_ASK_ID, n)
        runCatching { startActivity(ask) }.onFailure { Log.i(TAG, "AUDIO ask window not allowed: $it") }
        Log.i(TAG, "AUDIO request from ${e.peerName} source=${e.source}")
    }

    /** The service types in force: connectedDevice always, plus mediaProjection, camera and microphone while in use. */
    private var projecting = false
    private var cameraOn = false
    private var audioProjecting = false
    private var micOn = false

    /** Ongoing "sharing" notifications of a process that was killed are stale: nothing is shared now. */
    private fun clearStaleSharing() {
        val nm = getSystemService(NotificationManager::class.java)
        nm.cancel(SCREEN_SHARE_ID)
        nm.cancel(CAMERA_SHARE_ID)
        nm.cancel(AUDIO_SHARE_ID)
        nm.cancel(AUDIO_PLAY_ID)
        nm.cancel(AUDIO_ASK_ID)
    }

    private fun applyTypes(): Boolean = try {
        var types = ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE
        if (projecting || audioProjecting) types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION
        if (micOn && Build.VERSION.SDK_INT >= 30) types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
        if (cameraOn && Build.VERSION.SDK_INT >= 30) types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA
        startForeground(ONGOING_ID, statusNotification(), types)
        true
    } catch (t: Throwable) {
        Log.e(TAG, "SERVICE types refused (projection=$projecting camera=$cameraOn): $t")
        false
    }

    /** While sharing the screen the service also runs as mediaProjection (Android 14+ rule). */
    fun enterProjection(): Boolean {
        projecting = true
        return applyTypes().also { if (!it) projecting = false }
    }

    /** Phone audio sharing: playback capture needs the mediaProjection type, the microphone the microphone type. */
    fun enterAudio(source: dev.phonebridge.app.audio.AudioSource): Boolean {
        val mic = source == dev.phonebridge.app.audio.AudioSource.MICROPHONE
        if (mic) micOn = true else audioProjecting = true
        return applyTypes().also {
            if (!it) {
                if (mic) micOn = false else audioProjecting = false
            }
        }
    }

    fun leaveAudio(source: dev.phonebridge.app.audio.AudioSource) {
        if (source == dev.phonebridge.app.audio.AudioSource.MICROPHONE) micOn = false else audioProjecting = false
        applyTypes()
    }

    fun leaveProjection() {
        projecting = false
        applyTypes()
    }

    /** While the camera is on the service also runs as camera (entered while our window is visible). */
    fun enterCamera(): Boolean {
        cameraOn = true
        return applyTypes().also { if (!it) cameraOn = false }
    }

    fun leaveCamera() {
        cameraOn = false
        applyTypes()
    }

    private fun askControl(e: PbEvent.ControlRequest) {
        val peerId = uniquePeerId(Bridge.node?.peers()?.map { it.name to it.id }.orEmpty(), e.peerName)
        if (allowedFor(peerId, Prefs(this).allowedControlPcs)) {
            Bridge.node?.answerOffer(e.id, true)
            Log.i(TAG, "CONTROL request from ${e.peerName}: always allowed")
            return
        }
        askNotes[e.id] = CONTROL_ASK_ID
        fun answer(accept: Boolean) = PendingIntent.getBroadcast(
            this, if (accept) 41 else 40,
            Intent(this, OfferReceiver::class.java).putExtra("offer", e.id.toLong()).putExtra("accept", accept).putExtra("nid", CONTROL_ASK_ID),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val ask = Intent(this, ControlRequestActivity::class.java)
            .putExtra("id", e.id.toLong())
            .putExtra("peer", e.peerName)
            .putExtra("peerId", peerId)
            .putExtra("at", android.os.SystemClock.elapsedRealtime())
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        val n = Notification.Builder(this, CH_CONTROL)
            .setSmallIcon(R.drawable.ic_stat_bridge)
            .setContentTitle("${e.peerName} wants to control this phone")
            .setContentText("Its mouse and keyboard would work here until you stop it")
            .setContentIntent(PendingIntent.getActivity(this, 42, ask, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT))
            .addAction(Notification.Action.Builder(null, "Deny", answer(false)).build())
            .addAction(Notification.Action.Builder(null, "Allow", answer(true)).build())
            .setCategory(Notification.CATEGORY_CALL)
            .setTimeoutAfter(30_000)
            .setAutoCancel(true)
            .build()
        getSystemService(NotificationManager::class.java).notify(CONTROL_ASK_ID, n)
        runCatching { startActivity(ask) }.onFailure { Log.i(TAG, "CONTROL ask window not allowed: $it") }
        Log.i(TAG, "CONTROL request from ${e.peerName}")
    }

    /** Quiet progress for a file on its way (one per direction). */
    private fun notifyProgress(e: PbEvent.TransferProgress) {
        val id = if (e.incoming) PROGRESS_IN_ID else PROGRESS_OUT_ID
        if (e.done >= e.total) {
            cancelProgress(e.incoming)
            return
        }
        val pct = if (e.total == 0uL) 0 else (e.done.toDouble() * 100 / e.total.toDouble()).toInt()
        val now = android.os.SystemClock.elapsedRealtime()
        val started = progressStart.compute(e.incoming) { _, old -> if (old != null && old.first == e.name && e.done >= old.third) old else Triple(e.name, now, e.done) }!!
        val line = transferLine(e.done.toLong() - started.third.toLong(), e.total.toLong() - started.third.toLong(), now - started.second)
        val stop = PendingIntent.getBroadcast(
            this, if (e.incoming) 61 else 60,
            Intent(this, TransferReceiver::class.java).putExtra("do", "cancel").putExtra("incoming", e.incoming),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val n = Notification.Builder(this, CH_PROGRESS)
            .setSmallIcon(R.drawable.ic_stat_bridge)
            .setContentTitle(if (e.incoming) "Receiving ${e.name}" else "Sending ${e.name}")
            .setContentText("${if (e.incoming) "From" else "To"} ${e.peerName} \u00b7 $pct% \u00b7 $line")
            .addAction(Notification.Action.Builder(null, "Cancel", stop).build())
            .setProgress(100, pct, false)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setShowWhen(false)
            .build()
        getSystemService(NotificationManager::class.java).notify(id, n)
    }

    /** Per direction: file name, when its first progress arrived and how much was done then (for speed and ETA). */
    private val progressStart = java.util.concurrent.ConcurrentHashMap<Boolean, Triple<String, Long, ULong>>()

    private fun cancelProgress(incoming: Boolean) {
        progressStart.remove(incoming)
        getSystemService(NotificationManager::class.java).cancel(if (incoming) PROGRESS_IN_ID else PROGRESS_OUT_ID)
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

    /** One quiet line: how Relay stands with the PC (see [Bridge.status]). No buttons; tap opens the app. */
    private fun statusNotification(): Notification {
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
        val st = Bridge.status(this, serviceUp = true)
        return Notification.Builder(this, CH_STATUS)
            .setSmallIcon(R.drawable.ic_stat_bridge)
            .setContentTitle(st.title)
            .setContentText(st.detail)
            .setOngoing(true)
            .setShowWhen(false)
            .setContentIntent(open)
            .build()
    }

    fun updateStatus() {
        // After stopForeground a notify would leave a stale status line behind.
        if (destroyed) return
        getSystemService(NotificationManager::class.java).notify(ONGOING_ID, statusNotification())
        refreshTile(this)
        // Node up, PC paired or removed, control started or ended (all come through here).
        RelayWidget.update(this)
    }

    companion object {
        /** The running service (screen sharing changes its foreground type). */
        @Volatile var instance: PbService? = null

        private const val STATUS_REDRAW_MS = 60_000L

        const val ACTION_STOP = "dev.phonebridge.app.STOP"
        private const val ACTION_REFRESH = "dev.phonebridge.app.REFRESH"

        /** A send from this phone ended without finishing: drop its progress bar. */
        fun cancelOutgoingProgress(ctx: Context) {
            ctx.getSystemService(NotificationManager::class.java).cancel(PROGRESS_OUT_ID)
        }

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
        val (text, sensitive) = synchronized(Bridge.pendingClips) { Bridge.pendingClips.remove(id) } ?: run {
            // Phone Bridge restarted since this notification was posted.
            context.getSystemService(NotificationManager::class.java).cancel(id)
            android.widget.Toast.makeText(context, "That clip is no longer available", android.widget.Toast.LENGTH_SHORT).show()
            return
        }
        val clip = ClipData.newPlainText("$APP_NAME", text)
        if (sensitive) {
            clip.description.extras = PersistableBundle().apply { putBoolean("android.content.extra.IS_SENSITIVE", true) }
        }
        context.getSystemService(ClipboardManager::class.java).setPrimaryClip(clip)
        context.getSystemService(NotificationManager::class.java).cancel(id)
        Log.i(TAG, "COPIED clip $id to clipboard")
    }
}

/** Accept/Decline buttons on an incoming-file notification. */
/** "Stop" and "Restore brightness" on the screen-sharing notification. */
class ScreenReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        when (intent.getStringExtra("do")) {
            "stop" -> {
                ScreenShare.stop(userStopped = true)
                // A fresh process (the old one died) has nothing to stop; the stale bar must still go.
                context.getSystemService(NotificationManager::class.java).cancel(SCREEN_SHARE_ID)
            }
            "bright" -> {
                ControlService.instance?.setDim(false)
                ScreenShare.dimEnded()
            }
        }
    }
}

class CameraReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        when (intent.getStringExtra("do")) {
            "stop" -> {
                CameraShare.stop(userStopped = true)
                context.getSystemService(NotificationManager::class.java).cancel(CAMERA_SHARE_ID)
            }
            "lens" -> CameraShare.switchLens()
        }
    }
}

/** Shown while a PC uses this camera. */
fun notifyCamera(ctx: Context, peer: String?) {
    val nm = ctx.getSystemService(NotificationManager::class.java)
    if (peer == null) {
        nm.cancel(CAMERA_SHARE_ID)
        return
    }
    fun action(what: String, code: Int) = PendingIntent.getBroadcast(
        ctx, code, Intent(ctx, CameraReceiver::class.java).putExtra("do", what),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )
    val n = Notification.Builder(ctx, "control")
        .setSmallIcon(R.drawable.ic_stat_bridge)
        .setContentTitle("Camera in use by $peer")
        .setContentText("Your camera is a webcam on that PC")
        .setOngoing(true)
        .addAction(Notification.Action.Builder(null, "Stop", action("stop", 52)).build())
        .addAction(Notification.Action.Builder(null, "Switch lens", action("lens", 53)).build())
        .build()
    nm.notify(CAMERA_SHARE_ID, n)
}

/** Shown while a PC sees this screen. */
fun notifySharing(ctx: Context, peer: String?) {
    val nm = ctx.getSystemService(NotificationManager::class.java)
    if (peer == null) {
        nm.cancel(SCREEN_SHARE_ID)
        return
    }
    fun action(what: String, code: Int) = PendingIntent.getBroadcast(
        ctx, code, Intent(ctx, ScreenReceiver::class.java).putExtra("do", what),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )
    val n = Notification.Builder(ctx, "control")
        .setSmallIcon(R.drawable.ic_stat_bridge)
        .setContentTitle("Sharing screen with $peer")
        .setContentText(screenShareText(ControlService.instance != null, ScreenShare.inputWasIgnored()))
        .setOngoing(true)
        .addAction(Notification.Action.Builder(null, "Stop", action("stop", 50)).build())
        .addAction(Notification.Action.Builder(null, "Restore brightness", action("bright", 51)).build())
        .build()
    nm.notify(SCREEN_SHARE_ID, n)
}

/** "Cancel" on a progress notification and "Retry" on a failed send. */
class TransferReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        when (intent.getStringExtra("do")) {
            "cancel" -> {
                val incoming = intent.getBooleanExtra("incoming", false)
                if (!incoming) OutgoingShare.cancelRequested = true
                Bridge.node?.cancelTransfers(incoming)
                Log.i(TAG, "TRANSFER cancel requested incoming=$incoming")
            }
            "retry" -> {
                val id = intent.getLongExtra("retry", -1)
                context.getSystemService(NotificationManager::class.java).cancel(intent.getIntExtra("nid", 0))
                OutgoingShare.retry(context.applicationContext, id)
            }
        }
    }
}

class OfferReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val offer = intent.getLongExtra("offer", -1)
        val accept = intent.getBooleanExtra("accept", false)
        Bridge.node?.answerOffer(offer.toULong(), accept)
        PbService.instance?.askAnswered(offer.toULong())
        context.getSystemService(NotificationManager::class.java).cancel(intent.getIntExtra("nid", 0))
        // Answered from the notification: close the popup asking the same thing.
        ControlRequestActivity.answeredElsewhere(offer)
        Log.i(TAG, "OFFER $offer answered accept=$accept")
    }
}
