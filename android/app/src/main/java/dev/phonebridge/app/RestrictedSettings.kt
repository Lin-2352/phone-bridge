package dev.phonebridge.app

import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.net.Uri
import android.os.Build
import android.provider.Settings

/**
 * Android 13+ greys out Accessibility and Notification access ("Restricted setting") for apps
 * installed from a browser or a file manager, until the user allows it in App info (A3). Store and
 * adb installs are not restricted.
 */
fun needsRestrictedSettingsHelp(sdkInt: Int, packageSource: Int?, granted: Boolean): Boolean =
    sdkInt >= 33 && !granted && (
        packageSource == PackageInstaller.PACKAGE_SOURCE_LOCAL_FILE ||
            packageSource == PackageInstaller.PACKAGE_SOURCE_DOWNLOADED_FILE
        )

/** Android 17 with Advanced Protection removes accessibility from apps that are not accessibility tools (A10). */
fun advancedProtectionBlocksControl(sdkInt: Int, apOn: Boolean?): Boolean = sdkInt >= 37 && apOn == true

fun packageSource(ctx: Context): Int? =
    if (Build.VERSION.SDK_INT >= 33) {
        runCatching { ctx.packageManager.getInstallSourceInfo(ctx.packageName).packageSource }.getOrNull()
    } else {
        null
    }

/** Unknown (null) when Android has no such setting or does not tell. */
fun advancedProtectionOn(ctx: Context): Boolean? =
    if (Build.VERSION.SDK_INT >= 36) {
        runCatching {
            ctx.getSystemService(android.security.advancedprotection.AdvancedProtectionManager::class.java)
                ?.isAdvancedProtectionEnabled
        }.getOrNull()
    } else {
        null
    }

/** App info, where ⋮ → "Allow restricted settings" lives. */
fun openAppInfo(ctx: Context) {
    runCatching {
        ctx.startActivity(
            Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", ctx.packageName, null))
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
        )
    }
}

const val RESTRICTED_HELP = "Installed from a file: if Android says “Restricted setting”, tap here for App info, " +
    "choose ⋮ › Allow restricted settings, then come back"
