package dev.phonebridge.app

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Test

class ImageAccessTest {
    @Test
    fun user_selected_is_requested_from_34() {
        assertArrayEquals(
            arrayOf("android.permission.READ_MEDIA_IMAGES", "android.permission.READ_MEDIA_VISUAL_USER_SELECTED"),
            imagePermissions(34),
        )
        assertArrayEquals(arrayOf("android.permission.READ_MEDIA_IMAGES"), imagePermissions(33))
        assertArrayEquals(arrayOf("android.permission.READ_EXTERNAL_STORAGE"), imagePermissions(32))
    }

    @Test
    fun partial_grant_is_partial() {
        assertEquals(ImageAccess.PARTIAL, imageAccess(34, full = false, partial = true))
    }

    @Test
    fun full_wins_over_partial() {
        assertEquals(ImageAccess.FULL, imageAccess(34, full = true, partial = true))
    }

    @Test
    fun nothing_granted_is_none() {
        assertEquals(ImageAccess.NONE, imageAccess(34, full = false, partial = false))
        assertEquals(ImageAccess.NONE, imageAccess(33, full = false, partial = true)) // no such grant before 14
    }
}
