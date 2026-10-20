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
