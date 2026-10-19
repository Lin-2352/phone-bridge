package dev.phonebridge.app

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.provider.OpenableColumns
import android.util.Log
import android.widget.Toast
import kotlinx.coroutines.launch

private const val TAG = "PhoneBridge"

/**
 * Share target: "Phone Bridge" in Android's share sheet. Files are streamed
 * straight from their content URI (detached file descriptor, no copy); text
 * and links travel inline. Finishes immediately; sending continues in the
 * background and reports through notifications.
 */
class ShareActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        honourUserStop(this)
        if (!Prefs(this).enabled) {
            Toast.makeText(this, "Phone Bridge is off. Turn it on from its Quick Settings tile", Toast.LENGTH_LONG).show()
            finish()
            return
        }
        startForegroundService(Intent(this, PbService::class.java))
        // The service may have been stopped by Android and is starting again
        // now; then the node (and the peer list) comes a moment later.
        val known = Bridge.firstPeer()
        if (Bridge.node != null && known == null) {
            Toast.makeText(this, "Pair Phone Bridge with your PC first", Toast.LENGTH_LONG).show()
            finish()
            return
        }
        val uris = mutableListOf<Uri>()
        when (intent?.action) {
            Intent.ACTION_SEND -> {
                @Suppress("DEPRECATION")
                (intent.getParcelableExtra<Uri>(Intent.EXTRA_STREAM))?.let(uris::add)
            }
            Intent.ACTION_SEND_MULTIPLE -> {
                @Suppress("DEPRECATION")
                intent.getParcelableArrayListExtra<Uri>(Intent.EXTRA_STREAM)?.let(uris::addAll)
            }
        }
        val text = intent?.getStringExtra(Intent.EXTRA_TEXT)
        val resolver = contentResolver
        // Open every file now, while this activity still holds the sender's
        // URI grant (it ends with finish() below); only the sending is async.
        data class Opened(val name: String, val size: Long, val mime: String, val fd: Int)
        val opened = uris.mapNotNull { uri ->
            // Only content shared by another app: never our own files, raw
            // paths, or the save folder we hold access to.
            if (uri.scheme != "content" || uri.authority?.startsWith(packageName) == true ||
                contentResolver.persistedUriPermissions.any { uri.toString().startsWith(it.uri.toString()) }
            ) {
                Log.e(TAG, "SHARE refused $uri")
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
                    Log.e(TAG, "SHARE could not open $uri: no file descriptor")
                    return@mapNotNull null
                }
                if (size < 0) size = pfd.statSize
                Opened(name, size, resolver.getType(uri) ?: "", pfd.detachFd())
            } catch (t: Throwable) {
                Log.e(TAG, "SHARE could not open $uri: $t")
                null
            }
        }
        if (uris.isNotEmpty() && opened.isEmpty()) {
            Toast.makeText(this, "Could not read the shared files", Toast.LENGTH_LONG).show()
            finish()
            return
        }
        val app = applicationContext
        Bridge.scope.launch {
            val node = Bridge.awaitNode()
            val peer = node?.peers()?.firstOrNull()
            if (node == null || peer == null) {
                opened.forEach { runCatching { android.os.ParcelFileDescriptor.adoptFd(it.fd).close() } }
                Log.e(TAG, "SHARE dropped: ${if (node == null) "Phone Bridge did not start" else "not paired"}")
                android.os.Handler(android.os.Looper.getMainLooper()).post {
                    Toast.makeText(app, if (node == null) "Phone Bridge could not start. Try again" else "Pair Phone Bridge with your PC first", Toast.LENGTH_LONG).show()
                }
                return@launch
            }
            for (f in opened) {
                try {
                    node.sendFileFd(peer.id, f.fd, f.name, f.size.toULong(), f.mime)
                    Log.i(TAG, "SHARE sent ${f.name} (${f.size} bytes) to ${peer.name}")
                    Bridge.addRecent(RecentItem(f.name, "Sent to ${peer.name}", RecentItem.Kind.File))
                } catch (t: Throwable) {
                    Log.e(TAG, "SHARE failed: $t")
                    PbService.cancelOutgoingProgress(app)
                    notifyInfo(app, "Could not send ${f.name}", "${peer.name} did not receive it. Is Phone Bridge running there?")
                }
            }
            if (uris.isEmpty() && !text.isNullOrBlank()) {
                val isLink = text.startsWith("http://") || text.startsWith("https://")
                try {
                    node.sendText(peer.id, text, isLink)
                    Log.i(TAG, "SHARE sent ${if (isLink) "link" else "text"} to ${peer.name}")
                    Bridge.addRecent(RecentItem(text.take(80), "Sent to ${peer.name}", if (isLink) RecentItem.Kind.Link else RecentItem.Kind.Clipboard))
                } catch (t: Throwable) {
                    Log.e(TAG, "SHARE text failed: $t")
                    notifyInfo(app, if (isLink) "Could not send the link" else "Could not send the text", "${peer.name} did not receive it")
                }
            }
        }
        Toast.makeText(this, if (known != null) "Sending to ${known.name}" else "Sending to your PC", Toast.LENGTH_SHORT).show()
        finish()
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
