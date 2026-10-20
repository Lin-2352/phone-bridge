package dev.phonebridge.app

import android.app.ActivityOptions
import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Person
import android.app.RemoteInput
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.drawable.BitmapDrawable
import android.graphics.drawable.Drawable
import android.media.AudioManager
import android.os.Build
import android.os.Bundle
import android.os.Parcelable
import android.provider.Settings
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import android.util.Log
import androidx.compose.runtime.mutableStateOf
import java.io.ByteArrayOutputStream
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors
import uniffi.pb_ffi.NotifyHandler
import uniffi.pb_ffi.PbException
import uniffi.pb_ffi.PbNotification
import uniffi.pb_ffi.PbNotifyActionDesc
import uniffi.pb_ffi.PbNotifyActionKind

private const val TAG = "PhoneBridge"
private const val ICON_PX = 96
private const val ICON_MAX = 16 * 1024
private const val PICTURE_PX = 256
private const val PICTURE_MAX = 48 * 1024
private const val MAX_LINES = 5

/** Short id for logs: says nothing about the notification's content. */
fun keyHash(key: String): String = Integer.toHexString(key.hashCode())

fun hasNotifyAccess(ctx: Context): Boolean =
    ctx.getSystemService(NotificationManager::class.java)
        .isNotificationListenerAccessGranted(ComponentName(ctx, NotifyListener::class.java))

/**
 * "Notifications on PC": hands the phone's notifications to the node, which
 * sends them to paired PCs. The user grants access once in Settings.
 */
class NotifyListener : NotificationListenerService() {
    companion object {
        @Volatile var instance: NotifyListener? = null
        val connected = mutableStateOf(false)

        /** key -> live notification, for actions a PC asks for. */
        val live = ConcurrentHashMap<String, StatusBarNotification>()

        /** Apps whose icon the node already has (sent once per process). */
        private val iconSent: MutableSet<String> = ConcurrentHashMap.newKeySet()
    }

    /** Bitmaps and the node call stay off the main thread. */
    private val bg = Executors.newSingleThreadExecutor()

    override fun onListenerConnected() {
        instance = this
        connected.value = true
        Log.i(TAG, "NOTIFY listener connected")
        seedAll()
    }

    override fun onListenerDisconnected() {
        connected.value = false
        instance = null
        Log.i(TAG, "NOTIFY listener disconnected")
        requestRebind(ComponentName(this, NotifyListener::class.java))
    }

    override fun onDestroy() {
        if (instance === this) instance = null
        connected.value = false
        bg.shutdown()
        super.onDestroy()
    }

    /** What is on the phone already: known to the node for sync, not sent. */
    fun seedAll() {
        val node = Bridge.node ?: return
        val active = try { activeNotifications } catch (_: Throwable) { null } ?: return
        bg.execute {
            try {
                val list = active.filterNot(::skip).map {
                    live[it.key] = it
                    toPb(it, quiet = true, withMedia = false)
                }
                node.notifySeed(list)
            } catch (t: Throwable) {
                Log.e(TAG, "NOTIFY seed failed: $t")
            }
        }
    }

    override fun onNotificationPosted(sbn: StatusBarNotification, rankingMap: RankingMap?) {
        if (skip(sbn)) {
            // An incoming call that was answered becomes an ongoing call:
            // the call toast goes, and a silenced ringer comes back.
            if (live.remove(sbn.key) != null) {
                if (sbn.notification.category == Notification.CATEGORY_CALL) restoreRinger(this)
                val node = Bridge.node ?: return
                bg.execute { runCatching { node.notifyRemoved(sbn.key) } }
            }
            return
        }
        val update = live.put(sbn.key, sbn) != null
        val node = Bridge.node ?: return
        if (!Prefs(this).notifyToPc) return
        val r = Ranking()
        val ranked = rankingMap?.getRanking(sbn.key, r) == true
        val onlyOnce = sbn.notification.flags and Notification.FLAG_ONLY_ALERT_ONCE != 0
        val quiet = (ranked && (r.importance < NotificationManager.IMPORTANCE_DEFAULT || !r.matchesInterruptionFilter())) ||
            (update && onlyOnce)
        bg.execute {
            try {
                node.notifyPosted(toPb(sbn, quiet, withMedia = true))
                Log.i(TAG, "NOTIFY posted pkg=${sbn.packageName} key=${keyHash(sbn.key)}")
            } catch (t: Throwable) {
                Log.e(TAG, "NOTIFY post failed pkg=${sbn.packageName}: $t")
            }
        }
    }

    override fun onNotificationRemoved(sbn: StatusBarNotification, rankingMap: RankingMap?) {
        live.remove(sbn.key)
        if (sbn.notification.category == Notification.CATEGORY_CALL) restoreRinger(this)
        if (skip(sbn)) return
        val node = Bridge.node ?: return
        bg.execute {
            try {
                node.notifyRemoved(sbn.key)
            } catch (t: Throwable) {
                Log.e(TAG, "NOTIFY remove failed: $t")
            }
        }
    }

    /** A ringing incoming call (dialers post these as ongoing). */
    private fun ringing(n: Notification): Boolean =
        n.category == Notification.CATEGORY_CALL && (
            n.fullScreenIntent != null ||
                (Build.VERSION.SDK_INT >= 31 && n.extras.getInt(Notification.EXTRA_CALL_TYPE) == Notification.CallStyle.CALL_TYPE_INCOMING)
            )

    private fun skip(sbn: StatusBarNotification): Boolean {
        val n = sbn.notification
        return (sbn.isOngoing && !ringing(n)) ||
            n.flags and Notification.FLAG_GROUP_SUMMARY != 0 ||
            sbn.packageName == packageName
    }

    private fun appLabel(pkg: String): String = try {
        packageManager.getApplicationLabel(packageManager.getApplicationInfo(pkg, 0)).toString()
    } catch (_: Throwable) {
        pkg
    }

    private fun toPb(sbn: StatusBarNotification, quiet: Boolean, withMedia: Boolean): PbNotification {
        val nf = sbn.notification
        val app = appLabel(sbn.packageName)
        val secret = nf.visibility == Notification.VISIBILITY_SECRET
        val usePublic = nf.visibility == Notification.VISIBILITY_PRIVATE && nf.publicVersion != null
        val ex = (if (usePublic) nf.publicVersion else nf).extras
        val private = secret || usePublic
        val title = if (secret) app else ex.getCharSequence(Notification.EXTRA_TITLE)?.toString() ?: app
        val text = if (secret) {
            "New notification"
        } else {
            (ex.getCharSequence(Notification.EXTRA_BIG_TEXT) ?: ex.getCharSequence(Notification.EXTRA_TEXT))
                ?.toString() ?: ""
        }
        val messages = if (private) emptyList() else messages(nf.extras)
        val conversation = if (messages.isEmpty()) {
            ""
        } else {
            nf.shortcutId ?: nf.extras.getCharSequence(Notification.EXTRA_CONVERSATION_TITLE)?.toString() ?: title
        }
        val actions = nf.actions.orEmpty().map { a ->
            PbNotifyActionDesc(a.title?.toString() ?: "", a.remoteInputs?.any { it.allowFreeFormInput } == true)
        }
        val icon = if (withMedia && iconSent.add(sbn.packageName)) appIcon(sbn.packageName) else null
        val picture = if (withMedia && !private) picture(nf) else null
        return PbNotification(
            sbn.key,
            sbn.packageName,
            app,
            title,
            text,
            nf.`when`.takeIf { it > 0 } ?: sbn.postTime,
            conversation,
            messages,
            quiet,
            private,
            nf.category == Notification.CATEGORY_CALL,
            actions,
            icon,
            picture,
        )
    }

    /** The latest messages of a MessagingStyle notification as "sender: text". */
    private fun messages(ex: Bundle): List<String> {
        val arr: Array<out Parcelable>? = if (Build.VERSION.SDK_INT >= 33) {
            ex.getParcelableArray(Notification.EXTRA_MESSAGES, Parcelable::class.java)
        } else {
            @Suppress("DEPRECATION")
            ex.getParcelableArray(Notification.EXTRA_MESSAGES)
        }
        return arr.orEmpty().mapNotNull { p ->
            val b = p as? Bundle ?: return@mapNotNull null
            val text = b.getCharSequence("text")?.toString() ?: return@mapNotNull null
            val person = if (Build.VERSION.SDK_INT >= 33) {
                b.getParcelable("sender_person", Person::class.java)
            } else {
                @Suppress("DEPRECATION")
                b.getParcelable("sender_person")
            }
            val sender = person?.name?.toString() ?: b.getCharSequence("sender")?.toString()
            if (sender.isNullOrEmpty()) text else "$sender: $text"
        }.takeLast(MAX_LINES)
    }

    private fun appIcon(pkg: String): ByteArray? = try {
        val bmp = drawableBitmap(packageManager.getApplicationIcon(pkg), ICON_PX)
        val out = ByteArrayOutputStream()
        bmp.compress(Bitmap.CompressFormat.PNG, 100, out)
        out.toByteArray().takeIf { it.size <= ICON_MAX }
    } catch (_: Throwable) {
        null
    }

    /** Photo or avatar: longest side 256 px, JPEG of at most 48 KiB. */
    private fun picture(nf: Notification): ByteArray? = try {
        val ex = nf.extras
        val bmp: Bitmap? = if (Build.VERSION.SDK_INT >= 33) {
            ex.getParcelable(Notification.EXTRA_PICTURE, Bitmap::class.java)
        } else {
            @Suppress("DEPRECATION")
            ex.getParcelable(Notification.EXTRA_PICTURE)
        } ?: nf.getLargeIcon()?.loadDrawable(this)?.let { drawableBitmap(it, PICTURE_PX) }
        bmp?.let { jpeg(scaled(it, PICTURE_PX)) }
    } catch (_: Throwable) {
        null
    }

    private fun drawableBitmap(d: Drawable, px: Int): Bitmap {
        if (d is BitmapDrawable && d.bitmap != null) return scaled(d.bitmap, px)
        val bmp = Bitmap.createBitmap(px, px, Bitmap.Config.ARGB_8888)
        val c = Canvas(bmp)
        d.setBounds(0, 0, px, px)
        d.draw(c)
        return bmp
    }

    private fun scaled(b: Bitmap, px: Int): Bitmap {
        val longest = maxOf(b.width, b.height)
        if (longest <= px) return b
        val f = px.toFloat() / longest
        return Bitmap.createScaledBitmap(b, (b.width * f).toInt().coerceAtLeast(1), (b.height * f).toInt().coerceAtLeast(1), true)
    }

    private fun jpeg(b: Bitmap): ByteArray? {
        var q = 80
        while (q >= 40) {
            val out = ByteArrayOutputStream()
            b.compress(Bitmap.CompressFormat.JPEG, q, out)
            if (out.size() <= PICTURE_MAX) return out.toByteArray()
            q -= 10
        }
        return null
    }
}

/** Put the ringer back after a call that a PC silenced. */
fun restoreRinger(ctx: Context) {
    val prefs = Prefs(ctx)
    val saved = prefs.savedRinger
    if (saved < 0) return
    try {
        ctx.getSystemService(AudioManager::class.java).ringerMode = saved
    } catch (t: Throwable) {
        Log.e(TAG, "NOTIFY ringer restore failed: $t")
    }
    prefs.savedRinger = -1
}

/** Runs what a PC asked for on a notification. Errors go back to the PC. */
object NotifyActions : NotifyHandler {
    private const val GONE = "That notification was cleared on the phone"

    private fun fail(reason: String): Nothing = throw PbException.Failed(reason)

    override fun onAction(key: String, kind: PbNotifyActionKind, index: UInt, text: String) {
        val svc = NotifyListener.instance ?: fail("Notification access is off on the phone")
        val sbn = NotifyListener.live[key]
        Log.i(TAG, "NOTIFY action $kind pkg=${sbn?.packageName} key=${keyHash(key)}")
        try {
            when (kind) {
                PbNotifyActionKind.DISMISS -> svc.cancelNotification(key)
                PbNotifyActionKind.BUTTON -> action(sbn, index).actionIntent.send()
                PbNotifyActionKind.REPLY -> {
                    val a = action(sbn, index)
                    val inputs = a.remoteInputs ?: fail(GONE)
                    val input = inputs.firstOrNull { it.allowFreeFormInput } ?: fail(GONE)
                    val intent = Intent()
                    RemoteInput.addResultsToIntent(inputs, intent, Bundle().apply { putCharSequence(input.resultKey, text) })
                    a.actionIntent.send(svc, 0, intent)
                }
                PbNotifyActionKind.OPEN -> {
                    val pi = sbn?.notification?.contentIntent ?: fail(GONE)
                    val options = if (Build.VERSION.SDK_INT >= 34) {
                        ActivityOptions.makeBasic()
                            .setPendingIntentBackgroundActivityStartMode(ActivityOptions.MODE_BACKGROUND_ACTIVITY_START_ALLOWED)
                            .toBundle()
                    } else {
                        null
                    }
                    pi.send(svc, 0, null, null, null, null, options)
                }
                PbNotifyActionKind.SILENCE -> silence(svc)
            }
        } catch (e: PendingIntent.CanceledException) {
            fail(GONE)
        }
    }

    private fun action(sbn: StatusBarNotification?, index: UInt): Notification.Action =
        sbn?.notification?.actions?.getOrNull(index.toInt()) ?: fail(GONE)

    private fun silence(ctx: Context) {
        val nm = ctx.getSystemService(NotificationManager::class.java)
        if (!nm.isNotificationPolicyAccessGranted) {
            askPolicyAccess(ctx)
            fail("Allow Do Not Disturb access on the phone first")
        }
        val audio = ctx.getSystemService(AudioManager::class.java)
        // Already on vibrate or silent: nothing to do (never make it louder).
        if (audio.ringerMode != AudioManager.RINGER_MODE_NORMAL) return
        val prefs = Prefs(ctx)
        if (prefs.savedRinger < 0) prefs.savedRinger = audio.ringerMode
        audio.ringerMode = AudioManager.RINGER_MODE_VIBRATE
    }

    private fun askPolicyAccess(ctx: Context) {
        val nm = ctx.getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(
            android.app.NotificationChannel("events", "Files and clipboard", NotificationManager.IMPORTANCE_DEFAULT),
        )
        val open = PendingIntent.getActivity(
            ctx,
            7,
            Intent(Settings.ACTION_NOTIFICATION_POLICY_ACCESS_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            PendingIntent.FLAG_IMMUTABLE,
        )
        val n = Notification.Builder(ctx, "events")
            .setSmallIcon(R.drawable.ic_stat_bridge)
            .setContentTitle("Allow Do Not Disturb access")
            .setContentText("So your PC can silence calls")
            .setContentIntent(open)
            .setAutoCancel(true)
            .build()
        nm.notify(260, n)
    }
}
