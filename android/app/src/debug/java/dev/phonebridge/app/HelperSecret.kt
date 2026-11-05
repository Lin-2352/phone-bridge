package dev.phonebridge.app

import dev.phonebridge.helper.HelperProtocol

/** This build carries the audio helper (debug builds only). */
const val HELPER_BUILT = true

/** Shared with the helper Relay starts, and with nothing else. Made once, on first need. */
fun Prefs.audioHelperSecret(): ByteArray {
    HelperProtocol.secretFromHex(audioHelperSecretHex)?.let { return it }
    val made = ByteArray(HelperProtocol.SECRET_BYTES).also { java.security.SecureRandom().nextBytes(it) }
    audioHelperSecretHex = HelperProtocol.secretToHex(made)
    return made
}
