package dev.phonebridge.app

import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.os.PersistableBundle
import android.util.Log
import android.view.View
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import uniffi.pb_ffi.PbEvent
import uniffi.pb_ffi.PbNode
import uniffi.pb_ffi.startNode

private const val TAG = "PhoneBridge"
private const val PORT = 47100

/** Hands the Application context to Rust (iroh needs it for system network config). */
object NativeInit {
    init {
        System.loadLibrary("pb_ffi")
    }

    @JvmStatic
    external fun initContext(context: Context)
}

/**
 * Phase 1 screen: pair with a PC, then send and receive clipboard text.
 *
 * Test hooks (debug use over adb): intent extras `pair_code`, `confirm` ("yes"/"no"),
 * `send_text`. They drive exactly the same code paths as the buttons.
 */
class MainActivity : Activity() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private var node: PbNode? = null
    private lateinit var status: TextView
    private lateinit var peersView: TextView
    private lateinit var sasPanel: LinearLayout
    private lateinit var sasText: TextView
    private lateinit var log: LinearLayout

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        buildUi()
        scope.launch {
            try {
                NativeInit.initContext(applicationContext)
                val n = startNode(filesDir.absolutePath, Build.MODEL ?: "Android", PORT.toUShort())
                node = n
                status.text = "This phone: ${n.fingerprint()}\n${n.addresses().joinToString()}"
                say("NODE_ID=${n.peerId()}")
                refreshPeers()
                handleIntent(intent)
                pollEvents(n)
            } catch (e: Throwable) {
                say("FAILED to start: $e")
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleIntent(intent)
    }

    override fun onDestroy() {
        val n = node
        scope.launch { n?.shutdown() }
        super.onDestroy()
    }

    private fun handleIntent(intent: Intent?) {
        intent ?: return
        intent.getStringExtra("pair_code")?.let { join(it) }
        intent.getStringExtra("confirm")?.let { confirm(it == "yes") }
        intent.getStringExtra("send_text")?.let { send(it) }
    }

    private fun buildUi() {
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(32, 48, 32, 32)
        }
        status = TextView(this).apply { textSize = 13f }
        peersView = TextView(this).apply { textSize = 15f; setPadding(0, 24, 0, 24) }
        val pairBtn = Button(this).apply {
            text = "Pair: paste code from PC"
            setOnClickListener {
                val code = clipboard().primaryClip?.getItemAt(0)?.coerceToText(context)?.toString()
                if (code.isNullOrBlank()) say("Copy the code shown by `pb pair-host` first") else join(code)
            }
        }
        sasText = TextView(this).apply { textSize = 28f }
        sasPanel = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            visibility = View.GONE
            addView(TextView(context).apply { text = "Does the PC show this code?" })
            addView(sasText)
            addView(LinearLayout(context).apply {
                addView(Button(context).apply { text = "Yes, pair"; setOnClickListener { confirm(true) } })
                addView(Button(context).apply { text = "No"; setOnClickListener { confirm(false) } })
            })
        }
        val sendBtn = Button(this).apply {
            text = "Send my clipboard to PC"
            setOnClickListener {
                val text = clipboard().primaryClip?.getItemAt(0)?.coerceToText(context)?.toString()
                if (text.isNullOrEmpty()) say("Clipboard is empty") else send(text)
            }
        }
        log = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        listOf(status, peersView, pairBtn, sasPanel, sendBtn, log).forEach(root::addView)
        setContentView(ScrollView(this).apply { addView(root) })
    }

    private fun clipboard() = getSystemService(ClipboardManager::class.java)

    private fun say(line: String) {
        Log.i(TAG, line)
        log.addView(TextView(this).apply { text = line }, 0)
    }

    private fun refreshPeers() {
        val peers = node?.peers().orEmpty()
        peersView.text = if (peers.isEmpty()) "Not paired yet" else
            "Paired: " + peers.joinToString { "${it.name} (${it.fingerprint.take(9)})" }
    }

    private fun join(code: String) = scope.launch {
        val n = node ?: return@launch
        try {
            val sas = n.joinPairing(code.trim())
            sasText.text = sas
            sasPanel.visibility = View.VISIBLE
            say("SAS=$sas")
        } catch (e: Throwable) {
            say("Pairing failed: ${e.message}")
        }
    }

    private fun confirm(accept: Boolean) = scope.launch {
        val n = node ?: return@launch
        sasPanel.visibility = View.GONE
        try {
            val name = n.confirmPairing(accept)
            say("PAIRED with $name")
        } catch (e: Throwable) {
            say("Pairing not completed: ${e.message}")
        }
        refreshPeers()
    }

    private fun send(text: String) = scope.launch {
        val n = node ?: return@launch
        val peer = n.peers().firstOrNull() ?: run { say("Pair with a PC first"); return@launch }
        try {
            n.sendClipboard(peer.id, text, false)
            say("SENT ${text.length} chars to ${peer.name}")
        } catch (e: Throwable) {
            say("Send failed: ${e.message}")
        }
    }

    private suspend fun pollEvents(n: PbNode) {
        while (true) {
            for (e in n.events()) when (e) {
                is PbEvent.Connected -> say("Connected: ${e.peerName}")
                is PbEvent.Disconnected -> say("Disconnected: ${e.peerName} (${e.reason})")
                is PbEvent.Clipboard -> showClip(e)
            }
            delay(500)
        }
    }

    private fun showClip(e: PbEvent.Clipboard) {
        Log.i(TAG, if (e.sensitive) "CLIP from ${e.peerName} (sensitive)" else "CLIP from ${e.peerName}: ${e.text}")
        val row = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        row.addView(TextView(this).apply {
            text = if (e.sensitive) "From ${e.peerName}: (hidden)" else "From ${e.peerName}: ${e.text}"
            layoutParams = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f)
        })
        row.addView(Button(this).apply {
            text = "Copy"
            setOnClickListener {
                val clip = ClipData.newPlainText("Phone Bridge", e.text)
                if (e.sensitive) {
                    clip.description.extras = PersistableBundle().apply {
                        putBoolean("android.content.extra.IS_SENSITIVE", true)
                    }
                }
                clipboard().setPrimaryClip(clip)
            }
        })
        log.addView(row, 0)
    }
}
