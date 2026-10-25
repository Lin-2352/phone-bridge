package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class HomeCardsTest {
    @Test
    fun tiles_are_disabled_while_not_connected() {
        val off = homeTiles(connected = false, cameraOn = false, screenOn = false, clipboardOn = false)
        assertEquals(listOf(Group.Settings), off.filter { it.enabled }.map { it.group })
        assertTrue(homeTiles(connected = true, cameraOn = false, screenOn = false).all { it.enabled })
    }

    @Test
    fun camera_tile_shows_on_while_streaming() {
        val tiles = homeTiles(connected = true, cameraOn = true, screenOn = false)
        assertTrue(tiles.first { it.group == Group.Camera }.on)
        assertFalse(tiles.first { it.group == Group.Screen }.on)
    }

    @Test
    fun tiles_follow_group_order() {
        val groups = homeTiles(connected = true, cameraOn = false, screenOn = false).map { it.group }
        val order = listOf(Group.Camera, Group.Screen, Group.Files)
        assertEquals(order, groups.filter { it in order })
    }

    @Test
    fun send_file_tile_is_never_on() {
        for (c in listOf(true, false)) for (s in listOf(true, false)) {
            val tile = homeTiles(connected = true, cameraOn = c, screenOn = s).first { it.group == Group.Files }
            assertFalse(tile.on)
        }
    }

    @Test
    fun a_live_camera_tile_stays_enabled_without_a_connection() {
        val tiles = homeTiles(connected = false, cameraOn = true, screenOn = true, clipboardOn = false)
        assertTrue(tiles.first { it.group == Group.Camera }.enabled)
        assertTrue(tiles.first { it.group == Group.Screen }.enabled)
        assertFalse(tiles.first { it.group == Group.Files }.enabled)
    }

    @Test
    fun clipboard_tile_follows_its_setting_and_is_always_enabled() {
        for (c in listOf(true, false)) {
            val tile = homeTiles(connected = false, cameraOn = false, screenOn = false, clipboardOn = c).first { it.group == Group.Settings }
            assertEquals(c, tile.on)
            assertTrue(tile.enabled)
        }
    }
}
