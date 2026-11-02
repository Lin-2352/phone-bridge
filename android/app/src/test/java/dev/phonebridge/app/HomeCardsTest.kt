package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class HomeCardsTest {
    private fun tile(q: Quick, running: Boolean = true, paired: Boolean = true, cameraOn: Boolean = false, screenOn: Boolean = false) =
        quickTiles(running, paired, cameraOn, screenOn).first { it.quick == q }

    @Test
    fun grid_is_two_by_three_in_order() {
        val tiles = quickTiles(running = true, paired = true, cameraOn = false, screenOn = false)
        assertEquals(listOf(Quick.SendFile, Quick.SendClipboard, Quick.Screen, Quick.Camera, Quick.Touchpad, Quick.Pair), tiles.map { it.quick })
        assertEquals(3, tiles.chunked(2).size)
    }

    @Test
    fun everything_works_while_paired_and_running() {
        assertTrue(quickTiles(running = true, paired = true, cameraOn = false, screenOn = false).all { it.enabled })
    }

    @Test
    fun only_pair_works_while_not_paired() {
        val tiles = quickTiles(running = true, paired = false, cameraOn = false, screenOn = false)
        assertEquals(listOf(Quick.Pair), tiles.filter { it.enabled }.map { it.quick })
    }

    @Test
    fun nothing_but_live_shares_works_while_off() {
        assertTrue(quickTiles(running = false, paired = true, cameraOn = false, screenOn = false).none { it.enabled })
        val live = quickTiles(running = false, paired = false, cameraOn = true, screenOn = true)
        assertEquals(listOf(Quick.Screen, Quick.Camera), live.filter { it.enabled }.map { it.quick })
    }

    @Test
    fun screen_and_camera_show_on_while_live() {
        assertTrue(tile(Quick.Camera, cameraOn = true).on)
        assertFalse(tile(Quick.Screen, cameraOn = true).on)
        assertTrue(tile(Quick.Screen, screenOn = true).on)
        for (q in listOf(Quick.SendFile, Quick.SendClipboard, Quick.Touchpad, Quick.Pair)) {
            assertFalse(tile(q, cameraOn = true, screenOn = true).on)
        }
    }
}
