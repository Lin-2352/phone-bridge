package dev.phonebridge.app

/**
 * Degrees the camera image must be rotated clockwise to be upright, the
 * standard Camera2 formula. The front lens is mirrored, so the device
 * rotation counts the other way round (`sensor + device`); the back lens
 * uses `sensor - device`.
 */
fun rotationDegrees(sensorOrientation: Int, deviceRotation: Int, front: Boolean): Int =
    if (front) (sensorOrientation + deviceRotation) % 360
    else (sensorOrientation - deviceRotation + 360) % 360

/**
 * The device rotation the formula wants (the display-rotation convention:
 * 0, 90 = turned counter-clockwise, ...) from `OrientationEventListener`'s
 * degrees (the physical angle, clockwise; `ORIENTATION_UNKNOWN` = -1), or
 * null when the sensor cannot say (flat on a table).
 */
fun deviceRotationFromSensor(degrees: Int): Int? {
    if (degrees < 0) return null
    val snapped = ((degrees + 45) / 90 * 90) % 360
    return (360 - snapped) % 360
}

/**
 * Another app (the phone's own Camera app) may still be releasing the
 * sensor when we open it: try again a few times, 0, 0.4, 0.8 and 1.2 s
 * after the start, then give up (null).
 */
fun retryDelayMs(attempt: Int): Long? = when (attempt) {
    0 -> 0L
    1 -> 400L
    2 -> 800L
    3 -> 1200L
    else -> null
}

/**
 * The auto-exposure frame-rate range to lock: exactly 30 when the camera
 * offers [30, 30] (so indoor light does not drop the stream to 15 fps),
 * otherwise the highest fixed range, otherwise the one with the highest
 * upper bound.
 */
fun pickFpsRange(ranges: List<Pair<Int, Int>>): Pair<Int, Int>? {
    if (ranges.isEmpty()) return null
    if (30 to 30 in ranges) return 30 to 30
    val fixed = ranges.filter { it.first == it.second }
    return fixed.maxByOrNull { it.second } ?: ranges.maxByOrNull { it.second }
}

/** A 16:9 output size: 1920x1080 if offered, else 1280x720, else null. */
fun pickSize(sizes: List<Pair<Int, Int>>): Pair<Int, Int>? =
    listOf(1920 to 1080, 1280 to 720).firstOrNull { it in sizes }

/**
 * Flags of the invisible window kept in front while the camera streams.
 * Touch-through, but still focusable: with no focusable window the system has
 * no focus window and realme's watchdog kills the app after about 13 s.
 */
fun holdWindowFlags(): Int = android.view.WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE
