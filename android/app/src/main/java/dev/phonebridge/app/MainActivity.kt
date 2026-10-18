package dev.phonebridge.app

import android.Manifest
import android.app.Activity
import android.content.ClipboardManager
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.util.Log
import android.view.View
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import uniffi.pb_ffi.PbEvent
import uniffi.pb_ffi.PbNode

private const val TAG = "PhoneBridge"

/**
 * UI over the [PbService]-owned node: pair with a PC, send clipboard text.
 * Received clips arrive as notifications from the service even when this
 * screen is closed.
 *
 * Test hooks (debug use over adb): intent extras `pair_code`, `confirm` ("yes"/"no"),
 * `send_text`. They drive exactly the same code paths as the buttons.
 */
class MainActivity : Activity() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private lateinit var status: TextView
    private lateinit var peersView: TextView
    private lateinit var sasPanel: LinearLayout
    private lateinit var sasText: TextView
    private lateinit var log: LinearLayout
    private val listener: (PbEvent) -> Unit = { e -> runOnUiThread { onEvent(e) } }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        buildUi()
        if (Build.VERSION.SDK_INT >= 33 &&
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
        }
        startForegroundService(Intent(this, PbService::class.java))
        Bridge.listeners.add(listener)
        scope.launch {
            val n = awaitNode() ?: run { say("Service did not start (see logcat)"); return@launch }
            status.text = "This phone: ${n.fingerprint()}\n${n.addresses().joinToString()}"
            say("NODE_ID=${n.peerId()}")
            refreshPeers()
            handleIntent(intent)
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleIntent(intent)
    }

    override fun onDestroy() {
        Bridge.listeners.remove(listener)
        scope.cancel()
        super.onDestroy()
    }

    private suspend fun awaitNode(): PbNode? {
        repeat(100) {
            Bridge.node?.let { return it }
            delay(100)
        }
        return null
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
                if (code.isNullOrBlank()) say("Copy the code shown by the PC first") else join(code)
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
        val stopBtn = Button(this).apply {
            text = "Stop Phone Bridge"
            setOnClickListener {
                startService(Intent(context, PbService::class.java).setAction(PbService.ACTION_STOP))
                finish()
            }
        }
        log = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        listOf(status, peersView, pairBtn, sasPanel, sendBtn, stopBtn, log).forEach(root::addView)
        setContentView(ScrollView(this).apply { addView(root) })
    }

    private fun clipboard() = getSystemService(ClipboardManager::class.java)

    private fun say(line: String) {
        Log.i(TAG, line)
        log.addView(TextView(this).apply { text = line }, 0)
    }

    private fun onEvent(e: PbEvent) = when (e) {
        is PbEvent.Connected -> say("Connected: ${e.peerName}")
        is PbEvent.Disconnected -> say("Disconnected: ${e.peerName} (${e.reason})")
        is PbEvent.Clipboard -> say(if (e.sensitive) "From ${e.peerName}: (hidden)" else "From ${e.peerName}: ${e.text}")
    }

    private fun refreshPeers() {
        val peers = Bridge.node?.peers().orEmpty()
        peersView.text = if (peers.isEmpty()) "Not paired yet" else
            "Paired: " + peers.joinToString { "${it.name} (${it.fingerprint.take(9)})" }
    }

    private fun join(code: String) = scope.launch {
        val n = Bridge.node ?: return@launch
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
        val n = Bridge.node ?: return@launch
        sasPanel.visibility = View.GONE
        try {
            say("PAIRED with ${n.confirmPairing(accept)}")
        } catch (e: Throwable) {
            say("Pairing not completed: ${e.message}")
        }
        refreshPeers()
    }

    private fun send(text: String) = scope.launch {
        val n = Bridge.node ?: return@launch
        val peer = n.peers().firstOrNull() ?: run { say("Pair with a PC first"); return@launch }
        try {
            n.sendClipboard(peer.id, text, false)
            say("SENT ${text.length} chars to ${peer.name}")
        } catch (e: Throwable) {
            say("Send failed: ${e.message}")
        }
    }
}
