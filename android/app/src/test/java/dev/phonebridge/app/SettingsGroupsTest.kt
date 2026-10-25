package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Test

class SettingsGroupsTest {
    @Test
    fun devices_belong_to_connection() {
        assertEquals(Group.Connection, groupFor("Devices"))
    }

    @Test
    fun control_from_pc_belongs_to_screen() {
        assertEquals(Group.Screen, groupFor("Control from PC"))
    }

    @Test
    fun sharing_belongs_to_files() {
        assertEquals(Group.Files, groupFor("Sharing"))
    }

    @Test
    fun appearance_clipboard_and_background_belong_to_settings() {
        for (s in listOf("Appearance", "Clipboard", "Background")) {
            assertEquals(s, Group.Settings, groupFor(s))
        }
    }

    @Test
    fun an_unknown_section_falls_back_to_settings() {
        assertEquals(Group.Settings, groupFor("Something new"))
    }

    @Test
    fun groups_are_listed_in_display_order() {
        assertEquals(
            listOf("Connection", "Camera", "Screen", "Files", "Settings"),
            Group.entries.map(::groupTitle),
        )
    }
}
