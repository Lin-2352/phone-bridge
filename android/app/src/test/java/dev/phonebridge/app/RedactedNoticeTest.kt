package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class RedactedNoticeTest {
    private val placeholders = setOf("Sensitive notification content hidden")

    @Test fun placeholderTextIsRedacted() {
        assertTrue(isRedactedText("Sensitive notification content hidden", placeholders))
        assertTrue(isRedactedText("  Sensitive notification content hidden\n", placeholders))
    }

    @Test fun normalTextIsNot() {
        assertFalse(isRedactedText("Your code is 123456", placeholders))
        assertFalse(isRedactedText(null, placeholders))
        assertFalse(isRedactedText("anything", emptySet()))
    }

    @Test fun thePcIsToldPlainly() =
        assertEquals("Code hidden by Android: open it on the phone", textForPc("Sensitive notification content hidden", placeholders))
}
