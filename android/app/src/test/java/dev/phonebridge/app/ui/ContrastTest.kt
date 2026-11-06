package dev.phonebridge.app.ui

import androidx.compose.material3.ColorScheme
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.compositeOver
import androidx.compose.ui.unit.dp
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.pow

class ContrastTest {
    private fun lin(c: Float) = if (c <= 0.03928f) c / 12.92 else ((c + 0.055) / 1.055).pow(2.4)
    private fun lum(c: Color) = 0.2126 * lin(c.red) + 0.7152 * lin(c.green) + 0.0722 * lin(c.blue)
    private fun ratio(a: Color, b: Color): Double {
        val (x, y) = lum(a) to lum(b)
        return (maxOf(x, y) + 0.05) / (minOf(x, y) + 0.05)
    }

    private fun backgrounds(s: ColorScheme) = mapOf(
        "surface" to s.surface, "lowest" to s.surfaceContainerLowest, "low" to s.surfaceContainerLow,
        "container" to s.surfaceContainer, "high" to s.surfaceContainerHigh, "highest" to s.surfaceContainerHighest,
    )

    private fun check(name: String, s: ColorScheme) {
        for ((n, bg) in backgrounds(s)) {
            assertTrue("$name onSurface on $n", ratio(s.onSurface, bg) >= 4.5)
            assertTrue("$name onSurfaceVariant on $n", ratio(s.onSurfaceVariant, bg) >= 4.5)
            assertTrue("$name outline on $n: ${ratio(s.outline, bg)}", ratio(s.outline, bg) >= 3.0)
        }
        assertTrue("$name onPrimary on primary", ratio(s.onPrimary, s.primary) >= 4.5)
        assertTrue("$name onError on error", ratio(s.onError, s.error) >= 4.5)
    }

    @Test fun light_contrast() = check("light", relayLightScheme())
    @Test fun dark_contrast() = check("dark", relayDarkScheme())

    /** An amber icon on its own tint (`IconBadge`: 14% in light, 22% in dark over the page or a card) is a 3:1 pair. */
    @Test fun amber_icon_badges_are_3_to_1() {
        for ((name, t, alpha) in listOf(Triple("light", Tokens.Light, 0.14f), Triple("dark", Tokens.Dark, 0.22f))) {
            for ((bgName, bg) in listOf("surface" to t.surface, "surfaceVariant" to t.surfaceVariant)) {
                val badge = t.attention.copy(alpha = alpha).compositeOver(bg)
                val r = ratio(t.attention, badge)
                assertTrue("$name amber icon on its tint over $bgName: $r", r >= 3.0)
            }
        }
    }

    /** The pairing code's digits are drawn in onSurface on a 16% amber tint, so they keep a very high contrast. */
    @Test fun pairing_code_digits_are_aa_on_the_amber_tint() {
        for ((name, t, s) in listOf(Triple("light", Tokens.Light, relayLightScheme()), Triple("dark", Tokens.Dark, relayDarkScheme()))) {
            for ((bgName, bg) in backgrounds(s)) {
                val chip = t.attention.copy(alpha = 0.16f).compositeOver(bg)
                val r = ratio(s.onSurface, chip)
                assertTrue("$name digits on the amber tint over $bgName: $r", r >= 7.0)
            }
        }
    }

    @Test fun ladder_is_monotonic_lighter_than_page() {
        for (s in listOf(relayLightScheme(), relayDarkScheme())) {
            val l = listOf(s.surfaceContainerLowest, s.surfaceContainerLow, s.surfaceContainer, s.surfaceContainerHigh, s.surfaceContainerHighest).map(::lum)
            assertTrue(l.zipWithNext().all { (a, b) -> b > a })
            assertEquals(s.surface, s.background)
        }
    }

    @Test fun hairline_token_is_outline_variant() {
        assertEquals(Tokens.Light.outline, relayLightScheme().outlineVariant)
        assertEquals(Tokens.Dark.outline, relayDarkScheme().outlineVariant)
    }

    @Test fun shapes_use_token_radii() {
        assertEquals(androidx.compose.foundation.shape.RoundedCornerShape(8.dp), RelayShapes.small)
        assertEquals(androidx.compose.foundation.shape.RoundedCornerShape(12.dp), RelayShapes.medium)
        assertEquals(androidx.compose.foundation.shape.RoundedCornerShape(20.dp), RelayShapes.large)
    }
}
