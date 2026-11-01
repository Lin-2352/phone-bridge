package dev.phonebridge.app

import android.content.Context
import android.net.Uri
import android.util.Log
import androidx.core.util.AtomicFile
import java.io.File
import java.io.FileNotFoundException
import java.io.FileOutputStream
import java.io.IOException
import java.math.BigDecimal
import java.math.RoundingMode
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import org.json.JSONArray
import org.json.JSONObject

/*
 * The phone's timeline: a metadata-only record of what moved between this
 * phone and its PCs. Never stored: clipboard text, links, picture bytes, file
 * contents, notification titles or bodies. Text and links keep their
 * character count, pictures their size and dimensions, files their name, size
 * and saved uri, notifications the app name only. Same caps and summary
 * wording as the PC (`crates/pb-tray/src/timeline.rs`).
 *
 * The file is `filesDir/timeline.json`, written through AtomicFile off the
 * main thread. `prune` / `summary` / `encode` / `decode` are pure.
 */

private const val TAG = "PhoneBridge"

enum class TlKind { File, ClipText, ClipImage, Link, Notification }
enum class TlDir { ToPc, ToPhone }

data class TlEntry(
    val kind: TlKind,
    val dir: TlDir,
    val peer: String,
    /** Unix milliseconds. */
    val at: Long,
    val name: String? = null,
    val size: Long? = null,
    val uri: String? = null,
    val chars: Int? = null,
    val width: Int? = null,
    val height: Int? = null,
    val app: String? = null,
)

const val TL_MAX_ENTRIES = 1000
const val TL_MAX_AGE_MS = 30L * 24 * 3600 * 1000
private const val TL_VERSION = 1

// On-disk codes, as on the PC; never enum names, which R8 may rename in a release build.
private fun kindCode(k: TlKind) = when (k) {
    TlKind.File -> "F"
    TlKind.ClipText -> "T"
    TlKind.ClipImage -> "I"
    TlKind.Link -> "L"
    TlKind.Notification -> "N"
}
private fun kindOf(c: String) = TlKind.entries.firstOrNull { kindCode(it) == c } ?: throw IllegalArgumentException("bad kind")
private fun dirCode(d: TlDir) = if (d == TlDir.ToPc) "c" else "p"
private fun dirOf(c: String) = when (c) {
    "c" -> TlDir.ToPc
    "p" -> TlDir.ToPhone
    else -> throw IllegalArgumentException("bad direction")
}

fun fileEntry(peer: String, dir: TlDir, name: String, size: Long, uri: String?, at: Long = System.currentTimeMillis()) =
    TlEntry(TlKind.File, dir, peer, at, name = name, size = size, uri = uri)

/** Characters as the PC counts them (code points, not UTF-16 units). */
private fun charCount(s: String) = s.codePointCount(0, s.length)

/** Takes the text only to count its characters; the text itself is dropped. */
fun clipTextEntry(peer: String, text: String, dir: TlDir = TlDir.ToPc, at: Long = System.currentTimeMillis()) =
    TlEntry(TlKind.ClipText, dir, peer, at, chars = charCount(text))

/** Like [clipTextEntry]: the link's length only, never the address. */
fun linkEntry(peer: String, url: String, dir: TlDir = TlDir.ToPc, at: Long = System.currentTimeMillis()) =
    TlEntry(TlKind.Link, dir, peer, at, chars = charCount(url))

/** A picture's byte size and dimensions (see [pngDims]), never its bytes. */
fun clipImageEntry(peer: String, dir: TlDir, size: Long, dims: Pair<Int, Int>?, at: Long = System.currentTimeMillis()) =
    TlEntry(TlKind.ClipImage, dir, peer, at, size = size, width = dims?.first, height = dims?.second)

/** The source app's name only: never the title or text. */
fun notificationEntry(peer: String, app: String, at: Long = System.currentTimeMillis()) =
    TlEntry(TlKind.Notification, TlDir.ToPc, peer, at, app = app)

/** Width and height from a PNG's IHDR (its first 24 bytes), or null. */
fun pngDims(head: ByteArray): Pair<Int, Int>? {
    if (head.size < 24 || !isPngBytes(head)) return null
    fun be(i: Int) = ((head[i].toInt() and 0xFF) shl 24) or ((head[i + 1].toInt() and 0xFF) shl 16) or
        ((head[i + 2].toInt() and 0xFF) shl 8) or (head[i + 3].toInt() and 0xFF)
    val w = be(16)
    val h = be(20)
    return if (w > 0 && h > 0) w to h else null
}

/** Drops entries older than 30 days, then keeps the newest 1,000 (newest first, stable). */
fun prune(list: List<TlEntry>, nowMs: Long): List<TlEntry> {
    val oldest = nowMs - TL_MAX_AGE_MS
    return list.filter { it.at >= oldest }.sortedByDescending { it.at }.take(TL_MAX_ENTRIES)
}

/** Rust's `{:.1}`: ties round to even on the exact binary value. */
private fun oneDecimal(v: Double) = BigDecimal(v).setScale(1, RoundingMode.HALF_EVEN).toPlainString()

private fun humanSize(n: Long): String = when {
    n < 1000 -> "$n B"
    n < 1_000_000 -> "${oneDecimal(n / 1e3)} KB"
    n < 1_000_000_000 -> "${oneDecimal(n / 1e6)} MB"
    else -> "${oneDecimal(n / 1e9)} GB"
}

private fun count(label: String, n: Int?) = when (n) {
    null -> label
    1 -> "$label, 1 character"
    else -> "$label, $n characters"
}

/** One line for a timeline row, worded as on the PC. Links (the PC has none) read like text. */
fun summary(e: TlEntry): String = when (e.kind) {
    TlKind.File -> {
        val name = e.name ?: "File"
        e.size?.let { "$name, ${humanSize(it)}" } ?: name
    }
    TlKind.ClipText -> count("Text", e.chars)
    TlKind.Link -> count("Link", e.chars)
    TlKind.ClipImage -> buildString {
        append("Picture")
        if (e.width != null && e.height != null) append(", ${e.width} × ${e.height}")
        e.size?.let { append(", ${humanSize(it)}") }
    }
    TlKind.Notification -> "${e.app ?: "App"} notification"
}

/** `{"v":1,"e":[...]}`, newest first; absent fields are left out. */
fun encode(list: List<TlEntry>): String {
    val arr = JSONArray()
    for (e in list) {
        val o = JSONObject()
            .put("k", kindCode(e.kind))
            .put("d", dirCode(e.dir))
            .put("peer", e.peer)
            .put("at", e.at)
        e.name?.let { o.put("name", it) }
        e.size?.let { o.put("size", it) }
        e.uri?.let { o.put("uri", it) }
        e.chars?.let { o.put("chars", it) }
        e.width?.let { o.put("w", it) }
        e.height?.let { o.put("h", it) }
        e.app?.let { o.put("app", it) }
        arr.put(o)
    }
    return JSONObject().put("v", TL_VERSION).put("e", arr).toString()
}

/** The entries, or null if [s] is not a timeline this version can read. */
fun decodeOrNull(s: String): List<TlEntry>? = try {
    val root = JSONObject(s)
    if (root.getInt("v") != TL_VERSION) {
        null
    } else {
        val arr = root.getJSONArray("e")
        (0 until arr.length()).map { i ->
            val o = arr.getJSONObject(i)
            fun str(k: String) = if (o.has(k) && !o.isNull(k)) o.getString(k) else null
            fun int(k: String) = if (o.has(k) && !o.isNull(k)) o.getInt(k) else null
            val w = int("w")
            val h = int("h")
            require((w == null) == (h == null)) { "half a size" }
            TlEntry(
                kind = kindOf(o.getString("k")),
                dir = dirOf(o.getString("d")),
                peer = o.getString("peer"),
                at = o.getLong("at"),
                name = str("name"),
                size = if (o.has("size") && !o.isNull("size")) o.getLong("size") else null,
                uri = str("uri"),
                chars = int("chars"),
                width = w,
                height = h,
                app = str("app"),
            )
        }
    }
} catch (e: Exception) {
    // JSONException, an unknown kind or direction, half a size.
    null
}

/** Corrupt or unknown content reads as empty. */
fun decode(s: String): List<TlEntry> = decodeOrNull(s) ?: emptyList()

/**
 * Whether the file at [uri] (content:// or file://) can still be opened.
 * Touches storage or a content provider: call it off the main thread.
 */
fun fileState(ctx: Context, uri: String): Boolean = try {
    val u = Uri.parse(uri)
    if (u.scheme == "file") {
        u.path?.let { File(it).canRead() } ?: false
    } else {
        ctx.contentResolver.openFileDescriptor(u, "r")?.use { true } ?: false
    }
} catch (e: SecurityException) {
    false // the grant is gone
} catch (e: FileNotFoundException) {
    false
} catch (e: Exception) {
    false // provider gone, bad uri
}

/**
 * The process-wide store. [init] loads the file once (from the service or
 * the UI, whichever starts first); [add] updates [entries] at once and writes
 * the file shortly after on a single I/O thread, so bursts coalesce into one
 * write. Every file operation runs on that thread, in order, so a write
 * queued before [clear] can never bring the history back.
 *
 * Read errors: a missing file is empty; unparsable content is logged and
 * replaced; any other read failure keeps the file untouched for this session
 * (recording continues in memory only), as on the PC.
 */
object Timeline {
    private const val FILE = "timeline.json"
    private const val WRITE_DELAY_MS = 300L

    private val _entries = MutableStateFlow<List<TlEntry>>(emptyList())
    /** Newest first. */
    val entries: StateFlow<List<TlEntry>> = _entries.asStateFlow()

    private val io = Executors.newSingleThreadScheduledExecutor { r -> Thread(r, "timeline-io").apply { isDaemon = true } }
    private val writeQueued = AtomicBoolean(false)
    @Volatile private var file: AtomicFile? = null
    @Volatile private var app: Context? = null
    /** "Keep history". Before [init] entries are kept in memory and merged with the file. */
    @Volatile private var enabled = true
    /** The file could not be read (not just parsed): never overwrite it this session. I/O thread only. */
    private var writeLocked = false

    fun init(ctx: Context) {
        synchronized(this) {
            if (file != null) return
            app = ctx.applicationContext
            file = AtomicFile(File(ctx.applicationContext.filesDir, FILE))
            enabled = Prefs(ctx).keepHistory
        }
        if (!enabled) clear() else io.execute { load() }
    }

    fun add(e: TlEntry) {
        if (!enabled) return
        _entries.update { prune(listOf(e) + it, System.currentTimeMillis()) }
        scheduleWrite()
    }

    /** Deletes the history (memory and file). */
    fun clear() {
        _entries.value = emptyList()
        io.execute {
            writeLocked = false // the user asked for it gone
            file?.delete()
        }
    }

    /** "Keep history": off deletes the history and stops recording; on starts again. */
    fun setEnabled(on: Boolean) {
        enabled = on
        app?.let { Prefs(it).keepHistory = on }
        if (!on) clear()
    }

    private fun scheduleWrite() {
        if (writeQueued.compareAndSet(false, true)) {
            io.schedule({ writeQueued.set(false); write() }, WRITE_DELAY_MS, TimeUnit.MILLISECONDS)
        }
    }

    private fun load() {
        val f = file ?: return
        val loaded = try {
            val text = f.readFully().toString(Charsets.UTF_8)
            decodeOrNull(text) ?: run {
                Log.e(TAG, "TIMELINE unreadable, starting empty")
                f.delete()
                emptyList()
            }
        } catch (e: FileNotFoundException) {
            emptyList()
        } catch (e: IOException) {
            // The history may be fine on disk: leave it alone this session.
            writeLocked = true
            Log.e(TAG, "TIMELINE read failed: $e")
            emptyList()
        }
        if (!enabled) return
        var pending = false
        _entries.update { cur ->
            pending = cur.isNotEmpty()
            prune(cur + loaded, System.currentTimeMillis())
        }
        // Entries added before the load finished are not on disk yet.
        if (pending) scheduleWrite()
    }

    private fun write() {
        val f = file ?: return
        if (!enabled || writeLocked) return
        val snapshot = _entries.value
        var out: FileOutputStream? = null
        try {
            out = f.startWrite()
            out.write(encode(snapshot).toByteArray(Charsets.UTF_8))
            f.finishWrite(out)
        } catch (e: IOException) {
            // The previous file stays; the entries stay in memory for the next write.
            out?.let { f.failWrite(it) }
            Log.e(TAG, "TIMELINE write failed: $e")
        }
    }
}
