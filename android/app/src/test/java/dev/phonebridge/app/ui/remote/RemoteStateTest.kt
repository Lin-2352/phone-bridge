package dev.phonebridge.app.ui.remote

import org.junit.Assert.assertEquals
import org.junit.Test

class RemoteStateTest {
    @Test
    fun off_wins_over_everything() {
        assertEquals(RemoteState.Off, remoteState(false, false, 0))
        assertEquals(RemoteState.Off, remoteState(false, true, 1))
    }

    @Test
    fun starting_until_the_node_is_ready_even_if_peers_unknown() {
        assertEquals(RemoteState.Starting, remoteState(true, false, 0))
        assertEquals(RemoteState.Starting, remoteState(true, false, 2))
    }

    @Test
    fun needs_pairing_only_when_ready_without_peers() {
        assertEquals(RemoteState.NeedsPairing, remoteState(true, true, 0))
    }

    @Test
    fun ready_with_a_peer() {
        assertEquals(RemoteState.Ready, remoteState(true, true, 1))
    }
}
