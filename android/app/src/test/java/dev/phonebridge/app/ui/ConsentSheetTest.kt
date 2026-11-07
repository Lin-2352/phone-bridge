package dev.phonebridge.app.ui

import dev.phonebridge.app.ui.components.DismissReason
import dev.phonebridge.app.ui.components.sheetDismissAllowed
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ConsentSheetTest {
    @Test
    fun consent_sheet_cannot_be_dismissed_without_a_choice() {
        assertTrue(sheetDismissAllowed(DismissReason.Choice))
        assertFalse(sheetDismissAllowed(DismissReason.OutsideTap))
        assertFalse(sheetDismissAllowed(DismissReason.Drag))
        assertFalse(sheetDismissAllowed(DismissReason.Back))
        assertEquals(4, DismissReason.values().size)
    }
}
