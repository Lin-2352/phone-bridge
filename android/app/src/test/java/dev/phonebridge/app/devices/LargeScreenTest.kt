package dev.phonebridge.app.devices

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class LargeScreenTest {
    private val pkg = "dev.phonebridge.app"

    @Test
    fun a_window_600dp_wide_or_more_is_a_large_screen() {
        assertFalse(isLargeScreen(599))
        assertTrue(isLargeScreen(600))
        assertTrue(isLargeScreen(1280))
        assertFalse(isLargeScreen(0))
    }

    @Test
    fun a_file_from_another_app_is_accepted() {
        assertTrue(dropAllowed("content://com.android.providers.media.documents/document/image%3A12", pkg, emptyList()))
    }

    @Test
    fun only_content_uris_are_accepted() {
        assertFalse(dropAllowed("file:///sdcard/secret.txt", pkg, emptyList()))
        assertFalse(dropAllowed("http://example.com/a.png", pkg, emptyList()))
        assertFalse(dropAllowed("", pkg, emptyList()))
    }

    @Test
    fun our_own_files_are_never_sent_by_a_drop() {
        assertFalse(dropAllowed("content://$pkg.files/identity/seed", pkg, emptyList()))
        assertFalse(dropAllowed("content://0@$pkg.files/identity/seed", pkg, emptyList()))
    }

    @Test
    fun a_file_inside_the_save_folder_is_not_sent_back() {
        val tree = "content://com.android.externalstorage.documents/tree/primary%3ADownload%2FRelay"
        val inside = "content://com.android.externalstorage.documents/tree/primary%3ADownload%2FRelay/document/primary%3ADownload%2FRelay%2Fa.txt"
        assertFalse(dropAllowed(inside, pkg, listOf(tree)))
        assertTrue(dropAllowed("content://com.android.externalstorage.documents/document/primary%3ADocuments%2Fb.txt", pkg, listOf(tree)))
    }

    @Test
    fun a_drop_keeps_the_order_and_skips_what_is_refused() {
        val ok1 = "content://media/external/file/1"
        val bad = "file:///x"
        val ok2 = "content://media/external/file/2"
        assertEquals(listOf(ok1, ok2), allowedDrops(listOf(ok1, bad, ok2), pkg, emptyList()))
    }
}
