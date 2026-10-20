package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class CameraMathTest {
    @Test
    fun backCameraRotationSubtractsTheDeviceRotation() {
        assertEquals(90, rotationDegrees(90, 0, front = false))
        assertEquals(0, rotationDegrees(90, 90, front = false))
        assertEquals(270, rotationDegrees(90, 180, front = false))
    }

    @Test
    fun frontCameraRotationAddsTheDeviceRotation() {
        assertEquals(270, rotationDegrees(270, 0, front = true))
        assertEquals(0, rotationDegrees(270, 90, front = true))
        assertEquals(90, rotationDegrees(270, 180, front = true))
    }

    @Test
    fun theFpsRangeIsLockedToThirtyWhenOffered() {
        assertEquals(30 to 30, pickFpsRange(listOf(15 to 30, 30 to 30, 24 to 24)))
    }

    @Test
    fun withoutThirtyTheHighestFixedRangeWins() {
        assertEquals(24 to 24, pickFpsRange(listOf(15 to 30, 24 to 24)))
        assertEquals(15 to 30, pickFpsRange(listOf(15 to 30, 7 to 20)))
        assertNull(pickFpsRange(emptyList()))
    }

    @Test
    fun theSizeIsFullHdThenHdAndNothingElse() {
        assertEquals(1920 to 1080, pickSize(listOf(640 to 480, 1280 to 720, 1920 to 1080)))
        assertEquals(1280 to 720, pickSize(listOf(640 to 480, 1280 to 720)))
        assertNull(pickSize(listOf(640 to 480, 4000 to 3000)))
    }
}
