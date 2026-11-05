package dev.phonebridge.app

import android.content.Intent
import android.media.projection.MediaProjectionConfig
import android.media.projection.MediaProjectionManager
import android.os.Build

/** What Android's capture prompt may offer. */
enum class CaptureConfig { NONE, DEFAULT_DISPLAY }

/**
 * From Android 14 the prompt also offers "a single app". Relay maps the PC's clicks onto the whole
 * screen and treats a black picture as a secure window, so it always asks for the whole display.
 */
fun captureConfig(sdk: Int): CaptureConfig = if (sdk >= 34) CaptureConfig.DEFAULT_DISPLAY else CaptureConfig.NONE

/** The capture prompt's intent, limited to the whole display where Android allows that. */
fun captureIntent(mpm: MediaProjectionManager): Intent =
    if (captureConfig(Build.VERSION.SDK_INT) == CaptureConfig.DEFAULT_DISPLAY && Build.VERSION.SDK_INT >= 34) {
        mpm.createScreenCaptureIntent(MediaProjectionConfig.createConfigForDefaultDisplay())
    } else {
        mpm.createScreenCaptureIntent()
    }
