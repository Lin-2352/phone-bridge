package dev.phonebridge.app.ui

import androidx.compose.material3.ColorScheme

/** The glass look the user chose: Native glass (Material 3, dynamic colour where the system has it) or Relay glass. */
enum class Look { Native, Relay }

/**
 * The appearance settings. `mode` is the theme mode from Prefs.theme (0 system, 1 light, 2 dark); `liteForced`
 * is the manual lite switch (it can force lite on, never off while an automatic reason applies).
 */
data class Appearance(
    val look: Look,
    val mode: Int,
    val reduceTransparency: Boolean,
    val liteForced: Boolean,
) {
    companion object {
        /** First run: Native glass, the system's mode, full effects. */
        val DEFAULT = Appearance(look = Look.Native, mode = 0, reduceTransparency = false, liteForced = false)
    }
}

/** The glass material for a look and a mode: `Tokens.Materials` is ordered native light, native dark, relay light, relay dark. */
fun materialFor(look: Look, dark: Boolean): GlassMaterial =
    Tokens.Materials[look.ordinal * 2 + if (dark) 1 else 0]

/**
 * The colour scheme for a look: Native glass uses the system's dynamic colour from Android 12 (API 31) when
 * the system offers one, otherwise the Relay scheme. Relay glass always uses the Relay scheme.
 */
fun colorSchemeFor(
    look: Look,
    dark: Boolean,
    sdk: Int,
    dynamic: (Boolean) -> ColorScheme?,
): ColorScheme {
    val relay = if (dark) relayDarkScheme() else relayLightScheme()
    if (look == Look.Relay || sdk < 31) return relay
    return dynamic(dark) ?: relay
}
