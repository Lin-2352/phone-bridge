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
    fun theSensorAngleBecomesTheDisplayConvention() {
        assertEquals(0, deviceRotationFromSensor(0))
        assertEquals(0, deviceRotationFromSensor(20))
        assertEquals(270, deviceRotationFromSensor(90))
        assertEquals(180, deviceRotationFromSensor(180))
        assertEquals(90, deviceRotationFromSensor(270))
        assertEquals(0, deviceRotationFromSensor(350))
        assertNull(deviceRotationFromSensor(-1))
    }

    @Test
    fun aBusyCameraIsRetriedAFewTimesThenGivenUp() {
        assertEquals(0L, retryDelayMs(0))
        assertEquals(400L, retryDelayMs(1))
        assertEquals(800L, retryDelayMs(2))
        assertEquals(1200L, retryDelayMs(3))
        assertNull(retryDelayMs(4))
        assertNull(retryDelayMs(-1))
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

    @Test
    fun steps_down_under_1200() {
        assertEquals(true, cameraStepDown(1199, steppedDown = false))
    }

    @Test
    fun stays_stepped_down_between_1200_and_1600() {
        assertEquals(true, cameraStepDown(1400, steppedDown = true))
        assertEquals(true, cameraStepDown(1600, steppedDown = true))
    }

    @Test
    fun returns_above_1600() {
        assertEquals(false, cameraStepDown(1601, steppedDown = true))
    }

    @Test
    fun stays_full_between_1200_and_1600_when_not_stepped() {
        assertEquals(false, cameraStepDown(1200, steppedDown = false))
        assertEquals(false, cameraStepDown(1400, steppedDown = false))
    }

    @Test
    fun steppedDownTheSizeIsCappedAtHd() {
        assertEquals(1280 to 720, pickSize(listOf(1280 to 720, 1920 to 1080), steppedDown = true))
        assertNull(pickSize(listOf(1920 to 1080), steppedDown = true))
    }
}
