package dev.phonebridge.app

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ShareFilterTest {
    private val pkg = "dev.phonebridge.app"

    @Test
    fun our_own_files_are_refused() {
        assertTrue(isOwnAuthority("dev.phonebridge.app.clips", pkg))
    }

    /** Android reads "0@authority" as user 0's copy of that provider: the same files. */
    @Test
    fun a_user_prefix_does_not_hide_our_own_files() {
        assertTrue(isOwnAuthority("0@dev.phonebridge.app.clips", pkg))
    }

    @Test
    fun another_apps_files_are_fine() {
        assertFalse(isOwnAuthority("com.android.providers.media", pkg))
        assertFalse(isOwnAuthority(null, pkg))
    }
}
