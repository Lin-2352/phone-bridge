package dev.phonebridge.app.audio

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SilenceLiftTest {
    // AudioAttributes.USAGE_MEDIA = 1, USAGE_ALARM = 4, USAGE_NOTIFICATION_RINGTONE = 6, USAGE_GAME = 14
    @Test
    fun alarm_lifts() = assertTrue(mustLiftSilencer(listOf(1, 4)))

    @Test
    fun ringtone_lifts() = assertTrue(mustLiftSilencer(listOf(6)))

    @Test
    fun media_does_not_lift() = assertFalse(mustLiftSilencer(listOf(1, 14)))

    @Test
    fun empty_does_not_lift() = assertFalse(mustLiftSilencer(emptyList()))
}
