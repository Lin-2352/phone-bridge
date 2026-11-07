package dev.phonebridge.app

/** The phone's settings groups; declaration order is display order. */
enum class Group { Connection, Camera, Screen, Audio, Notifications, Files, Settings }

/** Which group a settings section (by its title) is shown under; unknown sections land in Settings. */
fun groupFor(section: String): Group = when (section) {
    "Devices" -> Group.Connection
    "Camera" -> Group.Camera
    "Control from PC" -> Group.Screen
    "Audio" -> Group.Audio
    "Notifications" -> Group.Notifications
    "Sharing" -> Group.Files
    else -> Group.Settings
}

/** The page title; the last group is shown as "General" (its enum name stays [Group.Settings]). */
fun groupTitle(g: Group): String = if (g == Group.Settings) "General" else g.name

/** One setting as search sees it: the page it lives on, its title and its one-line description. */
data class SettingEntry(val group: Group, val title: String, val subtitle: String)

/** Every individual setting, in page order; mirrors the rows on the pages. */
val settingEntries: List<SettingEntry> = listOf(
    SettingEntry(Group.Settings, "Look", "Native glass or Relay glass"),
    SettingEntry(Group.Settings, "Reduce transparency", "Solid panels instead of see-through ones"),
    SettingEntry(Group.Settings, "Lite mode", "No blur and no idle animation; turns on by itself when the phone needs it"),
    SettingEntry(Group.Connection, "Paired PCs", "Remove a PC you no longer use"),
    SettingEntry(Group.Connection, "Pair a new PC", "Scan the QR code shown on your PC"),
    SettingEntry(Group.Camera, "PCs always allowed to use the camera", "Skip the question; Android's camera indicator still shows"),
    SettingEntry(Group.Screen, "Let my PC control this phone", "Use the PC's mouse and keyboard here; needs Accessibility permission"),
    SettingEntry(Group.Screen, "Keep Control on between sessions", "Don't switch Relay control off after each session"),
    SettingEntry(Group.Notifications, "Notifications on PC", "Show this phone's notifications on your PC"),
    SettingEntry(Group.Notifications, "Apps on your PC", "Choose which apps may show notifications on your PC; banking apps are blocked by default"),
    SettingEntry(Group.Screen, "Instant screen", "Skip Android's prompt after the first share"),
    SettingEntry(Group.Screen, "Show my screen on my PC without asking", "Skip the question before your screen is shared"),
    SettingEntry(Group.Files, "Accept from my devices automatically", "Only paired devices can send"),
    SettingEntry(Group.Files, "Save files to", "Choose the folder for received files"),
    SettingEntry(Group.Audio, "Silence this phone while streaming", "Its speaker stays quiet while its audio plays on your PC"),
    SettingEntry(Group.Audio, "Silence earphones too", "Connected earphones stay silent as well"),
    SettingEntry(Group.Audio, "Volume keys in every app", "Set the PC's level in YouTube and Instagram too, through $APP_NAME control"),
    SettingEntry(Group.Audio, "Share this phone's audio without asking", "Skips only $APP_NAME's question; Android's capture prompt and the microphone question always appear"),
    SettingEntry(Group.Settings, "Theme", "System, Light or Dark"),
    SettingEntry(Group.Settings, "Share clipboard", "Share what you copy with your paired PC"),
    SettingEntry(Group.Settings, "Passwords and codes", "Whether text marked sensitive is sent"),
    SettingEntry(Group.Settings, "Share copied images", "Put pictures copied on your PC on this phone's clipboard"),
    SettingEntry(Group.Settings, "Send new screenshots to PC", "Each screenshot you take is copied to your PC's clipboard"),
    SettingEntry(Group.Settings, APP_NAME, "Turn it on or off; off means your PC cannot reach this phone"),
    SettingEntry(Group.Settings, "Keep running in background", "Let Android bring $APP_NAME back when memory is low"),
    SettingEntry(Group.Settings, "Keep logs", "Technical notes about connections and transfers, never your content"),
    SettingEntry(Group.Settings, "Log size", "How much space the daily logs use"),
    SettingEntry(Group.Settings, "Export logs", "Share a zip of the daily logs"),
    SettingEntry(Group.Settings, "Clear all logs", "Delete every daily log on this phone"),
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
    Group.Audio -> "Sharing this phone's audio and playing your PC's"
    Group.Notifications -> "Show this phone's notifications on your PC"
    Group.Files -> "Accepting files and where they are saved"
    Group.Settings -> "Theme, clipboard, background and logs"
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

/** What the live one-line status of each group is built from; plain values so it is testable. */
data class StatusInput(
    val pairedNames: List<String> = emptyList(),
    val cameraAllowedCount: Int = 0,
    val controlAllowed: Boolean = false,
    val controlServiceOn: Boolean = false,
    val audioSkipAsk: Boolean = false,
    val notifyOn: Boolean = false,
    val notifyAccess: Boolean = false,
    val saveLabel: String = "",
    val themeName: String = "System",
    val keepLogs: Boolean = true,
    val logSize: String = "0.0 MB",
)

/** The live one-line status shown under a group's entry; falls back to nothing blank. */
fun statusLine(g: Group, s: StatusInput): String = when (g) {
    Group.Connection -> when (s.pairedNames.size) {
        0 -> "No PC paired yet"
        1 -> "Paired with ${s.pairedNames[0]}"
        else -> "Paired with ${s.pairedNames[0]} and ${s.pairedNames.size - 1} more"
    }
    Group.Camera -> if (s.cameraAllowedCount == 0) "Ask each time" else "${s.cameraAllowedCount} ${if (s.cameraAllowedCount == 1) "PC" else "PCs"} always allowed"
    Group.Screen -> when {
        !s.controlServiceOn -> "Control needs accessibility permission"
        s.controlAllowed -> "PC control allowed"
        else -> "PC control asks first"
    }
    Group.Audio -> if (s.audioSkipAsk) "Phone audio starts without $APP_NAME's question" else "Phone audio asks first"
    Group.Notifications -> when {
        !s.notifyOn -> "Off"
        !s.notifyAccess -> "On, needs notification access"
        else -> "On, showing on your PC"
    }
    Group.Files -> "Saving to ${s.saveLabel}"
    Group.Settings -> "Theme ${s.themeName.lowercase()}, " + if (s.keepLogs) "logs ${s.logSize}" else "logs off"
}
