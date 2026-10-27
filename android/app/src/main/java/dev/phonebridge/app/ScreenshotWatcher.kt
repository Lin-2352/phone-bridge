package dev.phonebridge.app

import android.content.Context
import android.database.ContentObserver
import android.net.Uri
import android.os.Handler
import android.os.HandlerThread
import android.provider.MediaStore
import android.util.Log
import kotlinx.coroutines.launch

private const val TAG = "PhoneBridge"

/**
 * Sends each new screenshot to the PC. Started by the service while the user's
 * switch is on, Android's picture access is granted and the node runs; it watches
 * the gallery index and only acts on rows [isScreenshot] accepts.
 */
object ScreenshotWatcher {
    private var thread: HandlerThread? = null
    private var observer: ContentObserver? = null
    private var seen = emptyList<Long>()

    /** Start or stop to match [Prefs.shareScreenshots] and the access Android has granted. */
    @Synchronized
    fun sync(ctx: Context) {
        val app = ctx.applicationContext
        val want = Prefs(app).shareScreenshots && ClipImages.hasImageAccess(app)
        if (want && observer == null) start(app)
        if (!want && observer != null) stop(app)
    }

    @Synchronized
    fun stop(ctx: Context) {
        observer?.let { ctx.applicationContext.contentResolver.unregisterContentObserver(it) }
        observer = null
        thread?.quitSafely()
        thread = null
    }

    private fun start(app: Context) {
        val t = HandlerThread("screenshots").apply { start() }
        val obs = object : ContentObserver(Handler(t.looper)) {
            override fun onChange(selfChange: Boolean) = check(app)
        }
        app.contentResolver.registerContentObserver(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, true, obs)
        thread = t
        observer = obs
        Log.i(TAG, "SCREENSHOTS watching")
    }

    @Synchronized
    private fun check(app: Context) {
        val cols = arrayOf(
            MediaStore.Images.Media._ID,
            MediaStore.Images.Media.RELATIVE_PATH,
            MediaStore.Images.Media.DISPLAY_NAME,
            MediaStore.Images.Media.DATE_ADDED,
            MediaStore.Images.Media.IS_PENDING,
            MediaStore.Images.Media.SIZE,
        )
        val rows = mutableListOf<MediaRow>()
        runCatching {
            app.contentResolver.query(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, cols, null, null, "${MediaStore.Images.Media.DATE_ADDED} DESC")?.use { c ->
                while (c.moveToNext() && rows.size < 5) {
                    rows += MediaRow(c.getLong(0), c.getString(1), c.getString(2), c.getLong(3), c.getInt(4) != 0, c.getLong(5))
                }
            }
        }.onFailure { Log.e(TAG, "SCREENSHOTS query failed: $it") }
        val now = System.currentTimeMillis() / 1000
        for (row in rows.filter { isScreenshot(it, now, seen.toSet()) }) {
            seen = rememberSeen(seen, row.id)
            val uri = Uri.withAppendedPath(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, row.id.toString())
            Bridge.scope.launch {
                val problem = ClipImages.send(app, uri)
                Log.i(TAG, "SCREENSHOTS ${row.displayName}: ${problem ?: "sent"}")
            }
        }
    }
}
