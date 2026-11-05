package dev.phonebridge.app.ui.settings

import android.content.Context
import android.content.Intent
import android.os.Build
import android.provider.Settings
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import dev.phonebridge.app.APP_NAME
import dev.phonebridge.app.Prefs
import dev.phonebridge.app.RelayDialog
import dev.phonebridge.app.controlSettingsIntent
import dev.phonebridge.app.ui.home.openAppDetails
import dev.phonebridge.app.ui.home.openSettings

private const val PLAY_STORE = "com.android.vending"

/** The two Relay features behind a switch that Android restricts for apps not installed from a store. */
enum class SpecialAccess(val page: String, val entry: String) {
    Notifications("Notification access", APP_NAME),
    Control("Accessibility", "$APP_NAME control"),
}

/**
 * From Android 13, an installed APK can find these switches greyed out. Android doesn't give apps a
 * supported way to ask, so this only says it may happen: the steps are harmless if the switch works.
 */
internal fun restrictedAccessLikely(sdk: Int, installer: String?): Boolean = sdk >= 33 && installer != PLAY_STORE

/**
 * A service that has connected proves the lock is gone. It has to be remembered, because control
 * switches itself off after each session, so being off now doesn't mean it's locked.
 */
internal fun showRestrictedSteps(sdk: Int, installer: String?, serviceEverConnected: Boolean): Boolean =
    !serviceEverConnected && restrictedAccessLikely(sdk, installer)

internal fun restrictedAccessSteps(access: SpecialAccess): List<String> = listOf(
    "Open ${access.page} and try to turn on ${access.entry}. If Android says access was denied, close that message.",
    "Open App info, tap the ⋮ menu, then Allow restricted settings, and confirm.",
    "Go back to ${access.page} and turn on ${access.entry}.",
)

private fun installerOf(ctx: Context): String? =
    if (Build.VERSION.SDK_INT >= 30) {
        runCatching { ctx.packageManager.getInstallSourceInfo(ctx.packageName).installingPackageName }.getOrNull()
    } else {
        null
    }

private fun open(ctx: Context, access: SpecialAccess) = when (access) {
    SpecialAccess.Notifications -> openNotifyAccess(ctx)
    SpecialAccess.Control -> openSettings(ctx, controlSettingsIntent(ctx))
}

/** Returns what a Grant or Fix button calls: straight to the settings page, or through the steps first. */
@Composable
fun rememberAccessOpener(): (SpecialAccess) -> Unit {
    val ctx = LocalContext.current
    val installer = remember { installerOf(ctx) }
    var showing by rememberSaveable { mutableStateOf<SpecialAccess?>(null) }
    showing?.let { access ->
        RelayDialog(
            title = "Android may block this switch",
            onDismissRequest = { showing = null },
            confirmText = "Open ${access.page}",
            onConfirm = {
                showing = null
                open(ctx, access)
            },
            dismissText = "App info",
            onDismiss = {
                showing = null
                openAppDetails(ctx)
            },
        ) {
            restrictedAccessSteps(access).forEachIndexed { i, step ->
                Text("${i + 1}. $step", style = MaterialTheme.typography.bodyMedium)
            }
        }
    }
    return { access ->
        // Read on each tap: the service may have connected since this screen was composed.
        if (showRestrictedSteps(Build.VERSION.SDK_INT, installer, Prefs(ctx).specialAccessConnected)) showing = access else open(ctx, access)
    }
}
