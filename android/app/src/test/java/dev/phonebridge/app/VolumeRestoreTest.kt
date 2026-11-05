package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Test

class VolumeRestoreTest {
    /** Android 17 ignores volume calls from an app with no visible window and no foreground service. */
    @Test fun restoreOutsideAServiceIsDeferredOnAndroid17() =
        assertEquals(VolumeAction.Defer, volumeRestoreAction(sdkInt = 37, inForegroundService = false, pending = true))

    @Test fun olderAndroidRestoresAtOnce() =
        assertEquals(VolumeAction.RestoreNow, volumeRestoreAction(sdkInt = 36, inForegroundService = false, pending = true))

    @Test fun pendingRestoreRunsWhenTheServiceStarts() =
        assertEquals(VolumeAction.RestoreNow, volumeRestoreAction(sdkInt = 37, inForegroundService = true, pending = true))

    @Test fun nothingPendingDoesNothing() =
        assertEquals(VolumeAction.Nothing, volumeRestoreAction(sdkInt = 37, inForegroundService = true, pending = false))
}
