package dev.phonebridge.app

import dev.phonebridge.app.ui.RelayTheme
import dev.phonebridge.app.ui.pairing.DenialAction
import dev.phonebridge.app.ui.pairing.NotRelayTracker
import dev.phonebridge.app.ui.pairing.PairingFlow
import dev.phonebridge.app.ui.pairing.denialAction
import dev.phonebridge.app.ui.pairing.looksLikePairingCode
import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Bundle
import android.os.SystemClock
import android.provider.Settings
import android.util.Log
import android.widget.FrameLayout
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.ComposeView
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.QRCodeReader
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

private const val TAG = "PhoneBridge"

/**
 * Scans the pairing QR shown by the PC. Frames are decoded on-device with
 * ZXing (no network, no Play Services); only codes starting with `pb1:` are
 * accepted. The camera is used only while this screen is open. Without camera
 * access the page explains why, offers Android's settings page when the system
 * will not ask again, and takes a pasted code instead.
 */
class ScanActivity : ComponentActivity() {
    private val executor = Executors.newSingleThreadExecutor()
    private val done = AtomicBoolean(false)
    private lateinit var preview: PreviewView
    private var mode by mutableStateOf(ScanMode.Explain)
    private var denial by mutableStateOf(DenialAction.TryAgain)
    private var cameraStarted = false
    private var notRelay by mutableStateOf(false)
    private val tracker = NotRelayTracker()

    private val askCamera = registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) {
            beginScanning()
        } else {
            Log.i(TAG, "SCAN camera permission denied")
            // After "don't ask again" Android shows no prompt, so the only way back is the app's settings page.
            denial = denialAction(shouldShowRequestPermissionRationale(Manifest.permission.CAMERA))
            mode = ScanMode.Denied
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        preview = PreviewView(this)
        setContentView(FrameLayout(this).apply {
            addView(preview)
            addView(ComposeView(context).apply {
                setContent { RelayTheme { ScanUi() } }
            })
        })
        if (hasCamera()) beginScanning()
    }

    override fun onResume() {
        super.onResume()
        // Back from Android's settings page with the camera allowed: carry on without another tap.
        if (mode != ScanMode.Scanning && hasCamera()) beginScanning()
    }

    private fun hasCamera() = checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED

    private fun beginScanning() {
        mode = ScanMode.Scanning
        if (!cameraStarted) {
            cameraStarted = true
            startCamera()
        }
    }

    private fun openAppSettings() {
        runCatching {
            startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:$packageName")))
        }.onFailure {
            Toast.makeText(this, "Open Android Settings, then Apps, then $APP_NAME, then Permissions", Toast.LENGTH_LONG).show()
        }
    }

    /** Pairing starts in-process; MainActivity shows its progress, the code check and any error. */
    private fun handOff(code: String) {
        if (!done.compareAndSet(false, true)) return
        Log.i(TAG, "SCAN got pairing code")
        PairingFlow.start(code)
        runOnUiThread {
            startActivity(
                Intent(this, MainActivity::class.java)
                    .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
            )
            finish()
        }
    }

    @Composable
    private fun ScanUi() {
        when (mode) {
            ScanMode.Scanning -> {
                Column(Modifier.statusBarsPadding().padding(Spacing.lg), verticalArrangement = Arrangement.spacedBy(Spacing.sm)) {
                    ScrimCard("Point at the QR code shown by $APP_NAME on your PC")
                    if (notRelay) {
                        Box(Modifier.semantics { liveRegion = LiveRegionMode.Polite }) {
                            ScrimCard("That is not a $APP_NAME code. Use the QR code under Pair new device on your PC.")
                        }
                    }
                }
            }
            ScanMode.Explain -> CameraPage(
                title = "Scan the code on your PC",
                body = "$APP_NAME uses the camera only to read the pairing QR code on your PC. Nothing is recorded or sent anywhere.",
                primary = "Allow camera",
                onPrimary = { askCamera.launch(Manifest.permission.CAMERA) },
            )
            ScanMode.Denied -> CameraPage(
                title = "Camera access is off",
                body = if (denial == DenialAction.TryAgain) {
                    "Without the camera this phone cannot read the QR code. You can allow it, or paste the pairing code instead."
                } else {
                    "Android will not ask again. Open $APP_NAME's settings, choose Permissions, then Camera, and allow it. Or paste the pairing code instead."
                },
                primary = if (denial == DenialAction.TryAgain) "Try again" else "Open settings",
                onPrimary = { if (denial == DenialAction.TryAgain) askCamera.launch(Manifest.permission.CAMERA) else openAppSettings() },
            )
        }
    }

    /** A scrim card instead of bare shadowed text: readable over any camera image, in light and dark. */
    @Composable
    private fun ScrimCard(text: String) {
        Surface(
            shape = RoundedCornerShape(Radius.card),
            color = MaterialTheme.colorScheme.surface.copy(alpha = 0.85f),
        ) {
            Text(
                text,
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurface,
                modifier = Modifier.padding(horizontal = Spacing.lg, vertical = Spacing.md),
            )
        }
    }

    /** Full page shown before the camera is asked for and after it was refused; a pasted code is the way around the camera. */
    @Composable
    private fun CameraPage(title: String, body: String, primary: String, onPrimary: () -> Unit) {
        var code by remember { mutableStateOf("") }
        val clipboard = LocalClipboardManager.current
        Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.surface) {
            Column(
                Modifier.statusBarsPadding().navigationBarsPadding().imePadding().verticalScroll(rememberScrollState()).padding(Spacing.xl),
                verticalArrangement = Arrangement.spacedBy(Spacing.lg),
            ) {
                Text(title, style = MaterialTheme.typography.headlineSmall)
                Text(body, style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Button(onClick = onPrimary, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) { Text(primary) }
                Text("Or paste the pairing code", style = MaterialTheme.typography.titleSmall)
                OutlinedTextField(
                    value = code,
                    onValueChange = { code = it },
                    label = { Text("Pairing code (starts with pb1:)") },
                    maxLines = 3,
                    modifier = Modifier.fillMaxWidth(),
                )
                Row(horizontalArrangement = Arrangement.spacedBy(Spacing.sm)) {
                    FilledTonalButton(
                        onClick = { clipboard.getText()?.text?.let { code = it } },
                        modifier = Modifier.heightIn(min = 48.dp),
                    ) { Text("Paste") }
                    Button(
                        onClick = { handOff(code.trim()) },
                        enabled = looksLikePairingCode(code),
                        modifier = Modifier.heightIn(min = 48.dp),
                    ) { Text("Pair") }
                }
                if (code.isNotBlank() && !looksLikePairingCode(code)) {
                    Text("That does not look like a $APP_NAME pairing code.", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
                }
                TextButton(onClick = { finish() }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Cancel") }
            }
        }
    }

    override fun onDestroy() {
        executor.shutdown()
        super.onDestroy()
    }

    private fun startCamera() {
        val future = ProcessCameraProvider.getInstance(this)
        future.addListener({
            val provider = future.get()
            val p = Preview.Builder().build().also { it.surfaceProvider = preview.surfaceProvider }
            val analysis = ImageAnalysis.Builder()
                .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                .build()
            val reader = QRCodeReader()
            val hints = mapOf(DecodeHintType.TRY_HARDER to true)
            analysis.setAnalyzer(executor) { img -> analyze(img, reader, hints) }
            provider.unbindAll()
            provider.bindToLifecycle(this, CameraSelector.DEFAULT_BACK_CAMERA, p, analysis)
            Log.i(TAG, "SCAN camera started")
        }, mainExecutor)
    }

    private fun analyze(img: ImageProxy, reader: QRCodeReader, hints: Map<DecodeHintType, Any>) {
        img.use {
            if (done.get()) return
            val plane = it.planes[0]
            val buf = plane.buffer
            val data = ByteArray(buf.remaining()).also { b -> buf.get(b) }
            val src = PlanarYUVLuminanceSource(data, plane.rowStride, it.height, 0, 0, it.width, it.height, false)
            val text = try {
                reader.decode(BinaryBitmap(HybridBinarizer(src)), hints).text
            } catch (_: Exception) {
                null
            } finally {
                reader.reset()
            }
            if (text != null && text.startsWith("pb1:")) {
                handOff(text)
            } else {
                // Some other QR: after a second in view, say so instead of ignoring it.
                notRelay = tracker.onDecode(text, SystemClock.elapsedRealtime())
            }
        }
    }
}

private enum class ScanMode { Explain, Denied, Scanning }
