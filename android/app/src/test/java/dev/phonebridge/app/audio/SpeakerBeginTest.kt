package dev.phonebridge.app.audio

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SpeakerBeginTest {
    @Test
    fun speaker_does_not_begin_once_the_service_is_gone() {
        // A queued begin that runs after the service's final restore would leave the phone silenced.
        assertFalse(speakerMayBegin(serviceUp = false))
    }

    @Test
    fun speaker_begins_while_the_service_runs() {
        assertTrue(speakerMayBegin(serviceUp = true))
    }
}
