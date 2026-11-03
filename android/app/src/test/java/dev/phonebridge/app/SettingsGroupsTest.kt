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
            listOf("Connection", "Camera", "Screen", "Audio", "Notifications", "Files", "General"),
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

class ClipboardImagesSettingsTest {
    @Test
    fun the_two_image_switches_live_under_settings_and_search_finds_them() {
        val copied = searchSettings("copied images").single()
        assertEquals(Group.Settings, copied.group)
        val shots = searchSettings("screenshots").single()
        assertEquals(Group.Settings, shots.group)
    }
}

class SettingsStatusTest {
    private val base = StatusInput(saveLabel = "Downloads/Relay", logSize = "4.2 MB")

    @Test
    fun connection_names_the_pc() {
        assertEquals("No PC paired yet", statusLine(Group.Connection, base))
        assertEquals("Paired with DeskTray", statusLine(Group.Connection, base.copy(pairedNames = listOf("DeskTray"))))
        assertEquals("Paired with A and 2 more", statusLine(Group.Connection, base.copy(pairedNames = listOf("A", "B", "C"))))
    }

    @Test
    fun camera_counts_allowed_pcs() {
        assertEquals("Ask each time", statusLine(Group.Camera, base))
        assertEquals("1 PC always allowed", statusLine(Group.Camera, base.copy(cameraAllowedCount = 1)))
        assertEquals("2 PCs always allowed", statusLine(Group.Camera, base.copy(cameraAllowedCount = 2)))
    }

    @Test
    fun screen_reflects_permission_then_choice() {
        assertEquals("Control needs accessibility permission", statusLine(Group.Screen, base.copy(controlAllowed = true)))
        assertEquals("PC control allowed", statusLine(Group.Screen, base.copy(controlServiceOn = true, controlAllowed = true)))
        assertEquals("PC control asks first", statusLine(Group.Screen, base.copy(controlServiceOn = true)))
    }

    @Test
    fun notifications_off_missing_access_and_on() {
        assertEquals("Off", statusLine(Group.Notifications, base))
        assertEquals("On, needs notification access", statusLine(Group.Notifications, base.copy(notifyOn = true)))
        assertEquals("On, showing on your PC", statusLine(Group.Notifications, base.copy(notifyOn = true, notifyAccess = true)))
    }

    @Test
    fun files_and_general() {
        assertEquals("Saving to Downloads/Relay", statusLine(Group.Files, base))
        assertEquals("Theme dark, logs 4.2 MB", statusLine(Group.Settings, base.copy(themeName = "Dark")))
        assertEquals("Theme system, logs off", statusLine(Group.Settings, base.copy(keepLogs = false)))
    }

    @Test
    fun search_finds_the_per_app_notification_block_list() {
        for (q in listOf("block", "banking", "apps on your pc")) {
            val hits = searchSettings(q)
            assertEquals(q, true, hits.any { it.group == Group.Notifications && it.title == "Apps on your PC" })
        }
    }

    @Test
    fun accessibility_is_reached_through_the_control_row_only() {
        assertEquals(false, settingEntries.any { it.title == "Accessibility permission" })
        assertEquals(listOf("Let my PC control this phone"), searchSettings("accessibility").map { it.title })
    }

    @Test
    fun audio_status_says_relay_not_our() {
        assertEquals("Phone audio starts without $APP_NAME's question", statusLine(Group.Audio, base.copy(audioSkipAsk = true)))
    }
}
