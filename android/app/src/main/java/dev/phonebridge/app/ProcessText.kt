package dev.phonebridge.app

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import android.widget.Toast
import kotlinx.coroutines.launch

/** The most text one "Send to PC" sends (the same cap as the clipboard sync). */
const val PROCESS_TEXT_MAX = 60 * 1024

/** The text to send from a selection: trimmed of nothing but the cap; null when there is nothing to send. */
fun selectedTextToSend(selected: CharSequence?): String? = selected?.toString()?.take(PROCESS_TEXT_MAX)?.takeIf { it.isNotBlank() }

/**
 * "Send to PC" in the text-selection menu of any app (ACTION_PROCESS_TEXT). Android lets only the focused app read
 * the clipboard, so a copy in another app cannot reach the PC by itself; this sends the selected text directly.
 */
class ProcessTextActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val text = selectedTextToSend(intent.getCharSequenceExtra(Intent.EXTRA_PROCESS_TEXT))
        val app = applicationContext
        when {
            text == null -> Toast.makeText(this, "Nothing selected to send", Toast.LENGTH_SHORT).show()
            !Prefs(this).enabled || !Bridge.shownOn() ->
                Toast.makeText(this, "$APP_NAME is off. Turn it on from its Quick Settings tile", Toast.LENGTH_LONG).show()
            else -> {
                Toast.makeText(this, "Sending to your PC", Toast.LENGTH_SHORT).show()
                Bridge.scope.launch {
                    val problem = sendClipText(app, text, sensitive = false)
                    if (problem != null) notifyInfo(app, "Couldn't send the text", problem)
                }
            }
        }
        finish()
    }
}
