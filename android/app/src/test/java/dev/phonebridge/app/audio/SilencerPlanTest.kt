package dev.phonebridge.app.audio

import org.junit.Assert.assertEquals
import org.junit.Test

class SilencerPlanTest {
    @Test
    fun the_phone_gets_its_own_volume_back() {
        assertEquals(-15, stepsBack(current = 16, saved = 1))
        assertEquals(3, stepsBack(current = 1, saved = 4))
        assertEquals(0, stepsBack(current = 5, saved = 5))
    }

    /** Nothing saved means no share was running: the user's volume is not Relay's to move. */
    @Test
    fun with_nothing_saved_the_volume_stays_where_it_is() {
        assertEquals(0, stepsBack(current = 16, saved = -1))
    }

    /** A level left at 0 must not make the next share silent on the PC for no visible reason. */
    @Test
    fun a_muted_share_starts_at_a_saved_pc_level_unless_it_was_silence() {
        assertEquals(5, muteShareStartStep(5))
        assertEquals(PC_LEVEL_STEPS, muteShareStartStep(0))
    }

    /** The volume bar set the level before this version: nothing may jump in the user's headphones. */
    @Test
    fun first_share_after_update_keeps_the_old_loudness() {
        assertEquals(8, shareStartStep(saved = PC_LEVEL_STEPS, oldIndex = 8, max = 16))
    }

    @Test
    fun fresh_install_starts_at_full() {
        assertEquals(PC_LEVEL_STEPS, shareStartStep(saved = PC_LEVEL_STEPS, oldIndex = -1, max = 16))
    }

    @Test
    fun later_shares_start_at_the_saved_level() {
        assertEquals(5, shareStartStep(saved = 5, oldIndex = -1, max = 16))
    }

    /** Turning the PC down to nothing is for now: the next share comes back at a level that was heard. */
    @Test
    fun going_to_silence_keeps_the_level_for_next_time() {
        assertEquals(9, rememberedStep(saved = 9, step = 0))
        assertEquals(3, rememberedStep(saved = 9, step = 3))
    }

    /** Waiting while no packet can be ready would only hold up the capture thread. */
    @Test
    fun the_encoder_is_waited_for_only_when_a_packet_is_due() {
        assertEquals(0L, encoderWaitUs(pendingBytes = 1920, bytesPerPacket = 3840))
        assertEquals(ENCODER_WAIT_US, encoderWaitUs(pendingBytes = 3840, bytesPerPacket = 3840))
        assertEquals(ENCODER_WAIT_US, encoderWaitUs(pendingBytes = 5760, bytesPerPacket = 3840))
    }

    @Test
    fun earphones_are_silenced_too_unless_the_user_says_otherwise() {
        assertEquals(true, silenceWanted(silencePhone = true, silenceEarphones = true, earphonesOn = true))
        assertEquals(false, silenceWanted(silencePhone = true, silenceEarphones = false, earphonesOn = true))
    }

    @Test
    fun without_earphones_the_speaker_is_silenced_whichever_way_that_switch_is() {
        assertEquals(true, silenceWanted(silencePhone = true, silenceEarphones = false, earphonesOn = false))
        assertEquals(true, silenceWanted(silencePhone = true, silenceEarphones = true, earphonesOn = false))
    }

    @Test
    fun with_silencing_off_nothing_is_silenced() {
        assertEquals(false, silenceWanted(silencePhone = false, silenceEarphones = true, earphonesOn = true))
        assertEquals(false, silenceWanted(silencePhone = false, silenceEarphones = true, earphonesOn = false))
    }

    @Test
    fun wired_and_wireless_earphones_count_and_the_phones_own_outputs_do_not() {
        for (t in listOf(
            android.media.AudioDeviceInfo.TYPE_WIRED_HEADSET,
            android.media.AudioDeviceInfo.TYPE_WIRED_HEADPHONES,
            android.media.AudioDeviceInfo.TYPE_USB_HEADSET,
            android.media.AudioDeviceInfo.TYPE_BLUETOOTH_A2DP,
            android.media.AudioDeviceInfo.TYPE_BLE_HEADSET,
            android.media.AudioDeviceInfo.TYPE_HEARING_AID,
        )) {
            assertEquals("type $t", true, isEarphones(t))
        }
        for (t in listOf(
            android.media.AudioDeviceInfo.TYPE_BUILTIN_SPEAKER,
            android.media.AudioDeviceInfo.TYPE_BUILTIN_EARPIECE,
            android.media.AudioDeviceInfo.TYPE_TELEPHONY,
            android.media.AudioDeviceInfo.TYPE_REMOTE_SUBMIX,
        )) {
            assertEquals("type $t", false, isEarphones(t))
        }
    }

    /** Android keeps a volume per output: the speaker's level walked onto earphones could be painfully loud. */
    @Test
    fun a_saved_volume_only_goes_back_on_the_output_it_came_from() {
        assertEquals(true, volumeGoesBack(savedOnEarphones = false, earphonesOn = false))
        assertEquals(true, volumeGoesBack(savedOnEarphones = true, earphonesOn = true))
        assertEquals(false, volumeGoesBack(savedOnEarphones = false, earphonesOn = true))
        assertEquals(false, volumeGoesBack(savedOnEarphones = true, earphonesOn = false))
    }

    /** A microphone share must say the microphone: it is what the user would want to know. */
    @Test
    fun home_says_what_is_being_shared() {
        val mic = audioShareCard(AudioSource.MICROPHONE, "Desk")
        assertEquals("Sharing your microphone with Desk", mic.first)
        org.junit.Assert.assertTrue(mic.second, mic.second.contains("microphone"))
        val phone = audioShareCard(AudioSource.PLAYBACK, "Desk")
        assertEquals("Sharing this phone's audio with Desk", phone.first)
    }
}
