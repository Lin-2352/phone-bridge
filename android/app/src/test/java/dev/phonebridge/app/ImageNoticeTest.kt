package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ImageNoticeTest {
    @Test
    fun a_declined_send_tells_the_user_what_to_turn_on() {
        assertTrue(imageSendNotice("declined").contains("Share images"))
        assertTrue(imageSendNotice("The other device declined").contains("Share images"))
    }

    @Test
    fun a_too_large_picture_says_so() {
        assertTrue(imageSendNotice("too large").contains("10 MB"))
        assertTrue(imageSendNotice("the picture is over 10 MB").contains("10 MB"))
    }

    @Test
    fun an_unreachable_pc_is_not_blamed_on_settings() {
        val text = imageSendNotice("timed out")
        assertTrue(text.contains("reach"))
        assertEquals(false, text.contains("Share images"))
    }

    @Test
    fun a_png_is_told_from_other_pictures_by_its_signature() {
        val png = byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3)
        assertTrue(isPngBytes(png))
        assertEquals(false, isPngBytes(byteArrayOf(0xFF.toByte(), 0xD8.toByte(), 0xFF.toByte())))
        assertEquals(false, isPngBytes(ByteArray(0)))
        assertEquals(false, isPngBytes(png.copyOf(5)))
    }

    @Test
    fun no_message_still_gives_a_sentence() {
        assertTrue(imageSendNotice(null).isNotBlank())
    }
}
