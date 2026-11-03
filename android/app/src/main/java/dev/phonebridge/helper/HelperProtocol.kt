package dev.phonebridge.helper

import java.nio.ByteBuffer
import java.security.MessageDigest
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

/**
 * What Relay and its audio helper say over a loopback connection. The helper runs as the shell user,
 * so it can capture what a normal app can't, and any app on the phone can reach a loopback port. So
 * this is a security boundary: before anything else each side proves it knows a secret that only
 * Relay and the helper it started hold, over nonces that are new for every connection.
 *
 * The helper sends its nonce; Relay answers with its nonce and proof; the helper checks that and
 * only then sends its own proof. A stranger learns nothing, and Relay takes no audio from one.
 */
object HelperProtocol {
    /** Fixed so Relay can find it. An app holding it first only stops the helper starting: it can't pass the proof. */
    const val PORT = 47123
    const val SECRET_ENV = "RELAY_HELPER_SECRET"
    const val SECRET_BYTES = 32
    const val NONCE_BYTES = 16
    const val PROOF_BYTES = 32
    const val ROLE_RELAY: Byte = 'R'.code.toByte()
    const val ROLE_HELPER: Byte = 'H'.code.toByte()
    const val VERSION: Byte = 1
    const val CMD_PING: Byte = 'P'.code.toByte()
    const val CMD_CAPTURE: Byte = 'C'.code.toByte()
    const val REPLY_OK: Byte = 'O'.code.toByte()
    const val REPLY_BUSY: Byte = 'B'.code.toByte()
    const val REPLY_FAILED: Byte = 'F'.code.toByte()
    const val PARAMS_BYTES = 5

    data class CaptureParams(val sampleRate: Int, val channels: Int)

    /** Both sides use the nonces in this order; [role] keeps one side's proof from passing as the other's. */
    fun proof(secret: ByteArray, role: Byte, relayNonce: ByteArray, helperNonce: ByteArray): ByteArray {
        val mac = Mac.getInstance("HmacSHA256")
        mac.init(SecretKeySpec(secret, "HmacSHA256"))
        mac.update(role)
        mac.update(relayNonce)
        return mac.doFinal(helperNonce)
    }

    fun proofOk(secret: ByteArray, role: Byte, relayNonce: ByteArray, helperNonce: ByteArray, got: ByteArray): Boolean =
        MessageDigest.isEqual(proof(secret, role, relayNonce, helperNonce), got)

    fun encodeParams(p: CaptureParams): ByteArray =
        ByteBuffer.allocate(PARAMS_BYTES).putInt(p.sampleRate).put(p.channels.toByte()).array()

    /** Null for anything but the formats capture supports. */
    fun decodeParams(b: ByteArray): CaptureParams? {
        if (b.size != PARAMS_BYTES) return null
        val buf = ByteBuffer.wrap(b)
        val rate = buf.getInt()
        val channels = buf.get().toInt()
        if ((rate != 44100 && rate != 48000) || (channels != 1 && channels != 2)) return null
        return CaptureParams(rate, channels)
    }

    /** One 10 ms frame of 16-bit PCM. */
    fun frameBytes(sampleRate: Int, channels: Int): Int = sampleRate / 100 * 2 * channels

    fun secretToHex(secret: ByteArray): String = secret.joinToString("") { "%02x".format(it) }

    /** Null unless it is exactly a secret's worth of hex. */
    fun secretFromHex(text: String?): ByteArray? {
        if (text == null || text.length != SECRET_BYTES * 2) return null
        val out = ByteArray(SECRET_BYTES)
        for (i in out.indices) {
            val hi = Character.digit(text[i * 2], 16)
            val lo = Character.digit(text[i * 2 + 1], 16)
            if (hi < 0 || lo < 0) return null
            out[i] = (hi * 16 + lo).toByte()
        }
        return out
    }
}
