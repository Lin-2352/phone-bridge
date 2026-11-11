package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ClipboardPermissionTest {
    @Test fun only_enabling_without_permission_needs_the_overlay_settings() {
        assertTrue(needsClipboardOverlay(true, false))
        assertEquals(false, needsClipboardOverlay(true, true))
        assertEquals(false, needsClipboardOverlay(false, false))
        assertEquals(false, needsClipboardOverlay(false, true))
    }
    @Test fun the_pairing_notice_explains_default_off_and_both_devices() {
        assertTrue(NEW_PC_CLIPBOARD_NOTICE.contains("off"))
        assertTrue(NEW_PC_CLIPBOARD_NOTICE.contains("phone"))
        assertTrue(NEW_PC_CLIPBOARD_NOTICE.contains("PC"))
    }
}
