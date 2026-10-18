package dev.phonebridge.app

import android.app.Activity
import android.app.PendingIntent
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.OpenableColumns
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
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
        Bridge.scope.launch {
            for (uri in uris) {
                try {
                    var name = "file"
                    var size = -1L
                    resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { c ->
                        if (c.moveToFirst()) {
                            name = c.getString(0) ?: name
                            if (!c.isNull(1)) size = c.getLong(1)
                        }
                    }
                    val pfd = resolver.openFileDescriptor(uri, "r") ?: continue
                    if (size < 0) size = pfd.statSize
                    val mime = resolver.getType(uri) ?: ""
                    node.sendFileFd(peer.id, pfd.detachFd(), name, size.toULong(), mime)
                    Log.i(TAG, "SHARE sent $name ($size bytes) to ${peer.name}")
                    Bridge.addRecent(RecentItem(name, "Sent to ${peer.name}", RecentItem.Kind.File))
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

/**
 * Phone -> PC clipboard. Android only lets the focused app read the
 * clipboard, so this invisible activity reads it once it has focus, sends it,
 * and closes. Launched from the Quick Settings tile and the notification.
 * Text an app marked sensitive (passwords, codes) is never sent.
 */
class ClipSyncActivity : Activity() {
    private var done = false

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (!hasFocus || done) return
        done = true
        val cm = getSystemService(ClipboardManager::class.java)
        val clip = cm.primaryClip
        val sensitive = clip?.description?.extras?.getBoolean(ClipDescription.EXTRA_IS_SENSITIVE) == true
        val text = clip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(this)?.toString()
        val node = Bridge.node
        val peer = Bridge.firstPeer()
        when {
            node == null || peer == null -> Toast.makeText(this, "Pair Phone Bridge with your PC first", Toast.LENGTH_SHORT).show()
            text.isNullOrEmpty() -> Toast.makeText(this, "Clipboard is empty", Toast.LENGTH_SHORT).show()
            sensitive -> Toast.makeText(this, "Not sent: marked sensitive by the app that copied it", Toast.LENGTH_SHORT).show()
            else -> {
                Bridge.scope.launch {
                    try {
                        node.sendText(peer.id, text, false)
                        Log.i(TAG, "CLIPSEND ${text.length} chars to ${peer.name}")
                    } catch (t: Throwable) {
                        Log.e(TAG, "CLIPSEND failed: $t")
                    }
                }
                Toast.makeText(this, "Clipboard sent to ${peer.name}", Toast.LENGTH_SHORT).show()
            }
        }
        finish()
    }
}

/** Quick Settings tile: "Send clipboard" to the PC in one tap. */
class ClipTileService : TileService() {
    override fun onStartListening() {
        qsTile?.apply {
            state = if (Bridge.firstPeer() != null) Tile.STATE_ACTIVE else Tile.STATE_INACTIVE
            updateTile()
        }
    }

    override fun onClick() {
        val intent = Intent(this, ClipSyncActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        if (Build.VERSION.SDK_INT >= 34) {
            startActivityAndCollapse(PendingIntent.getActivity(this, 0, intent, PendingIntent.FLAG_IMMUTABLE))
        } else {
            @Suppress("DEPRECATION")
            startActivityAndCollapse(intent)
        }
    }
}
