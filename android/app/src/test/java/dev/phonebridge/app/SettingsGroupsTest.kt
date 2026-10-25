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
            listOf("Connection", "Camera", "Screen", "Notifications", "Files", "Settings"),
            Group.entries.map(::groupTitle),
        )
    }
}

class SettingsSearchTest {
    private val index = listOf(
        SettingEntry(Group.Camera, "PCs always allowed to use the camera", "Skip the question"),
        SettingEntry(Group.Files, "Save files to", "Downloads folder"),
        SettingEntry(Group.Settings, "Share clipboard", "Share what you copy"),
    )

    @Test
    fun an_empty_or_blank_query_matches_nothing() {
        assertEquals(emptyList<SettingEntry>(), searchSettings("", index))
        assertEquals(emptyList<SettingEntry>(), searchSettings("   ", index))
    }

    @Test
    fun matches_the_title_ignoring_case() {
        assertEquals(listOf(index[1]), searchSettings("SAVE", index))
    }

    @Test
    fun matches_the_subtitle() {
        assertEquals(listOf(index[2]), searchSettings("what you copy", index))
    }

    @Test
    fun no_match_gives_an_empty_list() {
        assertEquals(emptyList<SettingEntry>(), searchSettings("zebra", index))
    }

    @Test
    fun results_keep_index_order() {
        assertEquals(index, searchSettings("a", index))
    }

    @Test
    fun the_real_index_covers_every_group_and_finds_the_clipboard() {
        assertEquals(Group.entries.toSet(), settingEntries.map { it.group }.toSet())
        assertEquals(Group.Settings, searchSettings("clipboard").first().group)
    }

    @Test
    fun every_page_has_a_summary() {
        for (g in Group.entries) assertEquals(g.name, false, pageSummary(g).isBlank())
    }
}

class SettingsNavTest {
    @Test
    fun opening_a_page_then_back_returns_to_the_list() {
        val n = SettingsNav().open(Group.Camera)
        assertEquals(Group.Camera, n.page)
        assertEquals(SettingsNav(), n.back())
    }

    @Test
    fun back_from_the_list_clears_the_query_before_leaving() {
        val n = SettingsNav(query = "cam")
        assertEquals(SettingsNav(), n.back())
        assertEquals(null, SettingsNav().back())
    }

    @Test
    fun back_from_a_page_opened_by_search_keeps_the_query_on_the_list() {
        val n = SettingsNav(query = "cam").open(Group.Camera)
        assertEquals(SettingsNav(query = "cam"), n.back())
    }
}

class NotificationsPageTest {
    @Test
    fun notifications_have_their_own_page_that_search_finds() {
        assertEquals(Group.Notifications, groupFor("Notifications"))
        val hit = searchSettings("notifications on pc").single()
        assertEquals(Group.Notifications, hit.group)
    }

    @Test
    fun the_home_warning_shows_only_when_the_switch_is_on_and_access_is_missing() {
        assertEquals(true, notifyAccessMissing(notifyToPc = true, access = false))
        assertEquals(false, notifyAccessMissing(notifyToPc = true, access = true))
        assertEquals(false, notifyAccessMissing(notifyToPc = false, access = false))
    }
}
