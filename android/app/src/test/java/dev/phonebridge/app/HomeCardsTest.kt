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
    fun no_pair_tile_while_not_paired_because_the_hero_card_already_has_the_button() {
        val tiles = quickTiles(running = true, paired = false, cameraOn = false, screenOn = false)
        assertTrue(tiles.none { it.quick == Quick.Pair })
        assertTrue(tiles.none { it.enabled })
        assertEquals(5, tiles.size)
    }

    @Test
    fun a_dimmed_tile_says_why() {
        assertEquals("Turn on $APP_NAME first", tileDisabledReason(running = false, paired = true))
        assertEquals("Turn on $APP_NAME first", tileDisabledReason(running = false, paired = false))
        assertEquals("Pair a PC first", tileDisabledReason(running = true, paired = false))
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

class HeroPcTest {
    @Test fun live_name_wins() = assertEquals(HeroPc("Laptop", false), heroPc("Laptop", "Old", running = true, ready = true))
    @Test fun off_shows_stored_name_dimmed() = assertEquals(HeroPc("Laptop", true), heroPc(null, "Laptop", running = false, ready = false))
    @Test fun starting_shows_stored_name_dimmed() = assertEquals(HeroPc("Laptop", true), heroPc(null, "Laptop", running = true, ready = false))
    @Test fun really_unpaired_shows_nothing() = assertEquals(HeroPc(null, false), heroPc(null, "Laptop", running = true, ready = true))
    @Test fun never_paired_shows_nothing() = assertEquals(HeroPc(null, false), heroPc(null, null, running = false, ready = false))
}
