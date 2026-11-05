package dev.phonebridge.app

import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class LogPrivacyTest {
    @Test
    fun notify_log_line_has_no_package() {
        val line = notifyLogLine("posted", "ab12")
        assertEquals("NOTIFY posted key=ab12", line)
        assertFalse(line.contains("pkg="))
    }

    @Test
    fun logs_live_in_internal_storage() {
        val d = logDirs(File("/data/user/0/app/files"), File("/sdcard/Android/data/app/files/logs"))
        assertEquals(File("/data/user/0/app/files/logs"), d.current)
    }

    @Test
    fun old_external_logs_are_marked_for_removal() {
        val ext = File("/sdcard/Android/data/app/files/logs")
        assertEquals(ext, logDirs(File("/data/user/0/app/files"), ext).legacy)
        assertNull(logDirs(File("/data/user/0/app/files"), null).legacy)
        assertTrue(logDirs(File("/x"), File("/x/logs")).legacy == null) // never delete the current folder
    }
}
