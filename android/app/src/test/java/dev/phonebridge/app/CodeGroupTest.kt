package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Test

class CodeGroupTest {
    @Test fun sixDigitsSplitInTwoThrees() {
        assertEquals("628 183", groupCode("628183"))
    }
    @Test fun oddLengthsGroupFromTheLeft() {
        assertEquals("628 18", groupCode("62818"))
        assertEquals("628 183 1", groupCode("6281831"))
        assertEquals("1", groupCode("1"))
        assertEquals("123 4", groupCode("1234"))
    }
    @Test fun surroundingWhitespaceIsDropped() = assertEquals("628 183", groupCode(" 628183\n"))
    @Test fun nonDigitsAreLeftAlone() {
        assertEquals("62a183", groupCode("62a183"))
        assertEquals("", groupCode(""))
    }
    @Test fun alreadyGroupedIsKept() {
        assertEquals("628 183", groupCode("628 183"))
    }
}
