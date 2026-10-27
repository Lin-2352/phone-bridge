package dev.phonebridge.app

/** The phone's settings groups; declaration order is display order. */
enum class Group { Connection, Camera, Screen, Notifications, Files, Settings }

/** Which group a settings section (by its title) is shown under; unknown sections land in Settings. */
fun groupFor(section: String): Group = when (section) {
    "Devices" -> Group.Connection
    "Camera" -> Group.Camera
    "Control from PC" -> Group.Screen
    "Notifications" -> Group.Notifications
    "Sharing" -> Group.Files
    else -> Group.Settings
}

fun groupTitle(g: Group): String = g.name

/** One setting as search sees it: the page it lives on, its title and its one-line description. */
data class SettingEntry(val group: Group, val title: String, val subtitle: String)

/** Every individual setting, in page order; mirrors the rows on the pages. */
val settingEntries: List<SettingEntry> = listOf(
    SettingEntry(Group.Connection, "Paired PCs", "Remove a PC you no longer use"),
    SettingEntry(Group.Connection, "Pair a new PC", "Scan the QR code shown on your PC"),
    SettingEntry(Group.Camera, "PCs always allowed to use the camera", "Skip the question; Android's camera indicator still shows"),
    SettingEntry(Group.Screen, "Let my PC control this phone", "Use the PC's mouse and keyboard here"),
    SettingEntry(Group.Notifications, "Notifications on PC", "Show this phone's notifications on your PC"),
    SettingEntry(Group.Screen, "Instant screen", "Skip Android's prompt after the first share"),
    SettingEntry(Group.Screen, "Show my screen on my PC without asking", "Skip the question before your screen is shared"),
    SettingEntry(Group.Screen, "Accessibility permission", "$APP_NAME control lets your PC tap and type here"),
    SettingEntry(Group.Files, "Accept from my devices automatically", "Only paired devices can send"),
    SettingEntry(Group.Files, "Save files to", "Choose the folder for received files"),
    SettingEntry(Group.Settings, "Theme", "System, Light or Dark"),
    SettingEntry(Group.Settings, "Share clipboard", "Share what you copy with your paired PC"),
    SettingEntry(Group.Settings, "Passwords and codes", "Whether text marked sensitive is sent"),
    SettingEntry(Group.Settings, "Share copied images", "Put pictures copied on your PC on this phone's clipboard"),
    SettingEntry(Group.Settings, "Send new screenshots to PC", "Each screenshot you take is copied to your PC's clipboard"),
    SettingEntry(Group.Settings, APP_NAME, "Turn it on or off; off means your PC cannot reach this phone"),
    SettingEntry(Group.Settings, "Keep running in background", "Let Android bring $APP_NAME back when memory is low"),
    SettingEntry(Group.Settings, "Hide the status notification", "Open the notification's Android settings"),
)

/** Settings whose title or subtitle contains [query] (case-insensitive); a blank query matches nothing. */
fun searchSettings(query: String, entries: List<SettingEntry> = settingEntries): List<SettingEntry> {
    val q = query.trim()
    if (q.isEmpty()) return emptyList()
    return entries.filter { it.title.contains(q, ignoreCase = true) || it.subtitle.contains(q, ignoreCase = true) }
}

/** Whether the home screen should warn that phone notifications cannot reach the PC (switch on, Android access missing). */
fun notifyAccessMissing(notifyToPc: Boolean, access: Boolean): Boolean = notifyToPc && !access

/** The one-line description under a group's entry in the settings list. */
fun pageSummary(g: Group): String = when (g) {
    Group.Connection -> "Paired PCs and pairing a new one"
    Group.Camera -> "Which PCs may use this phone's camera"
    Group.Screen -> "Control and screen sharing with your PC"
    Group.Notifications -> "Show this phone's notifications on your PC"
    Group.Files -> "Accepting files and where they are saved"
    Group.Settings -> "Theme, clipboard and running in the background"
}

/**
 * Where the settings screen is: the group list ([page] null, optionally filtered by [query]) or one group's page.
 * [back] gives the previous place, or null when Back should leave settings.
 */
data class SettingsNav(val page: Group? = null, val query: String = "") {
    fun open(g: Group) = copy(page = g)
    fun back(): SettingsNav? = when {
        page != null -> copy(page = null)
        query.isNotEmpty() -> copy(query = "")
        else -> null
    }
}
