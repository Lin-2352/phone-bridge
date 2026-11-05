package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class EncoderChoiceTest {
    private val hardware = setOf("hw")
    private fun pick(all: List<String>, allowSoftware: Boolean) = usableEncoders(all, { it in hardware }, allowSoftware)

    @Test
    fun releaseBuildsNeverUseASoftwareEncoder() {
        assertTrue(pick(listOf("sw"), allowSoftware = false).isEmpty())
        assertEquals(listOf("hw"), pick(listOf("sw", "hw"), allowSoftware = false))
    }

    @Test
    fun aDebugBuildPrefersHardwareWhenThePhoneHasIt() {
        assertEquals(listOf("hw"), pick(listOf("sw", "hw"), allowSoftware = true))
    }

    @Test
    fun aDebugBuildFallsBackToSoftwareOnlyWhenThereIsNoHardware() {
        assertEquals(listOf("sw"), pick(listOf("sw"), allowSoftware = true))
    }
}
