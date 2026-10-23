package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Test

class TileLogicTest {
    @Test
    fun turningOnWhileUnlockedGoesThroughTheForegroundActivity() {
        // A background service start can be refused silently by the system
        // (realme): with the phone unlocked the tile never relies on it.
        assertEquals(TileClick.StartViaActivity, tileClick(shownOn = false, locked = false))
    }

    @Test
    fun turningOnWhileLockedStartsDirectlyWithoutAnUnlockPrompt() {
        // On the lock screen a tap must not demand a fingerprint first.
        assertEquals(TileClick.StartDirect, tileClick(shownOn = false, locked = true))
    }

    @Test
    fun turningOffStopsTheServiceLockedOrNot() {
        assertEquals(TileClick.Stop, tileClick(shownOn = true, locked = false))
        assertEquals(TileClick.Stop, tileClick(shownOn = true, locked = true))
    }

    @Test
    fun aKilledAppIsShownAsStoppedNotAsOn() {
        // The saved wish is "on" but nothing runs (Android killed it): the
        // tile says so and a tap starts it.
        assertEquals("Stopped. Tap to start", tileSubtitle(wished = true, running = false, starting = false, linked = null))
        assertEquals("Off", tileSubtitle(wished = false, running = false, starting = false, linked = null))
    }

    @Test
    fun theSubtitleFollowsTheRealState() {
        assertEquals("Starting\u2026", tileSubtitle(wished = true, running = false, starting = true, linked = null))
        assertEquals("On", tileSubtitle(wished = true, running = true, starting = false, linked = null))
        assertEquals("RMX PC", tileSubtitle(wished = true, running = true, starting = false, linked = "RMX PC"))
    }
}
