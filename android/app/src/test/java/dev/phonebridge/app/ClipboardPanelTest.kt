package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Test

class ClipboardPanelTest {
    @Test fun a_single_enabled_pc_can_receive_without_checking_the_all_pcs_switch() {
        assertEquals(ClipboardPanel(false, true), clipboardPanel(true, true, false))
    }
    @Test fun off_or_no_enabled_pc_cannot_send() {
        assertEquals(ClipboardPanel(true, false), clipboardPanel(false, true, true))
        assertEquals(ClipboardPanel(false, false), clipboardPanel(true, false, false))
    }
}
