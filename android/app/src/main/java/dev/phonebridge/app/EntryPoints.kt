package dev.phonebridge.app

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.provider.OpenableColumns
import android.util.Log
import android.widget.Toast
import kotlinx.coroutines.launch

/** Whether a content authority is one of ours. Android reads "0@authority" as that user's copy of it. */
internal fun isOwnAuthority(authority: String?, pkg: String): Boolean =
    authority?.substringAfterLast('@')?.startsWith(pkg) == true

private const val TAG = "PhoneBridge"

/**
 * Share target: "Phone Bridge" in Android's share sheet. Any app in front can
 * share, so nothing leaves the phone before the user confirms on a small sheet
 * that names the PC and previews what goes. Files are then streamed straight
 * from their content URI (detached file descriptor, no copy); text and links
 * travel inline. Sending continues in the background and reports through
 * notifications.
 */
class ShareActivity : Activity() {
    /** Files opened for the sheet and not sent yet: closed if the user cancels or leaves. */
    private var unsent: List<OutgoingShare.Out> = emptyList()

    override fun onDestroy() {
        unsent.forEach { runCatching { android.os.ParcelFileDescriptor.adoptFd(it.fd).close() }; runCatching { it.keep?.close() } }
        unsent = emptyList()
        super.onDestroy()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        AppLog.init(this)
        honourUserStop(this)
        if (!Prefs(this).enabled) {
            Toast.makeText(this, "$APP_NAME is off. Turn it on from its Quick Settings tile", Toast.LENGTH_LONG).show()
            finish()
            return
        }
        if (!startsOnOpen(enabled = true, identityLost = Prefs(this).identityLost)) {
            Toast.makeText(this, "$APP_NAME can't open its keys. Open $APP_NAME to reset it", Toast.LENGTH_LONG).show()
            finish()
            return
        }
        startForegroundService(Intent(this, PbService::class.java))
        // The service may have been stopped by Android and is starting again
        // now; then the node (and the peer list) comes a moment later.
        val known = Bridge.defaultPeer(this)
        var chosenId: String? = known?.id
        if (Bridge.node != null && known == null) {
            Toast.makeText(this, "Pair $APP_NAME with your PC first", Toast.LENGTH_LONG).show()
            finish()
            return
        }
        val uris = sharedUris(intent)
        if (uris == null) {
            Toast.makeText(this, "Could not read the shared files", Toast.LENGTH_LONG).show()
            finish()
            return
        }
        // Styled text (Gmail, Docs, browsers) is a Spanned CharSequence: getStringExtra would return null for it.
        val text = sharedText(intent?.getCharSequenceExtra(Intent.EXTRA_TEXT))
        val resolver = contentResolver
        // Open every file now, while this activity still holds the sender's
        // URI grant (it ends with finish() below); only the sending is async.
        val opened = uris.mapNotNull { uri ->
            // Only content shared by another app: never our own files, raw
            // paths, or the save folder we hold access to.
            if (uri.scheme != "content" || isOwnAuthority(uri.authority, packageName) ||
                contentResolver.persistedUriPermissions.any { isUnderTree(uri.toString(), it.uri.toString()) }
            ) {
                Log.e(TAG, "SHARE refused scheme=${uri.scheme}")
                return@mapNotNull null
            }
            try {
                var name = "file"
                var size = -1L
                resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { c ->
                    if (c.moveToFirst()) {
                        name = c.getString(0) ?: name
                        if (!c.isNull(1)) size = c.getLong(1)
                    }
                }
                val pfd = resolver.openFileDescriptor(uri, "r")
                if (pfd == null) {
                    Log.e(TAG, "SHARE could not open a file: no file descriptor")
                    return@mapNotNull null
                }
                if (size < 0) size = pfd.statSize
                // A duplicate stays with us for a Retry after a failed or cancelled send (the share grant ends with this activity).
                val keep = runCatching { pfd.dup() }.getOrNull()
                OutgoingShare.Out(name, size, resolver.getType(uri) ?: "", pfd.detachFd(), keep)
            } catch (t: Throwable) {
                Log.e(TAG, "SHARE could not open a file: ${t.javaClass.simpleName}")
                null
            }
        }
        if (uris.isNotEmpty() && opened.isEmpty()) {
            Toast.makeText(this, "Could not read the shared files", Toast.LENGTH_LONG).show()
            finish()
            return
        }
        val toSend = textToSend(text)
        if (opened.isEmpty() && toSend == null) {
            Toast.makeText(this, shareToast(known?.name, 0, false, 0), Toast.LENGTH_LONG).show()
            finish()
            return
        }
        unsent = opened
        val preview = sharePreview(
            when (toSend) {
                is SharedTextKind.Link -> toSend.url
                is SharedTextKind.Text -> toSend.text
                null -> null
            },
            opened.map { it.name },
        )
        window.decorView.filterTouchesWhenObscured = true
        showSheet(opened, toSend, preview, uris.size - opened.size, chosenId)
    }

    /** The confirm sheet. With several PCs paired it offers "Change PC" (the default is preselected). */
    private fun showSheet(opened: List<OutgoingShare.Out>, toSend: SharedTextKind?, preview: String, dropped: Int, chosenId: String?) {
        val peers = Bridge.node?.peers().orEmpty()
        val names = dev.phonebridge.app.devices.labels(peers, peers.associate { it.id to Prefs(this).pcNickname(it.id) })
        val target = peers.firstOrNull { it.id == chosenId }
        val label = target?.let { names[it.id] ?: it.name }
        val b = android.app.AlertDialog.Builder(this)
            .setTitle("Send to ${label ?: "your PC"}?")
            .setMessage(preview)
            .setPositiveButton("Send") { _, _ -> send(opened, toSend, label, dropped, target?.id) }
            .setNegativeButton("Cancel") { _, _ -> finish() }
            .setOnCancelListener { finish() }
        if (peers.size > 1) {
            b.setNeutralButton("Change PC") { _, _ ->
                val order = (dev.phonebridge.app.devices.sendTarget(peers, null, chosenId) as? dev.phonebridge.app.devices.SendTarget.Ask)?.ids
                    ?: peers.map { it.id }
                android.app.AlertDialog.Builder(this)
                    .setTitle("Send to")
                    .setItems(order.map { names[it] ?: it }.toTypedArray()) { _, i ->
                        showSheet(opened, toSend, preview, dropped, order[i])
                    }
                    .setOnCancelListener { showSheet(opened, toSend, preview, dropped, chosenId) }
                    .show()
            }
        }
        val sheet = b.create()
        // A tap that went through another app's overlay is not a decision.
        sheet.setOnShowListener { sheet.getButton(android.app.AlertDialog.BUTTON_POSITIVE)?.filterTouchesWhenObscured = true }
        sheet.show()
    }

    private fun send(opened: List<OutgoingShare.Out>, toSend: SharedTextKind?, knownName: String?, dropped: Int, peerId: String?) {
        unsent = emptyList()
        val app = applicationContext
        Bridge.scope.launch {
            val node = Bridge.awaitNode()
            // The chosen PC if still paired; with none chosen the default. A chosen PC that is gone is not replaced
            // by another one: the share is dropped and the person told.
            val peer = if (node == null) null else if (peerId != null) node.peers().firstOrNull { it.id == peerId } else Bridge.defaultPeer(app)
            if (node == null || peer == null) {
                opened.forEach { runCatching { android.os.ParcelFileDescriptor.adoptFd(it.fd).close() }; runCatching { it.keep?.close() } }
                AppLog.event("events", "share_dropped", "why" to if (node == null) "no_node" else "not_paired")
                Log.e(TAG, "SHARE dropped: ${if (node == null) "$APP_NAME did not start" else "not paired"}")
                android.os.Handler(android.os.Looper.getMainLooper()).post {
                    Toast.makeText(app, if (node == null) "$APP_NAME could not start. Try again" else "Pair $APP_NAME with your PC first", Toast.LENGTH_LONG).show()
                }
                return@launch
            }
            OutgoingShare.cancelRequested = false
            for (f in opened) {
                if (OutgoingShare.cancelRequested) {
                    // Cancel stopped one file; the rest of the batch goes with it (kept for Retry).
                    runCatching { android.os.ParcelFileDescriptor.adoptFd(f.fd).close() }
                    OutgoingShare.offerRetry(app, peer.id, peer.name, f, cancelled = true)
                    continue
                }
                OutgoingShare.send(app, node, peer.id, peer.name, f)
            }
            if (toSend != null) {
                val isLink = toSend is SharedTextKind.Link
                val text = when (toSend) {
                    is SharedTextKind.Link -> toSend.url
                    is SharedTextKind.Text -> toSend.text
                }
                try {
                    node.sendText(peer.id, text, isLink)
                    Log.i(TAG, "SHARE sent ${if (isLink) "link" else "text"} to ${peer.name}")
                    Bridge.attemptDone(app, reached = true)
                    AppLog.event("events", "share_sent", "kind" to if (isLink) "link" else "text", "chars" to text.length)
                    Timeline.add(if (isLink) linkEntry(peer.name, text) else clipTextEntry(peer.name, text))
                } catch (t: Throwable) {
                    Log.e(TAG, "SHARE text failed: $t")
                    Bridge.attemptDone(app, reached = false)
                    AppLog.event("events", "share_failed", "kind" to if (isLink) "link" else "text", "type" to t.javaClass.simpleName)
                    notifyInfo(
                        app, if (isLink) "Could not send the link" else "Could not send the text",
                        outdatedText(t.message, peer.name) ?: "${peer.name} did not receive it",
                    )
                }
            }
        }
        Toast.makeText(this, shareToast(knownName, opened.size, toSend != null, dropped), Toast.LENGTH_SHORT).show()
        finish()
    }
}

/**
 * The stream URIs of a share intent, or null when the extras are not what a share sends.
 * Typed getters: a foreign app can put any Parcelable (or a broken one) there; this must never throw.
 */
internal fun sharedUris(intent: Intent?): List<Uri>? {
    if (intent == null) return emptyList()
    return try {
        when (intent.action) {
            Intent.ACTION_SEND ->
                listOfNotNull(androidx.core.content.IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java))
            Intent.ACTION_SEND_MULTIPLE ->
                androidx.core.content.IntentCompat.getParcelableArrayListExtra(intent, Intent.EXTRA_STREAM, Uri::class.java)
                    ?.filterNotNull().orEmpty()
            else -> emptyList()
        }
    } catch (t: Throwable) {
        Log.e(TAG, "SHARE extras unreadable: ${t.javaClass.simpleName}")
        null
    }
}

/** Sending files: shared by the share sheet and Retry. */
object OutgoingShare {
    /** Cancel was tapped on the progress notification: the rest of the batch is not sent. */
    @Volatile var cancelRequested = false

    class Out(val name: String, val size: Long, val mime: String, val fd: Int, val keep: android.os.ParcelFileDescriptor?)
    private class Pending(val peerId: String, val peerName: String, val name: String, val size: Long, val mime: String, val keep: android.os.ParcelFileDescriptor)

    private val store = RetryStore<Pending>(4)
    private val seq = java.util.concurrent.atomic.AtomicLong(1)

    /** Sends one file; on failure or cancel posts a notification with Retry while a copy of the descriptor is kept. */
    suspend fun send(app: android.content.Context, node: uniffi.pb_ffi.PbNode, peerId: String, peerName: String, f: Out) {
        try {
            node.sendFileFd(peerId, f.fd, f.name, f.size.toULong(), f.mime)
            Log.i(TAG, "SHARE sent file ext=${LogFields.extOnly(f.name)} (${f.size} bytes)")
            Bridge.attemptDone(app, reached = true)
            AppLog.event("events", "share_sent", "kind" to "file", "ext" to LogFields.extOnly(f.name), "bytes" to f.size)
            // No uri: the share grant ends with this send, so it could not be opened later.
            Timeline.add(fileEntry(peerName, TlDir.ToPc, f.name, f.size, null))
            runCatching { f.keep?.close() }
        } catch (t: Throwable) {
            val cancelled = isCancelled(t.message.orEmpty())
            Log.e(TAG, "SHARE failed: $t")
            if (!cancelled) Bridge.attemptDone(app, reached = false)
            AppLog.event("events", "share_failed", "kind" to "file", "ext" to LogFields.extOnly(f.name), "type" to t.javaClass.simpleName, "cancelled" to cancelled)
            PbService.cancelOutgoingProgress(app)
            // A retry can't help until one side is updated: say which, and offer none.
            val outdated = outdatedText(t.message, peerName)
            if (outdated != null) notifyInfo(app, failureTitle(f.name, false, outgoing = true), outdated)
            else offerRetry(app, peerId, peerName, f, cancelled)
        }
    }

    fun offerRetry(app: android.content.Context, peerId: String, peerName: String, f: Out, cancelled: Boolean) {
        val title = failureTitle(f.name, cancelled, outgoing = true)
        val text = if (cancelled) "${f.name} was not sent to $peerName" else "$peerName did not receive ${f.name}. Is $APP_NAME running there?"
        val keep = f.keep
        if (keep == null) { notifyInfo(app, title, text); return }
        val id = seq.getAndIncrement()
        store.put(id, Pending(peerId, peerName, f.name, f.size, f.mime, keep)).forEach { runCatching { it.keep.close() } }
        val nid = 300 + (id % 20).toInt()
        val retry = android.app.PendingIntent.getBroadcast(
            app, nid,
            Intent(app, TransferReceiver::class.java).putExtra("do", "retry").putExtra("retry", id).putExtra("nid", nid),
            android.app.PendingIntent.FLAG_IMMUTABLE or android.app.PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val nm = app.getSystemService(android.app.NotificationManager::class.java)
        nm.createNotificationChannel(android.app.NotificationChannel("events", "Files and clipboard", android.app.NotificationManager.IMPORTANCE_DEFAULT))
        nm.notify(
            nid,
            android.app.Notification.Builder(app, "events").setSmallIcon(R.drawable.ic_stat_bridge)
                .setContentTitle(title).setContentText(text).setAutoCancel(true)
                .addAction(android.app.Notification.Action.Builder(null, "Retry", retry).build()).build(),
        )
    }

    /** Retry tap: rewind the kept descriptor and send again. */
    fun retry(app: android.content.Context, id: Long) {
        val p = store.take(id)
        if (p == null) {
            Toast.makeText(app, "That file is no longer available. Share it again", Toast.LENGTH_LONG).show()
            return
        }
        if (!Prefs(app).enabled) {
            // Turned off since the send failed: say so at once instead of waiting for a start that is not coming.
            runCatching { p.keep.close() }
            notifyInfo(app, "Could not send ${p.name}", "$APP_NAME is off. Turn it on, then share the file again")
            return
        }
        cancelRequested = false
        Bridge.scope.launch {
            val node = Bridge.awaitNode()
            val ok = node != null && runCatching {
                android.system.Os.lseek(p.keep.fileDescriptor, 0, android.system.OsConstants.SEEK_SET)
                p.keep.dup()
            }.getOrNull()?.let { d ->
                send(app, node, p.peerId, p.peerName, Out(p.name, p.size, p.mime, d.detachFd(), p.keep))
                true
            } == true
            if (!ok) {
                runCatching { p.keep.close() }
                notifyInfo(app, "Could not send ${p.name}", "$APP_NAME could not start. Share the file again")
            }
        }
    }
}

/** A plain one-off notification (e.g. a send that failed after its toast). */
fun notifyInfo(ctx: android.content.Context, title: String, text: String) {
    val nm = ctx.getSystemService(android.app.NotificationManager::class.java)
    nm.createNotificationChannel(
        android.app.NotificationChannel("events", "Files and clipboard", android.app.NotificationManager.IMPORTANCE_DEFAULT),
    )
    val n = android.app.Notification.Builder(ctx, "events")
        .setSmallIcon(R.drawable.ic_stat_bridge)
        .setContentTitle(title)
        .setContentText(text)
        .setAutoCancel(true)
        .build()
    nm.notify(200 + (System.nanoTime() % 50).toInt(), n)
}
