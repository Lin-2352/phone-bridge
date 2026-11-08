package dev.phonebridge.app.devices

import android.app.Activity
import android.content.ContentResolver
import android.net.Uri
import android.provider.OpenableColumns
import android.util.Log
import android.widget.Toast
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.ui.draganddrop.DragAndDropEvent
import androidx.compose.ui.draganddrop.DragAndDropTarget
import androidx.compose.ui.draganddrop.toAndroidDragEvent
import dev.phonebridge.app.APP_NAME
import dev.phonebridge.app.Bridge
import dev.phonebridge.app.OutgoingShare
import dev.phonebridge.app.isOwnAuthority
import dev.phonebridge.app.isUnderTree
import kotlinx.coroutines.launch

/** A window this wide (in dp) or wider takes dropped files: tablets, foldables, desktop mode, split screen at full width. */
const val LARGE_SCREEN_DP = 600

fun isLargeScreen(widthDp: Int): Boolean = widthDp >= LARGE_SCREEN_DP

/**
 * Whether a dropped item may be sent. Only content from another app: never a raw path or web address, never this app's
 * own files, and never the save folder we hold access to (a drop must not send a received file back).
 */
fun dropAllowed(uri: String, pkg: String, saveTrees: List<String>): Boolean {
    if (!uri.startsWith("content://", ignoreCase = true)) return false
    val authority = uri.substring("content://".length).substringBefore('/')
    if (isOwnAuthority(authority, pkg)) return false
    return saveTrees.none { isUnderTree(uri, it) }
}

/** The dropped items that may be sent, in the order they were dropped. */
fun allowedDrops(uris: List<String>, pkg: String, saveTrees: List<String>): List<String> =
    uris.filter { dropAllowed(it, pkg, saveTrees) }

private const val TAG = "PhoneBridge"

/** Opens a dropped file for sending, while the drop's permission is still held. */
private fun openDropped(resolver: ContentResolver, uri: Uri): OutgoingShare.Out? = try {
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
        null
    } else {
        if (size < 0) size = pfd.statSize
        // A copy stays for Retry: the drop's permission ends with the activity.
        val keep = runCatching { pfd.dup() }.getOrNull()
        OutgoingShare.Out(name, size, resolver.getType(uri) ?: "", pfd.detachFd(), keep)
    }
} catch (t: Throwable) {
    Log.e(TAG, "DROP could not open a file: ${t.javaClass.simpleName}")
    null
}

/** Receives files dragged in from another window on a large screen and sends them to the default PC. */
@OptIn(ExperimentalFoundationApi::class)
class DropReceiver(private val activity: Activity) : DragAndDropTarget {
    override fun onDrop(event: DragAndDropEvent): Boolean {
        val ev = event.toAndroidDragEvent()
        val clip = ev.clipData ?: return false
        val uris = (0 until clip.itemCount).mapNotNull { clip.getItemAt(it).uri?.toString() }
        val trees = activity.contentResolver.persistedUriPermissions.map { it.uri.toString() }
        val ok = allowedDrops(uris, activity.packageName, trees)
        if (ok.isEmpty()) return false
        val permissions = activity.requestDragAndDropPermissions(ev)
        val opened = try {
            ok.mapNotNull { openDropped(activity.contentResolver, Uri.parse(it)) }
        } finally {
            permissions?.release()
        }
        if (opened.isEmpty()) return false
        val app = activity.applicationContext
        Bridge.scope.launch {
            val node = Bridge.awaitNode()
            val peer = if (node == null) null else Bridge.defaultPeer(app)
            if (node == null || peer == null) {
                opened.forEach { runCatching { android.os.ParcelFileDescriptor.adoptFd(it.fd).close() }; runCatching { it.keep?.close() } }
                android.os.Handler(android.os.Looper.getMainLooper()).post {
                    Toast.makeText(app, if (node == null) "$APP_NAME could not start. Try again" else "Pair $APP_NAME with your PC first", Toast.LENGTH_LONG).show()
                }
                return@launch
            }
            OutgoingShare.cancelRequested = false
            for (f in opened) {
                if (OutgoingShare.cancelRequested) {
                    runCatching { android.os.ParcelFileDescriptor.adoptFd(f.fd).close() }
                    OutgoingShare.offerRetry(app, peer.id, peer.name, f, cancelled = true)
                    continue
                }
                OutgoingShare.send(app, node, peer.id, peer.name, f)
            }
        }
        return true
    }
}
