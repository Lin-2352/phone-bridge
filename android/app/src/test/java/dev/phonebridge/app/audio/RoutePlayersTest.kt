package dev.phonebridge.app.audio

import android.media.AudioAttributes
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Merged 2026-11-06 with the v2 branch's alarm lift (B14): the silencing effect steps aside while an alarm or
 * ringtone plays, so a covered video on the same output would be heard too. An alarm counts as a player, which
 * sends a covered sound back under the mute; the alarm itself plays on its own stream, which Relay never mutes.
 */
class RoutePlayersTest {
    @Test
    fun media_games_and_unknown_sound_are_players() {
        assertTrue(countsAsPlayer(AudioAttributes.USAGE_MEDIA))
        assertTrue(countsAsPlayer(AudioAttributes.USAGE_GAME))
        assertTrue(countsAsPlayer(AudioAttributes.USAGE_UNKNOWN))
    }

    @Test
    fun an_alarm_or_ringtone_counts_so_a_covered_sound_goes_back_under_the_mute() {
        assertTrue(countsAsPlayer(AudioAttributes.USAGE_ALARM))
        assertTrue(countsAsPlayer(AudioAttributes.USAGE_NOTIFICATION_RINGTONE))
    }

    @Test
    fun calls_and_notifications_are_not_players() {
        assertFalse(countsAsPlayer(AudioAttributes.USAGE_VOICE_COMMUNICATION))
        assertFalse(countsAsPlayer(AudioAttributes.USAGE_NOTIFICATION))
    }
}
