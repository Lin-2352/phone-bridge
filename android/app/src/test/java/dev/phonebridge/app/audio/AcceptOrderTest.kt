package dev.phonebridge.app.audio

import org.junit.Assert.assertEquals
import org.junit.Test

/** L3: the PC hears "yes" once the phone is silent, or after the wait runs out; nothing waits on the main thread. */
class AcceptOrderTest {
    @Test fun accept_waits_for_silence_off_the_main_thread() {
        assertEquals(AcceptStep.WAIT, acceptOrder(silenced = false, waitedMs = 0, limitMs = 1_500))
        assertEquals(AcceptStep.ANSWER, acceptOrder(silenced = true, waitedMs = 10, limitMs = 1_500))
        assertEquals(AcceptStep.ANSWER, acceptOrder(silenced = false, waitedMs = 1_500, limitMs = 1_500))
    }

    @Test fun a_busy_speaker_still_gets_the_answer_just_inside_the_limit() {
        assertEquals(AcceptStep.WAIT, acceptOrder(silenced = false, waitedMs = 1_499, limitMs = 1_500))
    }
}
