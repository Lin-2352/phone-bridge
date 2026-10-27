package dev.phonebridge.app

import android.app.Activity
import android.app.PendingIntent
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.os.Build
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import android.util.Log
import android.widget.Toast
import kotlinx.coroutines.launch

private const val TAG = "PhoneBridge"

/**
 * Quick Settings tile "Send clipboard": sends what is copied on the phone (a picture or text) to the PC.
 * Android lets only a focused app read the clipboard, so the tap opens [SendClipboardActivity].
 */
class SendClipTileService : TileService() {
    override fun onStartListening() {
        qsTile?.apply {
            state = Tile.STATE_INACTIVE
            if (Build.VERSION.SDK_INT >= 29) subtitle = "To your PC"
            updateTile()
        }
    }

    override fun onClick() {
        val i = Intent(this, SendClipboardActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        if (Build.VERSION.SDK_INT >= 34) {
            startActivityAndCollapse(PendingIntent.getActivity(this, 1, i, PendingIntent.FLAG_IMMUTABLE))
        } else {
            @Suppress("DEPRECATION")
            startActivityAndCollapse(i)
        }
    }
}

/** Reads the clipboard once it has focus, sends it to the PC and disappears. */
class SendClipboardActivity : Activity() {
    private var done = false

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (!hasFocus || done) return
        done = true
        val app = applicationContext
        val clip = getSystemService(ClipboardManager::class.java).primaryClip
        val item = clip?.takeIf { it.itemCount > 0 }?.getItemAt(0)
        val uri = item?.uri
        val isImage = clip?.description?.hasMimeType("image/*") == true && uri != null
        val text = if (isImage) null else item?.text?.toString()?.take(60 * 1024)
        val sensitive = clip?.description?.extras?.getBoolean(ClipDescription.EXTRA_IS_SENSITIVE) == true
        when {
            !Prefs(this).enabled || !Bridge.shownOn() ->
                Toast.makeText(this, "$APP_NAME is off. Turn it on from its Quick Settings tile", Toast.LENGTH_LONG).show()
            !isImage && text.isNullOrEmpty() ->
                Toast.makeText(this, "Nothing copied to send", Toast.LENGTH_SHORT).show()
            else -> {
                Toast.makeText(this, if (isImage) "Sending picture to your PC" else "Sending to your PC", Toast.LENGTH_SHORT).show()
                // The clipboard's picture grant ends with this activity: read it now.
                val bytes = if (isImage) runCatching { contentResolver.openInputStream(uri!!)?.use { it.readCapped(40L * 1024 * 1024) } }.getOrNull() else null
                Bridge.scope.launch {
                    val problem = when {
                        isImage && bytes == null -> imageSendNotice(null)
                        isImage -> ClipImages.sendBytes(app, bytes!!)
                        else -> sendText(app, text!!, sensitive)
                    }
                    if (problem != null) notifyInfo(app, "Couldn't send the clipboard", problem)
                }
            }
        }
        finish()
    }

    private suspend fun sendText(app: Context, text: String, sensitive: Boolean): String? {
        val node = Bridge.awaitNode() ?: return "$APP_NAME is not running"
        val peer = node.peers().firstOrNull() ?: return "Pair $APP_NAME with your PC first"
        return try {
            node.sendClipboard(peer.id, text, sensitive)
            Log.i(TAG, "SENDCLIP text to ${peer.name}")
            Bridge.addRecent(RecentItem(text.take(80), "Sent to ${peer.name}", RecentItem.Kind.Clipboard))
            null
        } catch (t: Throwable) {
            Log.e(TAG, "SENDCLIP failed: $t")
            "${peer.name} did not receive it"
        }
    }
}
