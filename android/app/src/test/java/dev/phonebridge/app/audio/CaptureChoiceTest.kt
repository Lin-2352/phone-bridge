package dev.phonebridge.app.audio

import org.junit.Assert.assertEquals
import org.junit.Test

class CaptureChoiceTest {
    private fun choose(
        source: AudioSource = AudioSource.PLAYBACK,
        silence: Boolean = true,
        helper: Boolean = true,
    ) = chooseCapture(source, silence, helper)

    @Test
    fun a_running_helper_serves_silent_phone_audio() {
        assertEquals(CaptureKind.HELPER, choose())
    }

    @Test
    fun without_the_helper_it_falls_back_to_androids_capture() {
        assertEquals(CaptureKind.ANDROID, choose(helper = false))
    }

    /** The helper's route takes the sound off the speaker, so it can't serve "plays on both". */
    @Test
    fun with_silence_off_the_phone_keeps_playing_through_androids_capture() {
        assertEquals(CaptureKind.ANDROID, choose(silence = false))
    }

    /** The microphone asks every time, and never turns into anything else. */
    @Test
    fun the_microphone_never_uses_the_helper() {
        assertEquals(CaptureKind.ANDROID, choose(source = AudioSource.MICROPHONE))
    }

    @Test
    fun a_start_with_nothing_to_ask_needs_the_helper_and_the_skip_setting() {
        assertEquals(true, startsWithoutWindow(CaptureKind.HELPER, skipConfirm = true))
        assertEquals(false, startsWithoutWindow(CaptureKind.HELPER, skipConfirm = false))
        assertEquals(false, startsWithoutWindow(CaptureKind.ANDROID, skipConfirm = true))
    }
}
