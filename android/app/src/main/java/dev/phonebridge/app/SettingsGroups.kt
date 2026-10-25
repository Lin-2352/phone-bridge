package dev.phonebridge.app

/** The phone's settings groups; declaration order is display order. */
enum class Group { Connection, Camera, Screen, Files, Settings }

/** Which group a settings section (by its title) is shown under; unknown sections land in Settings. */
fun groupFor(section: String): Group = when (section) {
    "Devices" -> Group.Connection
    "Camera" -> Group.Camera
    "Control from PC" -> Group.Screen
    "Sharing" -> Group.Files
    else -> Group.Settings
}

fun groupTitle(g: Group): String = g.name
