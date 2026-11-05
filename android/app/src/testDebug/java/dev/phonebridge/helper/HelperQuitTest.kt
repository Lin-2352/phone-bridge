package dev.phonebridge.helper

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Test

class HelperQuitTest {
    @Test
    fun quit_is_its_own_command() {
        assertEquals('Q'.code.toByte(), HelperProtocol.CMD_QUIT)
        assertNotEquals(HelperProtocol.CMD_PING, HelperProtocol.CMD_QUIT)
        assertNotEquals(HelperProtocol.CMD_CAPTURE, HelperProtocol.CMD_QUIT)
    }
}
