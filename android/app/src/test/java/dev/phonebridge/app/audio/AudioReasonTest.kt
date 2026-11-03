package dev.phonebridge.app.audio

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class AudioReasonTest {
    @Test
    fun codesMatchTheWire() {
        assertEquals(1, AudioReason.DECLINED.code)
        assertEquals(2, AudioReason.UNAVAILABLE.code)
        assertEquals(3, AudioReason.BUSY.code)
        assertEquals(4, AudioReason.BAD_PARAMS.code)
        assertEquals(5, AudioReason.NO_PERMISSION.code)
        assertEquals(6, AudioReason.APP_BLOCKS_CAPTURE.code)
    }

    @Test
    fun unknownCodeReadsAsDeclined() {
        assertEquals(AudioReason.DECLINED, AudioReason.fromCode(0))
        assertEquals(AudioReason.DECLINED, AudioReason.fromCode(99))
        assertEquals(AudioReason.APP_BLOCKS_CAPTURE, AudioReason.fromCode(6))
    }

    @Test
    fun playbackNeedsAndroid10() {
        assertEquals(AudioReason.UNAVAILABLE, planCapture(AudioSource.PLAYBACK, 28, true, true, 48000, 2))
    }

    @Test
    fun playbackNeedsTheProjectionToken() {
        assertEquals(AudioReason.UNAVAILABLE, planCapture(AudioSource.PLAYBACK, 34, true, false, 48000, 2))
    }

    @Test
    fun bothSourcesNeedRecordAudio() {
        assertEquals(AudioReason.NO_PERMISSION, planCapture(AudioSource.PLAYBACK, 34, false, true, 48000, 2))
        assertEquals(AudioReason.NO_PERMISSION, planCapture(AudioSource.MICROPHONE, 34, false, false, 48000, 2))
    }

    @Test
    fun microphoneNeedsNoProjectionAndWorksOnOldAndroid() {
        assertNull(planCapture(AudioSource.MICROPHONE, 26, true, false, 48000, 2))
    }

    @Test
    fun badFormatIsRejected() {
        assertEquals(AudioReason.BAD_PARAMS, planCapture(AudioSource.MICROPHONE, 34, true, false, 8000, 2))
        assertEquals(AudioReason.BAD_PARAMS, planCapture(AudioSource.MICROPHONE, 34, true, false, 48000, 3))
        assertNull(planCapture(AudioSource.MICROPHONE, 34, true, false, 44100, 1))
    }

    @Test
    fun exceptionsMapToReasons() {
        assertEquals(AudioReason.NO_PERMISSION, reasonForFailure(SecurityException("x")))
        assertEquals(AudioReason.UNAVAILABLE, reasonForFailure(IllegalStateException("x")))
        assertEquals(AudioReason.UNAVAILABLE, reasonForFailure(UnsupportedOperationException()))
    }
}
