package dev.phonebridge.helper

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class HelperProtocolTest {
    private val secret = ByteArray(32) { it.toByte() }
    private val other = ByteArray(32) { (it + 1).toByte() }
    private val relayNonce = ByteArray(16) { 1 }
    private val helperNonce = ByteArray(16) { 2 }

    private fun proof(key: ByteArray, role: Byte, own: ByteArray = relayNonce, peer: ByteArray = helperNonce) =
        HelperProtocol.proof(key, role, own, peer)

    @Test
    fun a_peer_with_the_secret_is_accepted() {
        val p = proof(secret, HelperProtocol.ROLE_RELAY)
        assertTrue(HelperProtocol.proofOk(secret, HelperProtocol.ROLE_RELAY, relayNonce, helperNonce, p))
    }

    /** Any app on the phone can reach a loopback port: only the secret tells Relay apart from them. */
    @Test
    fun a_peer_without_the_secret_is_refused() {
        val p = proof(other, HelperProtocol.ROLE_RELAY)
        assertFalse(HelperProtocol.proofOk(secret, HelperProtocol.ROLE_RELAY, relayNonce, helperNonce, p))
    }

    /** A recording of an earlier connection is useless: the other side's nonce is new each time. */
    @Test
    fun a_replayed_proof_is_refused() {
        val old = proof(secret, HelperProtocol.ROLE_RELAY)
        val fresh = ByteArray(16) { 9 }
        assertFalse(HelperProtocol.proofOk(secret, HelperProtocol.ROLE_RELAY, relayNonce, fresh, old))
    }

    /** Sending the helper's own proof back must not pass as Relay's. */
    @Test
    fun a_proof_reflected_back_is_refused() {
        val helperSays = HelperProtocol.proof(secret, HelperProtocol.ROLE_HELPER, relayNonce, helperNonce)
        assertFalse(HelperProtocol.proofOk(secret, HelperProtocol.ROLE_RELAY, relayNonce, helperNonce, helperSays))
    }

    @Test
    fun the_two_nonces_are_not_interchangeable() {
        assertNotEquals(
            proof(secret, HelperProtocol.ROLE_RELAY).toList(),
            HelperProtocol.proof(secret, HelperProtocol.ROLE_RELAY, helperNonce, relayNonce).toList(),
        )
    }

    @Test
    fun a_short_proof_is_refused() {
        assertFalse(HelperProtocol.proofOk(secret, HelperProtocol.ROLE_RELAY, relayNonce, helperNonce, ByteArray(4)))
    }

    @Test
    fun capture_parameters_round_trip() {
        val p = HelperProtocol.CaptureParams(48000, 2)
        assertEquals(p, HelperProtocol.decodeParams(HelperProtocol.encodeParams(p)))
    }

    @Test
    fun odd_capture_parameters_are_refused() {
        for (p in listOf(HelperProtocol.CaptureParams(8000, 2), HelperProtocol.CaptureParams(48000, 3), HelperProtocol.CaptureParams(-1, 0))) {
            assertNull(HelperProtocol.decodeParams(HelperProtocol.encodeParams(p)))
        }
    }

    @Test
    fun short_capture_parameters_are_refused() {
        assertNull(HelperProtocol.decodeParams(ByteArray(2)))
    }

    @Test
    fun a_frame_is_ten_milliseconds_of_pcm() {
        assertEquals(1920, HelperProtocol.frameBytes(48000, 2))
        assertEquals(882, HelperProtocol.frameBytes(44100, 1))
    }

    @Test
    fun a_secret_is_read_back_from_its_text_form() {
        assertEquals(secret.toList(), HelperProtocol.secretFromHex(HelperProtocol.secretToHex(secret))?.toList())
    }

    @Test
    fun a_bad_secret_text_is_refused() {
        assertNull(HelperProtocol.secretFromHex("nothex"))
        assertNull(HelperProtocol.secretFromHex("ab"))
        assertNull(HelperProtocol.secretFromHex(null))
    }
}
