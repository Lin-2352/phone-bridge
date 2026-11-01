package dev.phonebridge.app.ui

import org.junit.Assert.assertEquals
import org.junit.Test

class NavStateTest {
    @Test fun start_tab_defaults_to_home() {
        assertEquals(Tab.Home, startTab(null as String?, debuggable = true))
    }

    @Test fun start_tab_reads_the_route() {
        assertEquals(Tab.Remote, startTab("remote", debuggable = true))
        for (t in Tab.entries) assertEquals(t, startTab(t.route, debuggable = true))
    }

    @Test fun unknown_route_falls_back_to_home() {
        assertEquals(Tab.Home, startTab("photos", debuggable = true))
        assertEquals(Tab.Home, startTab("", debuggable = true))
        // Routes are lower case; the label is not a route.
        assertEquals(Tab.Home, startTab("Remote", debuggable = true))
    }

    @Test fun release_build_ignores_the_route() {
        // The extra is a test hook: MainActivity is exported, so release builds never honour it.
        assertEquals(Tab.Home, startTab("timeline", debuggable = false))
        assertEquals(Tab.Home, startTab("settings", debuggable = false))
    }

    @Test fun tabs_in_bar_order_with_routes_and_labels() {
        assertEquals(listOf("home", "timeline", "remote", "settings"), Tab.entries.map { it.route })
        assertEquals(listOf("Home", "Timeline", "Remote", "Settings"), Tab.entries.map { it.label })
    }
}
