package dev.phonebridge.app

import android.content.Context
import android.net.Uri
import android.util.Log
import androidx.core.util.AtomicFile
import java.io.File
import java.io.FileNotFoundException
import java.math.BigDecimal
import java.math.RoundingMode
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
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

enum class TlKind { File, ClipText, ClipImage, Link, Notification, Audio }
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
    TlKind.Audio -> "A"
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

/**
 * A finished audio session: who, which way, how long, never any audio. [title] is the session's name
 * ([audioTitle]); the length in seconds goes in `chars`, as on the PC.
 */
fun audioEntry(peer: String, dir: TlDir, title: String, seconds: Long, at: Long = System.currentTimeMillis()) =
    TlEntry(TlKind.Audio, dir, peer, at, name = title, chars = seconds.coerceIn(0, Int.MAX_VALUE.toLong()).toInt())

/** The session title, worded as on the PC. */
fun audioTitle(dir: TlDir) = if (dir == TlDir.ToPc) "Phone audio on this PC" else "PC audio on the phone"

/** "12 s", "4 min 12 s", "1 h 3 min": as the PC words a length. */
fun durationText(secs: Long): String = when {
    secs < 60 -> "$secs s"
    secs < 3600 -> if (secs % 60 == 0L) "${secs / 60} min" else "${secs / 60} min ${secs % 60} s"
    else -> if (secs % 3600 / 60 == 0L) "${secs / 3600} h" else "${secs / 3600} h ${secs % 3600 / 60} min"
}

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

internal fun humanSize(n: Long): String = when {
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

/** One line for a timeline row, worded as on the PC. Links (the PC has none) read "Link, N characters". */
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
    TlKind.Audio -> {
        val title = e.name ?: "Audio"
        e.chars?.let { "$title, ${durationText(it.toLong())}" } ?: title
    }
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
    // JSONException, an unknown kind or direction, half a size. The whole file
    // counts as unreadable and is replaced, as on the PC (`parse` fails on any
    // bad line), rather than half loaded.
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

/** The timeline file's three operations; [Timeline] uses AtomicFile, tests a fake. */
interface TlFile {
    /** The whole file; throws FileNotFoundException when there is none. */
    fun read(): ByteArray
    /** Replaces the file atomically: on any failure the previous file stays. */
    fun write(b: ByteArray)
    fun delete()
}

/** [TlFile] over `androidx.core.util.AtomicFile`. */
private class AtomicTlFile(private val af: AtomicFile) : TlFile {
    override fun read(): ByteArray = af.readFully()
    override fun write(b: ByteArray) {
        val out = af.startWrite()
        try {
            out.write(b)
            af.finishWrite(out)
        } catch (t: Throwable) {
            af.failWrite(out) // closes the stream, keeps the previous file
            throw t
        }
    }
    override fun delete() = af.delete()
}

/**
 * The store's logic, with the file and the I/O thread injected so tests can
 * run it in a fixed order. [exec] runs a task on the I/O thread, [later] runs
 * one there after a short delay (the coalesced write). Every file operation
 * goes through them, in order.
 *
 * Read errors: a missing file is empty; unparsable content (bad JSON, an
 * unknown version, kind or direction) is logged and replaced, as on the PC;
 * any other read failure keeps the file untouched for this session
 * (recording continues in memory only), as on the PC.
 */
class TimelineStore(
    private val exec: (Runnable) -> Unit,
    private val later: (Runnable) -> Unit,
    private val log: (String) -> Unit,
    private val now: () -> Long = System::currentTimeMillis,
) {
    private val lock = Any()
    private val _entries = MutableStateFlow<List<TlEntry>>(emptyList())
    /** Newest first. */
    val entries: StateFlow<List<TlEntry>> = _entries.asStateFlow()

    @Volatile private var file: TlFile? = null
    /** "Keep history". Changed only under [lock]. */
    @Volatile private var enabled = true
    /** Bumped by every clear: a load queued before it must not bring the history back. Under [lock]. */
    private var generation = 0
    private val writeQueued = AtomicBoolean(false)
    /** The file could not be read (not just parsed): never overwrite it this session. I/O thread only. */
    private var writeLocked = false

    /** Loads [f] (or deletes it if history is off). Entries added before this are kept and merged. */
    fun start(f: TlFile, on: Boolean) {
        file = f
        if (!on) {
            setEnabled(false)
            return
        }
        val gen = synchronized(lock) { generation }
        exec(Runnable { load(f, gen) })
    }

    fun add(e: TlEntry) {
        synchronized(lock) {
            // Checked under the lock setEnabled(false) takes: an add can never land after it.
            if (!enabled) return
            _entries.value = prune(listOf(e) + _entries.value, now())
        }
        scheduleWrite()
    }

    /** Removes one entry (the first equal one); false if it is not there. */
    fun remove(e: TlEntry): Boolean {
        val removed = synchronized(lock) {
            val cur = _entries.value
            val i = cur.indexOf(e)
            if (i < 0) return false
            _entries.value = cur.toMutableList().also { it.removeAt(i) }
            true
        }
        if (removed) scheduleWrite()
        return removed
    }

    /** Puts a removed entry back (Undo), in time order; nothing while history is off. */
    fun restore(e: TlEntry) = add(e)

    /** Deletes the history (memory and file). */
    fun clear() {
        synchronized(lock) {
            generation++
            _entries.value = emptyList()
        }
        exec(Runnable {
            writeLocked = false // the user asked for it gone
            file?.delete()
        })
    }

    /** Off deletes the history and stops recording; on starts again. */
    fun setEnabled(on: Boolean) {
        synchronized(lock) { enabled = on }
        if (!on) clear()
    }

    /** On the I/O thread: write now if a write is waiting (the delayed write, or on exit). */
    fun flushNow() {
        if (writeQueued.getAndSet(false)) write()
    }

    fun hasPendingWrite() = writeQueued.get()

    private fun scheduleWrite() {
        if (writeQueued.compareAndSet(false, true)) later(Runnable { flushNow() })
    }

    private fun load(f: TlFile, gen: Int) {
        val loaded = try {
            decodeOrNull(f.read().toString(Charsets.UTF_8)) ?: run {
                log("TIMELINE unreadable, starting empty")
                f.delete()
                emptyList()
            }
        } catch (e: FileNotFoundException) {
            emptyList()
        } catch (e: Exception) {
            // The history may be fine on disk: leave it alone this session.
            writeLocked = true
            log("TIMELINE read failed: ${e.javaClass.simpleName}")
            emptyList()
        }
        val pending = synchronized(lock) {
            // Cleared or turned off since this load was queued: the file's entries are gone.
            if (gen != generation || !enabled) return
            val cur = _entries.value
            _entries.value = prune(cur + loaded, now())
            cur.isNotEmpty()
        }
        // Entries added before the load finished are not on disk yet.
        if (pending) scheduleWrite()
    }

    private fun write() {
        val f = file ?: return
        if (writeLocked) return
        val snapshot = synchronized(lock) { if (enabled) _entries.value else return }
        try {
            f.write(encode(snapshot).toByteArray(Charsets.UTF_8))
        } catch (e: Exception) {
            // The previous file stays; the entries stay in memory for the next write.
            // Only the error's type is logged, never anything that could hold content.
            log("TIMELINE write failed: ${e.javaClass.simpleName}")
        }
    }
}

/**
 * The process-wide store: `filesDir/timeline.json` through AtomicFile, on
 * one daemon I/O thread. [init] loads the file once (from the service or the
 * UI, whichever starts first); [add] updates [entries] at once and writes the
 * file shortly after, so bursts coalesce into one write.
 */
object Timeline {
    private const val FILE = "timeline.json"
    private const val WRITE_DELAY_MS = 300L

    private val io = Executors.newSingleThreadScheduledExecutor { r -> Thread(r, "timeline-io").apply { isDaemon = true } }
    private val store = TimelineStore(
        exec = { io.execute(it) },
        later = { io.schedule(it, WRITE_DELAY_MS, TimeUnit.MILLISECONDS) },
        log = { Log.e(TAG, it) },
    )
    @Volatile private var app: Context? = null

    /** Newest first. */
    val entries: StateFlow<List<TlEntry>> get() = store.entries

    private val _historyOn = kotlinx.coroutines.flow.MutableStateFlow(true)
    /** "Keep history" as stored in Prefs; screens observe this rather than reading Prefs once. */
    val historyOn: StateFlow<Boolean> = _historyOn.asStateFlow()

    fun init(ctx: Context) {
        synchronized(this) {
            if (app != null) return
            app = ctx.applicationContext
        }
        val on = Prefs(ctx).keepHistory
        _historyOn.value = on
        store.start(AtomicTlFile(AtomicFile(File(ctx.applicationContext.filesDir, FILE))), on)
    }

    fun add(e: TlEntry) = store.add(e)

    /** Removes one entry; [restore] undoes it. */
    fun remove(e: TlEntry) = store.remove(e)
    fun restore(e: TlEntry) = store.restore(e)

    /** Deletes the history (memory and file). */
    fun clear() = store.clear()

    /** "Keep history": off deletes the history and stops recording; on starts again. */
    fun setEnabled(on: Boolean) {
        app?.let { Prefs(it).keepHistory = on }
        _historyOn.value = on
        store.setEnabled(on)
    }

    /**
     * Writes a waiting entry now (the app is quitting, e.g. right after a
     * transfer), waiting at most [maxMs]. Never blocks longer.
     */
    fun flush(maxMs: Long = 500) {
        if (!store.hasPendingWrite()) return
        try {
            io.submit { store.flushNow() }.get(maxMs, TimeUnit.MILLISECONDS)
        } catch (e: Exception) {
            Log.e(TAG, "TIMELINE flush on exit did not finish: ${e.javaClass.simpleName}")
        }
    }
}
