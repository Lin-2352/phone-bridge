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

/** One app in the "Apps on your PC" list; [installed] is false for a package that has since been removed (its label is the package name). */
data class AppRow(val pkg: String, val label: String, val installed: Boolean)

/** Installed apps first, each group by name; removed apps sink to the end so they never look like real ones. */
fun orderAppRows(rows: List<AppRow>): List<AppRow> =
    rows.sortedWith(compareBy<AppRow>({ !it.installed }, { it.label.lowercase() }))

/** The rows whose name or package contains [query]; a blank query keeps all. */
fun filterAppRows(rows: List<AppRow>, query: String): List<AppRow> {
    val q = query.trim()
    return if (q.isEmpty()) rows else rows.filter { it.label.contains(q, ignoreCase = true) || it.pkg.contains(q, ignoreCase = true) }
}
