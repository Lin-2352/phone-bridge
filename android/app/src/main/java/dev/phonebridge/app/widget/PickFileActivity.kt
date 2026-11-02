package dev.phonebridge.app.widget

import android.content.ClipData
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.result.contract.ActivityResultContracts
import dev.phonebridge.app.ShareActivity

/**
 * The widget's "Send file": the system picker, then the share flow; nothing on screen of its own.
 * Not noHistory: a noHistory activity never gets the picker's result.
 */
class PickFileActivity : ComponentActivity() {
    private val pick = registerForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        if (uris.isNotEmpty()) sendPickedFiles(this, uris)
        finish()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Recreated (rotation) while the picker is open: its result still comes here.
        if (savedInstanceState == null) runCatching { pick.launch(arrayOf("*/*")) }.onFailure { finish() }
    }
}

/** Into the share flow, as if shared from another app; the read grant goes along with the URIs. */
fun sendPickedFiles(ctx: Context, uris: List<Uri>) {
    val send = Intent(ctx, ShareActivity::class.java)
        .setAction(Intent.ACTION_SEND_MULTIPLE)
        .setType("*/*")
        .putParcelableArrayListExtra(Intent.EXTRA_STREAM, ArrayList(uris))
        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    send.clipData = ClipData.newRawUri(null, uris[0]).apply { uris.drop(1).forEach { addItem(ClipData.Item(it)) } }
    runCatching { ctx.startActivity(send) }
}
