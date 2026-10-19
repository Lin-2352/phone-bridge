package dev.phonebridge.app

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Bundle
import android.util.Log
import android.widget.FrameLayout
import android.widget.TextView
import androidx.activity.ComponentActivity
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.QRCodeReader
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.launch

private const val TAG = "PhoneBridge"

/**
 * Scans the pairing QR shown by the PC. Frames are decoded on-device with
 * ZXing (no network, no Play Services); only codes starting with `pb1:` are
 * accepted. The camera is used only while this screen is open.
 */
class ScanActivity : ComponentActivity() {
    private val executor = Executors.newSingleThreadExecutor()
    private val done = AtomicBoolean(false)
    private lateinit var preview: PreviewView

    private val askCamera = registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) startCamera() else {
            Log.i(TAG, "SCAN camera permission denied")
            finish()
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        preview = PreviewView(this)
        setContentView(FrameLayout(this).apply {
            addView(preview)
            addView(TextView(context).apply {
                text = "Point at the QR code shown by Phone Bridge on your PC"
                setPadding(32, 64, 32, 32)
                setTextColor(0xFFFFFFFF.toInt())
                setShadowLayer(4f, 0f, 0f, 0xFF000000.toInt())
            })
        })
        if (checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) startCamera()
        else askCamera.launch(Manifest.permission.CAMERA)
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
            if (text != null && text.startsWith("pb1:") && done.compareAndSet(false, true)) {
                Log.i(TAG, "SCAN got pairing code")
                // In-process hand-off: MainActivity shows the code check from Bridge state.
                Bridge.scope.launch { Bridge.join(text) }
                runOnUiThread {
                    startActivity(
                        Intent(this, MainActivity::class.java)
                            .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
                    )
                    finish()
                }
            }
        }
    }
}
