package dev.phonebridge.app.ui

/** The phone makers whose battery settings stop Relay from running in the background. */
enum class Oem { Xiaomi, Samsung, Huawei, OppoRealme, Other }

/**
 * The maker from the phone's manufacturer and brand (both are checked; a brand such as POCO or Honor counts
 * for its owner, and OnePlus is OxygenOS, which shares the Oppo/realme settings).
 */
fun oemOf(manufacturer: String, brand: String): Oem {
    val text = "${manufacturer.lowercase()} ${brand.lowercase()}"
    return when {
        listOf("xiaomi", "redmi", "poco").any { text.contains(it) } -> Oem.Xiaomi
        text.contains("samsung") -> Oem.Samsung
        listOf("huawei", "honor").any { text.contains(it) } -> Oem.Huawei
        listOf("oppo", "realme", "oneplus").any { text.contains(it) } -> Oem.OppoRealme
        else -> Oem.Other
    }
}

/**
 * What to tell the person, and the settings screens to open for them. Relay only opens the screens; it changes
 * nothing. Intents are `package/class` names, tried in order; with none, the app's own settings page opens.
 */
data class OemGuide(val steps: List<String>, val intents: List<String>)

fun oemGuide(oem: Oem): OemGuide = when (oem) {
    Oem.Xiaomi -> OemGuide(
        steps = listOf(
            "Open Autostart and turn on Relay.",
            "In Battery saver, choose No restrictions for Relay.",
        ),
        intents = listOf("com.miui.securitycenter/com.miui.permcenter.autostart.AutoStartManagementActivity"),
    )
    Oem.Samsung -> OemGuide(
        steps = listOf(
            "Open Battery and set Relay to Unrestricted.",
            "Keep Relay out of Sleeping apps.",
        ),
        intents = listOf("com.samsung.android.lool/com.samsung.android.sm.ui.battery.BatteryActivity"),
    )
    Oem.Huawei -> OemGuide(
        steps = listOf("Open App launch and set Relay to Manage manually, then turn on Auto-launch."),
        intents = listOf("com.huawei.systemmanager/.startupmgr.ui.StartupNormalAppListActivity"),
    )
    Oem.OppoRealme -> OemGuide(
        steps = listOf("Open Startup manager and allow Relay to start automatically and run in the background."),
        intents = listOf("com.coloros.safecenter/.permission.startup.StartupAppListActivity"),
    )
    Oem.Other -> OemGuide(
        steps = listOf(
            "Open Battery settings for Relay and choose Unrestricted or Don't optimise.",
        ),
        intents = emptyList(),
    )
}

/**
 * Opens the first maker settings screen this phone has (checked with resolveActivity), or the app's own
 * settings page. It only opens a settings screen for the person; nothing is changed by Relay.
 */
fun openMakerSettings(ctx: android.content.Context) {
    val guide = oemGuide(oemOf(android.os.Build.MANUFACTURER, android.os.Build.BRAND))
    for (name in guide.intents) {
        val (pkg, cls) = name.split("/", limit = 2)
        val intent = android.content.Intent()
            .setComponent(android.content.ComponentName(pkg, if (cls.startsWith(".")) pkg + cls else cls))
            .addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK)
        if (intent.resolveActivity(ctx.packageManager) != null) {
            ctx.startActivity(intent)
            return
        }
    }
    ctx.startActivity(
        android.content.Intent(android.provider.Settings.ACTION_APPLICATION_DETAILS_SETTINGS)
            .setData(android.net.Uri.parse("package:${ctx.packageName}"))
            .addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK),
    )
}
