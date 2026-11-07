package dev.phonebridge.app.ui

import androidx.compose.animation.core.SpringSpec
import androidx.compose.animation.core.TweenSpec
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class MotionTest {
    private val spring = TokenSpring(stiffness = 700f, dampingRatio = 0.9f)

    @Test
    fun normal_modes_give_the_token_spring() {
        val spec = tokenSpec<Float>(spring, Modes())
        assertTrue("a spring", spec is SpringSpec<*>)
        assertEquals(700f, (spec as SpringSpec<Float>).stiffness, 0f)
    }

    @Test
    fun reduced_motion_gives_a_fast_tween() {
        val spec = tokenSpec<Float>(spring, Modes(reducedMotion = true))
        assertTrue("a tween", spec is TweenSpec<*>)
        assertEquals(Tokens.Motion.FAST_MS, (spec as TweenSpec<Float>).durationMillis)
    }

    @Test
    fun lite_gives_a_fast_tween() {
        val spec = tokenSpec<Float>(spring, Modes(lite = true))
        assertTrue("a tween", spec is TweenSpec<*>)
        assertEquals(Tokens.Motion.FAST_MS, (spec as TweenSpec<Float>).durationMillis)
    }
}
