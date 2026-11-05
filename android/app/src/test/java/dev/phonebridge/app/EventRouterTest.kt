package dev.phonebridge.app

import java.util.concurrent.Executor
import org.junit.Assert.assertEquals
import org.junit.Test
import uniffi.pb_ffi.PbEvent

class EventRouterTest {
    private class Recording : Executor {
        val tasks = mutableListOf<Runnable>()
        override fun execute(r: Runnable) { tasks += r }
    }

    /** A multi-GB copy into MediaStore must not hold up the PC's control, screen or audio question (B1). */
    @Test fun fileSavesDoNotBlockControlRequests() {
        val events = Recording()
        val saves = Recording()
        val router = EventRouter(events, saves)
        val handled = mutableListOf<String>()
        router.route(PbEvent.FileReceived("PC", "/tmp/a", "a.mkv", 4_000_000_000uL)) { handled += "file" }
        router.route(PbEvent.ControlRequest(7uL, "PC")) { handled += "control" }
        assertEquals(1, saves.tasks.size)
        assertEquals(1, events.tasks.size)
        events.tasks.single().run() // the save is still queued on its own thread
        assertEquals(listOf("control"), handled)
    }
}
