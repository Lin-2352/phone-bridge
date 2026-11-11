package dev.phonebridge.app

import android.app.Activity
import android.app.KeyguardManager
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.PowerManager
import android.provider.Settings
import android.util.Log
import kotlinx.coroutines.launch

private const val PTAG = "PhoneBridge"

/** Why the phone cannot answer a PC's "get your clipboard" right now (null: it can try). */
fun pullRefusal(interactive: Boolean, locked: Boolean, overlay: Boolean): String? = when {
    !interactive -> "The phone's screen is off. Wake it and try again"
    locked -> "The phone is locked. Unlock it and try again"
    !overlay -> "Allow Display over other apps for Relay on the phone so a PC can ask for its clipboard"
    else -> null
}

/** What to send for what is on the clipboard (the PC may only ask; the phone's switches decide). */
sealed class PullAction {
    object SendText : PullAction()
    object SendImage : PullAction()
    data class Refuse(val reason: String) : PullAction()
}

fun pullAction(hasText: Boolean, hasImage: Boolean, wantImage: Boolean, imagesOn: Boolean, sensitive: Boolean, sendSensitive: Boolean): PullAction = when {
    hasImage && wantImage && imagesOn -> PullAction.SendImage
    hasImage -> PullAction.Refuse("A picture is copied, but picture sharing is off")
    !hasText -> PullAction.Refuse("Nothing is copied on the phone")
    sensitive && !sendSensitive -> PullAction.Refuse("That text is marked private; turn on Passwords and codes to send it")
    else -> PullAction.SendText
}

/** The PC asked for the clipboard: open [PullClipActivity] (Android lets only the focused app read it) or say why not. */
object PullClip {
    fun handle(ctx: Context, peerId: String, wantImage: Boolean) {
        val power = ctx.getSystemService(PowerManager::class.java)
        val keyguard = ctx.getSystemService(KeyguardManager::class.java)
        val why = pullRefusal(power.isInteractive, keyguard.isKeyguardLocked, Settings.canDrawOverlays(ctx))
        if (why != null) return refuse(peerId, why)
        val ok = runCatching {
            ctx.startActivity(
                Intent(ctx, PullClipActivity::class.java)
                    .putExtra("peer", peerId)
                    .putExtra("image", wantImage)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_NO_ANIMATION or Intent.FLAG_ACTIVITY_NO_HISTORY),
            )
        }.onFailure { Log.w(PTAG, "PULL could not open its window: $it") }.isSuccess
        if (!ok) refuse(peerId, "Relay could not open a window on the phone")
    }

    fun refuse(peerId: String, reason: String) {
        Log.i(PTAG, "PULL refused: $reason")
        Bridge.scope.launch { runCatching { Bridge.awaitNode()?.refuseClipboardPull(peerId, reason) } }
    }
}

/** See-through and short: takes focus, reads the clipboard, answers the PC and closes. */
class PullClipActivity : Activity() {
    private var done = false
    private val main = Handler(Looper.getMainLooper())

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        overridePendingTransition(0, 0)
        // Never leave the PC waiting if focus does not come.
        main.postDelayed({
            if (!done) {
                done = true
                PullClip.refuse(intent.getStringExtra("peer").orEmpty(), "Relay could not read the clipboard (no window focus)")
                finish()
            }
        }, 2500)
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (!hasFocus || done) return
        done = true
        main.removeCallbacksAndMessages(null)
        answer(applicationContext, intent.getStringExtra("peer").orEmpty(), intent.getBooleanExtra("image", false))
        finish()
        overridePendingTransition(0, 0)
    }

    private fun answer(app: Context, peerId: String, wantImage: Boolean) {
        val prefs = Prefs(app)
        val clip = app.getSystemService(ClipboardManager::class.java).primaryClip
        val item = clip?.takeIf { it.itemCount > 0 }?.getItemAt(0)
        val uri = item?.uri
        val isImage = clip?.description?.hasMimeType("image/*") == true && uri != null
        val text = if (isImage) null else item?.text?.toString()?.take(60 * 1024)
        val sensitive = clip?.description?.extras?.getBoolean(ClipDescription.EXTRA_IS_SENSITIVE) == true
        when (val a = pullAction(!text.isNullOrEmpty(), isImage, wantImage, prefs.clipboardImages, sensitive, prefs.sendSensitive)) {
            is PullAction.Refuse -> PullClip.refuse(peerId, a.reason)
            PullAction.SendText -> {
                Log.i(PTAG, "PULL answering with text ${text!!.length} chars")
                Bridge.scope.launch {
                    runCatching { Bridge.awaitNode()?.answerClipboardPull(peerId, text, sensitive) }
                        .onSuccess { ClipFingerprints.remember(app, text) }
                        .onFailure { Log.w(PTAG, "PULL text not sent: $it") }
                }
            }
            PullAction.SendImage -> {
                // The grant on the picture ends with this window: read it now.
                val bytes = runCatching { app.contentResolver.openInputStream(uri!!)?.use { it.readCapped(40L * 1024 * 1024) } }.getOrNull()
                if (bytes == null) PullClip.refuse(peerId, "The copied picture could not be read") else {
                    Log.i(PTAG, "PULL answering with a picture")
                    Bridge.scope.launch { ClipImages.sendBytes(app, bytes)?.let { PullClip.refuse(peerId, it) } }
                }
            }
        }
    }
}
