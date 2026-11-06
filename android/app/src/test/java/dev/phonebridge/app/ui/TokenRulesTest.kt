package dev.phonebridge.app.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** M4.3: the two rules the glass and motion tokens come with, written next to their callers. */
class TokenRulesTest {
    @Test fun reduced_transparency_is_opaque() {
        for (modes in listOf(Modes(reducedTransparency = true), Modes(lite = true), Modes(highContrast = true))) {
            for (m in Tokens.Materials) {
                val r = resolve(m, modes)
                assertEquals("$modes", 1f, r.opacity, 0f)
                assertEquals(0f, r.blur, 0f)
                assertEquals(0f, r.grain, 0f)
                assertEquals(0f, r.highlight, 0f)
                assertEquals("the colour is kept", m.tint, r.tint)
            }
        }
    }

    @Test fun normal_modes_leave_the_material_alone() {
        val m = Tokens.Materials[3]
        assertEquals(m.opacity, resolve(m, Modes()).opacity, 0f)
        assertEquals(m.blur, resolve(m, Modes(reducedMotion = true)).blur, 0f)
    }

    @Test fun reduced_motion_cuts_springs() {
        val s = Tokens.Springs.Standard
        assertNull(springOrCut(s, Modes(reducedMotion = true)))
        assertEquals(s, springOrCut(s, Modes()))
    }

    @Test fun effects_springs_never_overshoot() {
        assertEquals(true, Tokens.Springs.Effects.dampingRatio >= 1f)
    }
}
