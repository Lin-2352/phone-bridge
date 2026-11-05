package dev.phonebridge.app.audio

import android.view.KeyEvent

/**
 * The opt-in volume key filter, run by the Relay control accessibility service. With YouTube or Instagram in front,
 * Android hands the volume keys to that app's own player, and only a key filter can give them to the PC's level.
 * Banking apps refuse to open while an accessibility service is on, so this is the user's choice, and the service
 * switches itself off when the share ends.
 */
fun holdsVolumeKeys(setting: Boolean, silencedShare: Boolean): Boolean = setting && silencedShare

/** What the filter does with a key: null lets it through, 0 swallows it, +1 or -1 also moves the PC's level. */
fun volumeKeyStep(holding: Boolean, keyCode: Int, action: Int): Int? {
    if (!holding) return null
    val direction = when (keyCode) {
        KeyEvent.KEYCODE_VOLUME_UP -> 1
        KeyEvent.KEYCODE_VOLUME_DOWN -> -1
        else -> return null
    }
    // A held key repeats as more ACTION_DOWN events, so each repeat is a step, as with the phone's own volume.
    return if (action == KeyEvent.ACTION_DOWN) direction else 0
}
