package dev.phonebridge.app.audio

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class AudioConsentTest {
    @Test
    fun a_stop_from_the_pc_is_never_rewritten_by_the_capture_ending_with_it() {
        // The PC pressed Stop and the capture then died with the link: still the PC's stop, and silent.
        assertEquals(EndCause.PEER_STOPPED, resolveEnd(EndCause.PEER_STOPPED, EndCause.SOURCE_ENDED))
        assertEquals(EndCause.PEER_STOPPED, resolveEnd(EndCause.PEER_STOPPED, null))
        assertEquals(null, endNotice(resolveEnd(EndCause.PEER_STOPPED, EndCause.SOURCE_ENDED), AudioDirection.PHONE_TO_PC))
        // Our own capture ended (app blocks capture): the stop we then sent is not the user's, the reason stays.
        assertEquals(EndCause.APP_BLOCKS_CAPTURE, resolveEnd(EndCause.LOCAL_STOP, EndCause.APP_BLOCKS_CAPTURE))
        assertEquals(EndCause.LOCAL_STOP, resolveEnd(EndCause.LOCAL_STOP, null))
        assertEquals(EndCause.LINK_LOST, resolveEnd(EndCause.LINK_LOST, null))
    }

    private val relay = ConsentStep.RELAY_DIALOG
    private val record = ConsentStep.RECORD_AUDIO
    private val projection = ConsentStep.PROJECTION

    @Test fun playbackAsksRelayThenPermissionThenTheSystemDialog() {
        assertEquals(listOf(relay, record, projection), consentSteps(AudioSource.PLAYBACK, false, false))
    }

    @Test fun playbackSkipsOnlyRelaysOwnDialogNeverTheProjectionDialog() {
        assertEquals(listOf(projection), consentSteps(AudioSource.PLAYBACK, true, true))
        assertEquals(listOf(record, projection), consentSteps(AudioSource.PLAYBACK, true, false))
    }

    @Test fun playbackWithPermissionHeldDoesNotAskForItAgain() {
        assertEquals(listOf(relay, projection), consentSteps(AudioSource.PLAYBACK, false, true))
    }

    @Test fun microphoneIsAskedEveryTimeWhateverTheSkipSetting() {
        assertEquals(listOf(relay, record), consentSteps(AudioSource.MICROPHONE, false, false))
        assertEquals(listOf(relay, record), consentSteps(AudioSource.MICROPHONE, true, false))
        assertEquals(listOf(relay), consentSteps(AudioSource.MICROPHONE, true, true))
        assertEquals(listOf(relay), consentSteps(AudioSource.MICROPHONE, false, true))
    }

    @Test fun microphoneNeverNeedsTheProjectionDialog() {
        for (skip in listOf(true, false)) for (has in listOf(true, false)) {
            assertFalse(projection in consentSteps(AudioSource.MICROPHONE, skip, has))
            assertTrue(relay in consentSteps(AudioSource.MICROPHONE, skip, has))
        }
    }

    @Test fun oneSessionPerDirection() {
        val s = AudioSessions()
        assertTrue(s.tryBegin(AudioDirection.PHONE_TO_PC))
        assertFalse(s.tryBegin(AudioDirection.PHONE_TO_PC))
        assertTrue("the other direction is independent", s.tryBegin(AudioDirection.PC_TO_PHONE))
        assertTrue(s.isActive(AudioDirection.PHONE_TO_PC) && s.isActive(AudioDirection.PC_TO_PHONE))
        s.end(AudioDirection.PHONE_TO_PC)
        assertFalse(s.isActive(AudioDirection.PHONE_TO_PC))
        assertTrue(s.isActive(AudioDirection.PC_TO_PHONE))
        assertTrue(s.tryBegin(AudioDirection.PHONE_TO_PC))
    }

    @Test fun endingAnIdleDirectionIsHarmless() {
        val s = AudioSessions()
        s.end(AudioDirection.PC_TO_PHONE)
        assertFalse(s.isActive(AudioDirection.PC_TO_PHONE))
    }

    @Test fun directionsMapToTheWireNumbers() {
        assertEquals(AudioDirection.PHONE_TO_PC, AudioDirection.fromWire(0))
        assertEquals(AudioDirection.PC_TO_PHONE, AudioDirection.fromWire(1))
        assertNull(AudioDirection.fromWire(2))
    }

    @Test fun stoppingOnPurposeOrFromThePcIsSilent() {
        for (d in AudioDirection.entries) {
            assertNull(endNotice(EndCause.LOCAL_STOP, d))
            assertNull(endNotice(EndCause.PEER_STOPPED, d))
        }
    }

    @Test fun failuresTellTheUserWhatHappenedAndWhatToDo() {
        for (c in listOf(EndCause.LINK_LOST, EndCause.VIOLATION, EndCause.WRITE_FAILED, EndCause.APP_BLOCKS_CAPTURE, EndCause.SOURCE_ENDED)) {
            for (d in AudioDirection.entries) {
                val t = endNotice(c, d)
                assertTrue("$c $d", t != null && t.length > 20)
            }
        }
        assertTrue(endNotice(EndCause.APP_BLOCKS_CAPTURE, AudioDirection.PHONE_TO_PC)!!.contains("another app"))
        assertTrue(endNotice(EndCause.LINK_LOST, AudioDirection.PHONE_TO_PC)!!.contains("ask again"))
    }

    @Test fun everyRefusalReasonHasADistinctUserMessage() {
        val texts = AudioReason.entries.map { refusalNotice(it, AudioSource.PLAYBACK) }
        assertEquals(texts.size, texts.toSet().size)
        assertTrue(refusalNotice(AudioReason.NO_PERMISSION, AudioSource.MICROPHONE).contains("microphone", ignoreCase = true))
        assertTrue(refusalNotice(AudioReason.NO_PERMISSION, AudioSource.MICROPHONE).contains("Settings"))
        assertTrue(refusalNotice(AudioReason.APP_BLOCKS_CAPTURE, AudioSource.PLAYBACK).contains("another app"))
        assertTrue(refusalNotice(AudioReason.BUSY, AudioSource.PLAYBACK).contains("Stop"))
        assertTrue(refusalNotice(AudioReason.UNAVAILABLE, AudioSource.PLAYBACK).contains("Android 10"))
    }

    @Test fun microphoneNotificationSaysTheMicrophoneIsShared() {
        assertEquals("Microphone is being shared", sessionNoticeText(AudioDirection.PHONE_TO_PC, AudioSource.MICROPHONE))
        assertFalse(sessionNoticeText(AudioDirection.PHONE_TO_PC, AudioSource.PLAYBACK).contains("Microphone"))
        assertFalse(sessionNoticeText(AudioDirection.PC_TO_PHONE, AudioSource.PLAYBACK).contains("Microphone"))
    }
}
