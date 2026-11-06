package dev.phonebridge.app.ui

/** What the user and the system have asked for. */
data class Modes(
    val reducedTransparency: Boolean = false,
    val reducedMotion: Boolean = false,
    /** Relay's own lite mode: no effects at all. */
    val lite: Boolean = false,
    val highContrast: Boolean = false,
)

/**
 * The material to draw. Reduced transparency, lite and high contrast make it fully opaque with no blur,
 * grain or highlight, so text keeps its contrast whatever is behind the window. Written here, next to its
 * callers, and tested here; the PC has its own copy in `tokens_rules.rs`.
 */
fun resolve(m: GlassMaterial, modes: Modes): GlassMaterial =
    if (modes.reducedTransparency || modes.lite || modes.highContrast) {
        GlassMaterial(m.tint, opacity = 1f, blur = 0f, highlight = 0f, borderAlpha = m.borderAlpha, grain = 0f)
    } else {
        m
    }

/** The spring to animate with, or null to cut: reduced motion replaces travel with a short cross-fade. */
fun springOrCut(s: TokenSpring, modes: Modes): TokenSpring? = if (modes.reducedMotion) null else s
