package dev.phonebridge.app

import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ClipFingerprintTest {
    private fun macWith(key: Byte): (ByteArray) -> ByteArray = { data ->
        Mac.getInstance("HmacSHA256").apply { init(SecretKeySpec(ByteArray(32) { key }, "HmacSHA256")) }.doFinal(data)
    }

    @Test
    fun same_text_same_key_matches() {
        val mac = macWith(1)
        assertTrue(clipUnchanged("123456", clipFingerprint("123456", mac), mac))
    }

    @Test
    fun other_text_does_not_match() {
        val mac = macWith(1)
        assertFalse(clipUnchanged("123457", clipFingerprint("123456", mac), mac))
    }

    @Test
    fun other_key_does_not_match() {
        assertNotEquals(clipFingerprint("123456", macWith(1)), clipFingerprint("123456", macWith(2)))
    }

    @Test
    fun fingerprint_is_64_hex_chars() {
        val f = clipFingerprint("hello", macWith(1))
        assertEquals(64, f.length)
        assertTrue(f.all { it in "0123456789abcdef" })
    }

    @Test
    fun legacy_value_never_matches_and_is_dropped() {
        // The old pref held String.hashCode() as an int; it must not count as "unchanged".
        val mac = macWith(1)
        assertFalse(clipUnchanged("123456", "123456".hashCode().toString(), mac))
        assertFalse(clipUnchanged("123456", null, mac))
    }
}

class FallbackMacTest {
    private val fallback: (ByteArray) -> ByteArray = { byteArrayOf(7) }

    @Test
    fun a_key_that_breaks_in_use_falls_back_once() {
        var switched = 0
        val m = FallbackMac({ throw java.security.InvalidKeyException("invalid key blob") }, { fallback }) { switched++ }
        assertEquals(7, m.mac(byteArrayOf(1))[0].toInt())
        assertEquals(7, m.mac(byteArrayOf(2))[0].toInt())
        assertEquals(1, switched)
    }

    @Test
    fun a_working_key_is_used() {
        val m = FallbackMac({ byteArrayOf(3) }, { fallback }) { error("no fallback expected") }
        assertEquals(3, m.mac(byteArrayOf(1))[0].toInt())
    }
}
