package dev.phonebridge.app.ui

import androidx.compose.material3.ColorScheme
import androidx.compose.ui.graphics.Color
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AppearanceTest {
    private val marker: ColorScheme = relayLightScheme().copy(primary = Color.Red)

    /** ColorScheme has no value equality, so compare the roles that decide how a screen is drawn. */
    private fun sameRoles(a: ColorScheme, b: ColorScheme) =
        a.primary == b.primary && a.surface == b.surface && a.onSurface == b.onSurface && a.background == b.background

    @Test
    fun material_index_matches_look_and_mode() {
        assertEquals(Tokens.Materials[0], materialFor(Look.Native, dark = false))
        assertEquals(Tokens.Materials[1], materialFor(Look.Native, dark = true))
        assertEquals(Tokens.Materials[2], materialFor(Look.Relay, dark = false))
        assertEquals(Tokens.Materials[3], materialFor(Look.Relay, dark = true))
    }

    @Test
    fun native_below_31_uses_the_relay_scheme() {
        val noDynamic: (Boolean) -> ColorScheme? = { null }
        assertTrue(sameRoles(relayLightScheme(), colorSchemeFor(Look.Native, false, 30, noDynamic)))
        assertTrue(sameRoles(relayDarkScheme(), colorSchemeFor(Look.Native, true, 30, noDynamic)))
    }

    @Test
    fun native_on_31_uses_dynamic_when_available() {
        val dynamic: (Boolean) -> ColorScheme? = { marker }
        assertEquals(marker, colorSchemeFor(Look.Native, false, 31, dynamic))
        assertEquals(marker, colorSchemeFor(Look.Native, true, 36, dynamic))
    }

    @Test
    fun relay_ignores_dynamic_colour() {
        val dynamic: (Boolean) -> ColorScheme? = { marker }
        assertTrue(sameRoles(relayLightScheme(), colorSchemeFor(Look.Relay, false, 36, dynamic)))
        assertFalse(sameRoles(marker, colorSchemeFor(Look.Relay, false, 36, dynamic)))
    }

    @Test
    fun defaults_are_native_system_full_effects() {
        val d = Appearance.DEFAULT
        assertEquals(Look.Native, d.look)
        assertEquals(0, d.mode)
        assertEquals(false, d.reduceTransparency)
        assertEquals(false, d.liteForced)
    }
}
