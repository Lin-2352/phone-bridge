package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ClipboardImagePeerTest {
    @Test
    fun a_requested_picture_targets_the_pc_that_asked_not_the_saved_default() {
        assertEquals("requesting-pc", clipboardImagePeerId("requesting-pc", "default-pc"))
    }

    @Test
    fun a_manual_picture_still_uses_the_default_pc() {
        assertEquals("default-pc", clipboardImagePeerId(null, "default-pc"))
        assertNull(clipboardImagePeerId(null, null))
    }

    @Test
    fun an_explicit_request_without_a_default_still_targets_its_requester() {
        assertEquals("requesting-pc", clipboardImagePeerId("requesting-pc", null))
    }
}
