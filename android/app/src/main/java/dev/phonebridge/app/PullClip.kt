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
import android.graphics.PixelFormat
import android.os.PowerManager
import android.os.SystemClock
import android.view.Gravity
import android.view.View
import android.view.WindowManager
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

/** What an automatic pull does with the clip on the phone (decided from its copy time, never its content). */
enum class AutoVerdict { NOTHING, UNCHANGED, SEND }

/**
 * [copyTs] is when the clip was copied (0: unknown), [lastSent] when the clip last sent by an automatic pull was
 * copied, [pcChangedAgoMs] how long ago the PC's own clipboard changed (0: unknown). Only a clip that is new to the
 * PC and newer than the PC's own copy is sent.
 */
fun autoPullVerdict(copyTs: Long, nowMs: Long, lastSent: Long, pcChangedAgoMs: Long): AutoVerdict = when {
    copyTs <= 0 -> AutoVerdict.NOTHING
    copyTs <= lastSent -> AutoVerdict.UNCHANGED
    pcChangedAgoMs > 0 && nowMs - copyTs >= pcChangedAgoMs -> AutoVerdict.UNCHANGED
    else -> AutoVerdict.SEND
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
    fun handle(ctx: Context, peerId: String, wantImage: Boolean, auto: Boolean = false, pcChangedAgoMs: Long = 0) {
        if (!Prefs(ctx).pcFlag(peerId, "clipboard_sync")) return refuse(peerId, "Share clipboard is off for this PC on the phone")
        val power = ctx.getSystemService(PowerManager::class.java)
        val keyguard = ctx.getSystemService(KeyguardManager::class.java)
        val why = pullRefusal(power.isInteractive, keyguard.isKeyguardLocked, Settings.canDrawOverlays(ctx))
        if (why != null) return refuse(peerId, why)
        if (auto) return readByOverlay(ctx.applicationContext, peerId, wantImage, pcChangedAgoMs)
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

    /**
     * The automatic pull: a 1x1 almost invisible window takes focus for about 100 ms, asks only for the clip's copy
     * time, and reads the content only when there is something new to send. Measured on the realme: reads took
     * 60 to 160 ms over the launcher, YouTube, a game and a chat; the keyboard stayed up; a window with alpha 0
     * never gets focus, and a payment app that refuses overlays gives no focus (answered quietly).
     */
    private fun readByOverlay(app: Context, peerId: String, wantImage: Boolean, pcChangedAgoMs: Long) {
        val wm = app.getSystemService(WindowManager::class.java)
        val cm = app.getSystemService(ClipboardManager::class.java)
        val main = Handler(Looper.getMainLooper())
        main.post {
            var done = false
            val view = object : View(app) {
                override fun onWindowFocusChanged(hasFocus: Boolean) {
                    super.onWindowFocusChanged(hasFocus)
                    if (!hasFocus || done) return
                    done = true
                    try {
                        val desc = cm.primaryClipDescription
                        val ts = desc?.timestamp ?: 0L
                        val prefs = Prefs(app)
                        // No content: only the copy time, its clock base, and the kinds of data, to diagnose "nothing arrives".
                        Log.i(PTAG, "PULL auto: copyTs=$ts wall=${System.currentTimeMillis()} uptime=${SystemClock.elapsedRealtime()} lastSent=${prefs.lastPullSentTs} pcChangedAgo=$pcChangedAgoMs types=${(0 until (desc?.mimeTypeCount ?: 0)).map { desc!!.getMimeType(it) }}")
                        when (autoPullVerdict(ts, System.currentTimeMillis(), prefs.lastPullSentTs, pcChangedAgoMs)) {
                            AutoVerdict.NOTHING -> refuse(peerId, "Nothing is copied on the phone")
                            AutoVerdict.UNCHANGED -> {
                                Log.i(PTAG, "PULL auto: nothing new")
                                Bridge.scope.launch { runCatching { Bridge.awaitNode()?.answerClipboardUnchanged(peerId) } }
                            }
                            AutoVerdict.SEND -> answer(app, peerId, wantImage, auto = true) { prefs.lastPullSentTs = ts }
                        }
                    } finally {
                        runCatching { wm.removeView(this) }
                    }
                }
            }
            val lp = WindowManager.LayoutParams(
                1, 1, WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
                WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL or
                    WindowManager.LayoutParams.FLAG_ALT_FOCUSABLE_IM,
                PixelFormat.TRANSLUCENT,
            ).apply {
                gravity = Gravity.START or Gravity.TOP
                alpha = 0.01f
                softInputMode = WindowManager.LayoutParams.SOFT_INPUT_STATE_UNCHANGED
            }
            val added = runCatching { wm.addView(view, lp) }.isSuccess
            if (!added) {
                done = true
                return@post refuse(peerId, "Relay could not open a window on the phone")
            }
            // Taking focus: the window starts without it so it never appears to the app in front.
            lp.flags = lp.flags and WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE.inv()
            runCatching { wm.updateViewLayout(view, lp) }
            val started = SystemClock.elapsedRealtime()
            main.postDelayed({
                if (!done) {
                    done = true
                    Log.i(PTAG, "PULL auto: no focus after ${SystemClock.elapsedRealtime() - started} ms")
                    runCatching { wm.removeView(view) }
                    refuse(peerId, "Relay could not read the clipboard (no window focus)")
                }
            }, 2500)
        }
    }

    /** The clip on the phone, as an answer to the PC: text, a picture, or the reason there is none. */
    fun answer(app: Context, peerId: String, wantImage: Boolean, auto: Boolean = false, sent: (() -> Unit)? = null) {
        val prefs = Prefs(app)
        if (!prefs.pcFlag(peerId, "clipboard_sync")) return refuse(peerId, "Share clipboard is off for this PC on the phone")
        val clip = app.getSystemService(ClipboardManager::class.java).primaryClip
        val item = clip?.takeIf { it.itemCount > 0 }?.getItemAt(0)
        val uri = item?.uri
        val isImage = clip?.description?.hasMimeType("image/*") == true && uri != null
        val text = if (isImage) null else item?.text?.toString()?.take(60 * 1024)
        val sensitive = clip?.description?.extras?.getBoolean(ClipDescription.EXTRA_IS_SENSITIVE) == true
        when (val a = pullAction(!text.isNullOrEmpty(), isImage, wantImage, prefs.clipboardImages, sensitive, prefs.sendSensitive)) {
            is PullAction.Refuse -> {
                refuse(peerId, a.reason)
                // An automatic pull that was refused for this clip is done with it: asking again would only repeat the
                // refusal (and the notice) every few seconds.
                if (auto) sent?.invoke()
            }
            PullAction.SendText -> {
                // Text the PC itself sent is stamped here a moment after the PC copied it, so the copy time alone
                // can read as newer: an automatic pull never hands the PC its own text back.
                if (auto && ClipFingerprints.unchanged(app, text!!)) {
                    Log.i(PTAG, "PULL auto: that text came from the PC, nothing new")
                    sent?.invoke()
                    Bridge.scope.launch { runCatching { Bridge.awaitNode()?.answerClipboardUnchanged(peerId) } }
                    return
                }
                Log.i(PTAG, "PULL answering with text ${text!!.length} chars")
                Bridge.scope.launch {
                    runCatching { (Bridge.awaitNode() ?: error("Relay is not running")).answerClipboardPull(peerId, text, sensitive) }
                        .onSuccess {
                            ClipFingerprints.remember(app, text)
                            sent?.invoke()
                        }
                        .onFailure { Log.w(PTAG, "PULL text not sent: $it") }
                }
            }
            PullAction.SendImage -> {
                // The grant on the picture ends with this window: read it now.
                val bytes = runCatching { app.contentResolver.openInputStream(uri!!)?.use { it.readCapped(40L * 1024 * 1024) } }.getOrNull()
                if (bytes == null) {
                    refuse(peerId, "The copied picture could not be read")
                    if (auto) sent?.invoke()
                } else {
                    Log.i(PTAG, "PULL answering with a picture")
                    Bridge.scope.launch {
                        val failed = ClipImages.sendBytes(app, bytes, requestedPeerId = peerId)
                        if (failed != null) refuse(peerId, failed).also { if (auto) sent?.invoke() } else sent?.invoke()
                    }
                }
            }
        }
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

    private fun answer(app: Context, peerId: String, wantImage: Boolean) = PullClip.answer(app, peerId, wantImage)
}
