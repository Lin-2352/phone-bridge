package dev.phonebridge.app

/** Ask only after an explicit enable, never on disabling or merely opening a panel. */
fun needsClipboardOverlay(enabled: Boolean, granted: Boolean): Boolean = enabled && !granted

const val NEW_PC_CLIPBOARD_NOTICE = "Share clipboard starts off for a new PC. Turn it on in this PC's settings on the phone and in Relay on the PC."

/** Opens the existing Android permission page; only the owner can grant permission. */
fun requestClipboardOverlay(ctx: android.content.Context, enabled: Boolean) {
    if (!needsClipboardOverlay(enabled, android.provider.Settings.canDrawOverlays(ctx))) return
    android.widget.Toast.makeText(ctx, "Allow Display over other apps so your PC can request this phone's clipboard", android.widget.Toast.LENGTH_LONG).show()
    val intent = android.content.Intent(
        android.provider.Settings.ACTION_MANAGE_OVERLAY_PERMISSION,
        android.net.Uri.parse("package:${ctx.packageName}"),
    ).addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK)
    if (runCatching { ctx.startActivity(intent) }.isFailure) {
        notifyInfo(ctx, "Allow clipboard requests", "Open Android Settings, Apps, $APP_NAME, Display over other apps")
    }
}
