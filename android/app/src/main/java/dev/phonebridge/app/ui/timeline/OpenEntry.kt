package dev.phonebridge.app.ui.timeline

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import dev.phonebridge.app.TlEntry
import dev.phonebridge.app.TlKind
import dev.phonebridge.app.fileState
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** Only a saved file with a content:// uri opens; a file:// uri would throw FileUriExposedException. */
fun isOpenable(isFile: Boolean, uri: String?) = isFile && uri?.startsWith("content://") == true

fun isOpenable(e: TlEntry) = isOpenable(e.kind == TlKind.File, e.uri)

/**
 * Opens a received file in a viewer, the same way on every screen. The check touches storage, so it runs off
 * the main thread. Returns the message to show when nothing opened, or null when a viewer was started.
 */
suspend fun openSavedFile(ctx: Context, uri: String): String? {
    if (!withContext(Dispatchers.IO) { fileState(ctx, uri) }) return "File moved or deleted"
    val u = Uri.parse(uri)
    val type = withContext(Dispatchers.IO) { runCatching { ctx.contentResolver.getType(u) }.getOrNull() } ?: "*/*"
    val view = Intent(Intent.ACTION_VIEW).setDataAndType(u, type).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    return try {
        ctx.startActivity(view)
        null
    } catch (x: ActivityNotFoundException) {
        "No app can open this file"
    } catch (x: SecurityException) {
        "Can't open this file any more"
    }
}
