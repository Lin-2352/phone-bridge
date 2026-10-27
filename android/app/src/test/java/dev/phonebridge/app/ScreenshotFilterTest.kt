package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ScreenshotFilterTest {
    private val now = 1_000_000L

    private fun shot(
        id: Long = 1,
        path: String? = "Pictures/Screenshots/",
        name: String? = "Screenshot_20261027_101500.jpg",
        added: Long = now - 2,
        pending: Boolean = false,
        size: Long = 500_000,
    ) = MediaRow(id, path, name, added, pending, size)

    @Test
    fun a_fresh_screenshot_in_the_screenshots_folder_is_sent() {
        assertTrue(isScreenshot(shot(), now, emptySet()))
    }

    @Test
    fun a_screenshot_is_recognised_by_folder_or_by_file_name() {
        assertTrue(isScreenshot(shot(path = "DCIM/Screenshots/", name = "a.png"), now, emptySet()))
        assertTrue(isScreenshot(shot(path = "Pictures/", name = "Screenshot_1.png"), now, emptySet()))
        assertTrue(isScreenshot(shot(path = "pictures/SCREENSHOTS/", name = "x.png"), now, emptySet()))
    }

    @Test
    fun an_ordinary_photo_is_not_a_screenshot() {
        assertFalse(isScreenshot(shot(path = "DCIM/Camera/", name = "IMG_1.jpg"), now, emptySet()))
        assertFalse(isScreenshot(shot(path = null, name = null), now, emptySet()))
    }

    @Test
    fun a_file_still_being_written_waits() {
        assertFalse(isScreenshot(shot(pending = true), now, emptySet()))
    }

    @Test
    fun an_old_screenshot_is_not_sent_when_the_gallery_changes() {
        assertFalse(isScreenshot(shot(added = now - 3600), now, emptySet()))
    }

    @Test
    fun a_screenshot_already_handled_is_not_sent_twice() {
        assertFalse(isScreenshot(shot(id = 7), now, setOf(7L)))
        assertTrue(isScreenshot(shot(id = 8), now, setOf(7L)))
    }

    @Test
    fun empty_and_oversized_files_are_skipped() {
        assertFalse(isScreenshot(shot(size = 0), now, emptySet()))
        assertFalse(isScreenshot(shot(size = MAX_IMAGE_BYTES + 1), now, emptySet()))
        assertTrue(isScreenshot(shot(size = MAX_IMAGE_BYTES), now, emptySet()))
    }

    @Test
    fun the_seen_list_keeps_only_the_newest_ids() {
        var seen = emptyList<Long>()
        for (i in 1L..60L) seen = rememberSeen(seen, i)
        assertEquals(SEEN_LIMIT, seen.size)
        assertTrue(60L in seen)
        assertFalse(1L in seen)
        // Remembering an id twice does not duplicate it.
        assertEquals(seen, rememberSeen(seen, 60L))
    }

    @Test
    fun the_warning_shows_when_screenshots_are_on_but_access_is_missing() {
        assertTrue(screenshotAccessMissing(share = true, granted = false))
        assertFalse(screenshotAccessMissing(share = true, granted = true))
        assertFalse(screenshotAccessMissing(share = false, granted = false))
    }
}
