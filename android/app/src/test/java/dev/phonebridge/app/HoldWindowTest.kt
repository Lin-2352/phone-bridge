package dev.phonebridge.app

import android.view.WindowManager.LayoutParams as P
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class HoldWindowTest {
    @Test
    fun touchesPassThroughToTheAppUnderneath() {
        assertTrue(holdWindowFlags() and P.FLAG_NOT_TOUCHABLE != 0)
    }

    @Test
    fun theWindowStaysFocusableSoTheSystemAlwaysHasAFocusWindow() {
        // realme's watchdog kills an app whose foreground activity has no
        // focusable window for about 13 s ("NoFocusWindow" ANR): the camera
        // stream died every time that happened.
        assertEquals(0, holdWindowFlags() and P.FLAG_NOT_FOCUSABLE)
    }
}
