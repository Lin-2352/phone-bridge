package dev.phonebridge.app.ui.settings

import dev.phonebridge.app.Group
import dev.phonebridge.app.SettingsNav

/** [SettingsNav] as saveable strings (page name or "", then the query), so rotation and a tab switch keep the place. */
fun SettingsNav.toSaved(): List<String> = listOf(page?.name.orEmpty(), query)

fun settingsNavFromSaved(l: List<String>): SettingsNav =
    SettingsNav(Group.entries.firstOrNull { it.name == l.getOrNull(0) }, l.getOrNull(1).orEmpty())

fun pcCount(n: Int): String = if (n == 1) "1 PC" else "$n PCs"

/** What to do after the picture-access prompt: nothing, explain, or send the user to Android's settings (it will not ask again). */
enum class PhotoDenial { None, Explain, OpenSettings }

fun photoDenial(granted: Boolean, canAskAgain: Boolean): PhotoDenial = when {
    granted -> PhotoDenial.None
    canAskAgain -> PhotoDenial.Explain
    else -> PhotoDenial.OpenSettings
}
