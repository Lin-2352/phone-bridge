package dev.phonebridge.app.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class OemTest {
    @Test
    fun oem_detection_by_maker_and_brand() {
        assertEquals(Oem.Xiaomi, oemOf("Xiaomi", "Xiaomi"))
        assertEquals(Oem.Xiaomi, oemOf("Xiaomi", "POCO"))
        assertEquals(Oem.Samsung, oemOf("samsung", "samsung"))
        assertEquals(Oem.Huawei, oemOf("HUAWEI", "HONOR"))
        assertEquals(Oem.OppoRealme, oemOf("OPPO", "OPPO"))
        assertEquals(Oem.OppoRealme, oemOf("realme", "realme"))
        assertEquals(Oem.Other, oemOf("Google", "google"))
    }

    @Test
    fun oneplus_counts_as_oppo_realme() {
        assertEquals(Oem.OppoRealme, oemOf("OnePlus", "OnePlus"))
    }

    @Test
    fun every_oem_has_steps_and_a_fallback() {
        for (oem in Oem.values()) {
            val g = oemGuide(oem)
            assertTrue("$oem has steps", g.steps.isNotEmpty())
            if (oem == Oem.Other) {
                assertTrue("other makers open the app's own settings", g.intents.isEmpty())
            } else {
                assertFalse("$oem has a settings screen to open", g.intents.isEmpty())
            }
        }
        assertEquals(
            listOf("com.miui.securitycenter/com.miui.permcenter.autostart.AutoStartManagementActivity"),
            oemGuide(Oem.Xiaomi).intents,
        )
    }
}
