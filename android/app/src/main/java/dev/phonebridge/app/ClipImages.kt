package dev.phonebridge.app

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.util.Log
import androidx.core.content.FileProvider
import java.io.ByteArrayOutputStream
import java.io.File

private const val TAG = "PhoneBridge"

/** Reading a picture that is not a PNG is capped here; a PNG of it must still fit [MAX_IMAGE_BYTES]. */
private const val MAX_SOURCE_BYTES = 40L * 1024 * 1024

/** Pictures between the PC's clipboard and this phone: receiving one onto the clipboard, and sending one to the PC. */
object ClipImages {
    private fun clipsDir(ctx: Context) = File(ctx.cacheDir, "clips").apply { mkdirs() }

    /**
     * Put a PNG the PC sent on the clipboard. The file moves out of the node's
     * incoming folder into a small cache served by a FileProvider (not the gallery),
     * and old ones are pruned.
     */
    fun applyReceived(ctx: Context, path: String) {
        val src = File(path)
        val dest = File(clipsDir(ctx), src.name)
        if (!src.renameTo(dest)) {
            src.copyTo(dest, overwrite = true)
            src.delete()
        }
        dest.setLastModified(System.currentTimeMillis())
        val uri = FileProvider.getUriForFile(ctx, "${ctx.packageName}.clips", dest)
        val clip = ClipData.newUri(ctx.contentResolver, APP_NAME, uri)
        android.os.Handler(ctx.mainLooper).post {
            ctx.getSystemService(ClipboardManager::class.java).setPrimaryClip(clip)
        }
        prune(ctx)
    }

    private fun prune(ctx: Context) {
        val files = clipsDir(ctx).listFiles().orEmpty().map { CachedClip(it.name, it.lastModified()) }
        for (name in clipsToDelete(files, System.currentTimeMillis())) File(clipsDir(ctx), name).delete()
    }

    /** The runtime permission that lets us see the gallery's pictures (new screenshots). */
    val imagePermission: String
        get() = if (android.os.Build.VERSION.SDK_INT >= 33) android.Manifest.permission.READ_MEDIA_IMAGES else android.Manifest.permission.READ_EXTERNAL_STORAGE

    fun hasImageAccess(ctx: Context): Boolean =
        ctx.checkSelfPermission(imagePermission) == android.content.pm.PackageManager.PERMISSION_GRANTED

    /** Send the picture at [uri] (any format Android can decode). Returns null when sent, else a sentence for the user. */
    suspend fun send(ctx: Context, uri: Uri): String? {
        val bytes = try {
            ctx.contentResolver.openInputStream(uri)?.use { it.readCapped(MAX_SOURCE_BYTES) }
        } catch (t: Throwable) {
            Log.e(TAG, "CLIPIMG could not read $uri: $t")
            null
        } ?: return imageSendNotice(null)
        return sendBytes(ctx, bytes)
    }

    /**
     * Send a picture's bytes to the paired PC to put on its clipboard. Anything
     * that is not a PNG is converted first (the PC reads PNG). Returns null when
     * sent, else a sentence for the user.
     */
    suspend fun sendBytes(ctx: Context, bytes: ByteArray): String? {
        val node = Bridge.awaitNode() ?: return "$APP_NAME is not running"
        val peer = node.peers().firstOrNull() ?: return "Pair $APP_NAME with your PC first"
        val png = toPng(bytes) ?: return imageSendNotice("not a picture")
        if (png.size > MAX_IMAGE_BYTES) return imageSendNotice("too large")
        val tmp = File(ctx.cacheDir, "send-${System.nanoTime()}.png")
        return try {
            tmp.writeBytes(png)
            val fd = ParcelFileDescriptor.open(tmp, ParcelFileDescriptor.MODE_READ_ONLY).detachFd()
            node.sendClipboardImageFd(peer.id, fd)
            Log.i(TAG, "CLIPIMG sent ${png.size} bytes to ${peer.name}")
            Bridge.addRecent(RecentItem("Picture", "Sent to ${peer.name}", RecentItem.Kind.Clipboard))
            null
        } catch (t: Throwable) {
            Log.e(TAG, "CLIPIMG send failed: $t")
            imageSendNotice(t.message)
        } finally {
            tmp.delete()
        }
    }

    private fun toPng(bytes: ByteArray): ByteArray? {
        if (isPngBytes(bytes)) return bytes
        val bmp = try {
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size)
        } catch (t: Throwable) {
            null
        } ?: return null
        return try {
            ByteArrayOutputStream().also { bmp.compress(Bitmap.CompressFormat.PNG, 100, it) }.toByteArray()
        } finally {
            bmp.recycle()
        }
    }
}

/** Everything in the stream, or null if it is longer than [limit] bytes. */
fun java.io.InputStream.readCapped(limit: Long): ByteArray? {
    val out = ByteArrayOutputStream()
    val buf = ByteArray(64 * 1024)
    var total = 0L
    while (true) {
        val n = read(buf)
        if (n < 0) break
        total += n
        if (total > limit) return null
        out.write(buf, 0, n)
    }
    return out.toByteArray()
}
