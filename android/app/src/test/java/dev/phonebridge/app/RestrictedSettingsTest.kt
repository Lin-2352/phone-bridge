package dev.phonebridge.app

import android.content.pm.PackageInstaller
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class RestrictedSettingsTest {
    @Test fun sideloadedOn13WithoutGrantNeedsHelp() {
        assertTrue(needsRestrictedSettingsHelp(33, PackageInstaller.PACKAGE_SOURCE_DOWNLOADED_FILE, granted = false))
        assertTrue(needsRestrictedSettingsHelp(35, PackageInstaller.PACKAGE_SOURCE_LOCAL_FILE, granted = false))
    }

    @Test fun storeOrAdbInstallNeedsNoHelp() {
        assertFalse(needsRestrictedSettingsHelp(35, PackageInstaller.PACKAGE_SOURCE_STORE, granted = false))
        assertFalse(needsRestrictedSettingsHelp(35, PackageInstaller.PACKAGE_SOURCE_UNSPECIFIED, granted = false))
        assertFalse(needsRestrictedSettingsHelp(35, null, granted = false))
    }

    @Test fun below13OrGrantedNeedsNoHelp() {
        assertFalse(needsRestrictedSettingsHelp(32, PackageInstaller.PACKAGE_SOURCE_DOWNLOADED_FILE, granted = false))
        assertFalse(needsRestrictedSettingsHelp(35, PackageInstaller.PACKAGE_SOURCE_DOWNLOADED_FILE, granted = true))
    }

    @Test fun advancedProtectionBlocksOnlyWhenKnownOnAndroid17() {
        assertTrue(advancedProtectionBlocksControl(37, apOn = true))
        assertFalse(advancedProtectionBlocksControl(37, apOn = null))
        assertFalse(advancedProtectionBlocksControl(37, apOn = false))
        assertFalse(advancedProtectionBlocksControl(36, apOn = true))
    }
}
