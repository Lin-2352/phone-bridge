package dev.phonebridge.app.ui

import androidx.compose.animation.core.FiniteAnimationSpec
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween

/**
 * The animation for a token spring: the spring itself normally, and a fast tween when reduced motion or lite
 * mode asks for no travel (lite also stops idle motion; see LinkIndicator).
 */
fun <T> tokenSpec(s: TokenSpring, modes: Modes): FiniteAnimationSpec<T> {
    val spring = springOrCut(s, modes)
    return if (spring == null || modes.lite) {
        tween(durationMillis = Tokens.Motion.FAST_MS)
    } else {
        spring(dampingRatio = spring.dampingRatio, stiffness = spring.stiffness)
    }
}
