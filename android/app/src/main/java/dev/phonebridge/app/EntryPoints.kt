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
        if (!Prefs(this).enabled) {
            Toast.makeText(this, "Phone Bridge is off. Turn it on from its Quick Settings tile", Toast.LENGTH_LONG).show()
            finish()
            return
        }
        startForegroundService(Intent(this, PbService::class.java))
        val node = Bridge.node
        val peer = Bridge.firstPeer()
        if (node == null || peer == null) {
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
        Bridge.scope.launch {
            for (f in opened) {
                try {
                    node.sendFileFd(peer.id, f.fd, f.name, f.size.toULong(), f.mime)
                    Log.i(TAG, "SHARE sent ${f.name} (${f.size} bytes) to ${peer.name}")
                    Bridge.addRecent(RecentItem(f.name, "Sent to ${peer.name}", RecentItem.Kind.File))
                } catch (t: Throwable) {
                    Log.e(TAG, "SHARE failed: $t")
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
                }
            }
        }
        Toast.makeText(this, "Sending to ${peer.name}", Toast.LENGTH_SHORT).show()
        finish()
    }
}
