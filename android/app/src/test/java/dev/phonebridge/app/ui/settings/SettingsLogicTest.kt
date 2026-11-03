package dev.phonebridge.app.ui.settings

import dev.phonebridge.app.Group
import dev.phonebridge.app.SettingsNav
import org.junit.Assert.assertEquals
import org.junit.Test

class SettingsLogicTest {
    @Test fun nav_survives_save_and_restore() {
        for (n in listOf(SettingsNav(), SettingsNav(Group.Files), SettingsNav(null, "cam"), SettingsNav(Group.Settings, "x:y"))) {
            assertEquals(n, settingsNavFromSaved(n.toSaved()))
        }
    }

    @Test fun unreadable_saved_nav_is_the_top_list() {
        assertEquals(SettingsNav(), settingsNavFromSaved(emptyList()))
        assertEquals(SettingsNav(), settingsNavFromSaved(listOf("NoSuchGroup", "q")).copy(query = ""))
    }

    @Test fun pc_counts() {
        assertEquals("1 PC", pcCount(1))
        assertEquals("2 PCs", pcCount(2))
        assertEquals("0 PCs", pcCount(0))
    }

    @Test fun picture_denial() {
        assertEquals(PhotoDenial.None, photoDenial(granted = true, canAskAgain = false))
        assertEquals(PhotoDenial.Explain, photoDenial(granted = false, canAskAgain = true))
        assertEquals(PhotoDenial.OpenSettings, photoDenial(granted = false, canAskAgain = false))
    }
}
