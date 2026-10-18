package dev.phonebridge.smoke

import android.app.Activity
import android.content.Context
import android.os.Bundle
import android.util.Log
import android.widget.ScrollView
import android.widget.TextView
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import uniffi.pb_smoke.Spike
import uniffi.pb_smoke.residentKb
import uniffi.pb_smoke.startSpike

private const val TAG = "PbSmoke"
private const val PORT = 47100

/** Hands the Application context to Rust (iroh needs it for system network config). */
object NativeInit {
    init {
        System.loadLibrary("pb_smoke")
    }

    @JvmStatic
    external fun initContext(context: Context)
}

class MainActivity : Activity() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private var spike: Spike? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val text = TextView(this).apply {
            textSize = 12f
            setPadding(24, 24, 24, 24)
        }
        setContentView(ScrollView(this).apply { addView(text) })

        fun say(line: String) {
            Log.i(TAG, line)
            text.append(line + "\n")
        }

        scope.launch {
            try {
                NativeInit.initContext(applicationContext)
                say("JNI context installed")
                val s = startSpike(PORT.toUShort(), false)
                spike = s
                say("ENDPOINT_ID=${s.endpointId()}")
                say("ADDRS=${s.addresses()}")
                var tick = 0
                while (true) {
                    s.events().forEach { say("event: $it") }
                    if (tick++ % 10 == 0) say("rss_kb=${residentKb()}")
                    delay(1000)
                }
            } catch (e: Throwable) {
                say("FAILED: $e")
            }
        }
    }

    override fun onDestroy() {
        scope.launch { spike?.shutdown() }
        super.onDestroy()
    }
}
