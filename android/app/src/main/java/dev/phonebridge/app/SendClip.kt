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
            if (Build.VERSION.SDK_INT >= 29) subtitle = dev.phonebridge.app.devices.tileSubtitle(Bridge.defaultLabel(this@SendClipTileService))
            updateTile()
        }
    }

    override fun onClick() {
        val i = Intent(this, SendClipboardActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        if (Build.VERSION.SDK_INT >= 34) {
            startActivityAndCollapse(PendingIntent.getActivity(this, 1, i, PendingIntent.FLAG_IMMUTABLE))
        } else {
            collapseAndStartLegacy(i)
        }
    }
}

/** The only form API 29..33 has; the PendingIntent one is API 34+. */
@android.annotation.SuppressLint("StartActivityAndCollapseDeprecated")
@Suppress("DEPRECATION")
private fun TileService.collapseAndStartLegacy(i: Intent) = startActivityAndCollapse(i)

/** Lets the person pick which paired PC sends go to; the choice is the default everywhere (tile, widget, share). */
class ChoosePcActivity : Activity() {
    override fun onCreate(savedInstanceState: android.os.Bundle?) {
        super.onCreate(savedInstanceState)
        val peers = Bridge.node?.peers().orEmpty()
        if (peers.size < 2) {
            finish()
            return
        }
        val prefs = Prefs(this)
        val names = dev.phonebridge.app.devices.labels(peers, peers.associate { it.id to prefs.pcNickname(it.id) })
        val order = (dev.phonebridge.app.devices.sendTarget(peers, null, prefs.defaultPc) as? dev.phonebridge.app.devices.SendTarget.Ask)?.ids
            ?: peers.map { it.id }
        android.app.AlertDialog.Builder(this)
            .setTitle("Send to")
            .setItems(order.map { names[it] ?: it }.toTypedArray()) { _, i ->
                prefs.defaultPc = order[i]
                Bridge.linked.value = Bridge.defaultLabel(this)
                PbService.refresh(this)
                finish()
            }
            .setOnCancelListener { finish() }
            .show()
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
                        else -> sendClipText(app, text!!, sensitive)
                    }
                    if (problem != null) notifyInfo(app, "Couldn't send the clipboard", problem)
                }
            }
        }
        finish()
    }
}

/** Sends text to the default PC as a clipboard update; null when it went, else what to tell the user. */
suspend fun sendClipText(app: Context, text: String, sensitive: Boolean): String? {
    val node = Bridge.awaitNode() ?: return "$APP_NAME is not running"
    val peer = Bridge.defaultPeer(app) ?: return "Pair $APP_NAME with your PC first"
    return try {
        node.sendClipboard(peer.id, text, sensitive)
        Log.i(TAG, "SENDCLIP text to ${peer.name}")
        Bridge.attemptDone(app, reached = true)
        Timeline.add(clipTextEntry(peer.name, text))
        null
    } catch (t: Throwable) {
        Log.e(TAG, "SENDCLIP failed: $t")
        Bridge.attemptDone(app, reached = false)
        outdatedText(t.message, peer.name) ?: "${peer.name} did not receive it"
    }
}

