package dev.phonebridge.app.ui

import android.content.Intent
import androidx.compose.animation.fadeIn
import dev.phonebridge.app.ui.components.LocalModes
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.History
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.TouchApp
import androidx.compose.material3.Icon
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.navigation.NavDestination.Companion.hierarchy
import androidx.navigation.NavGraph.Companion.findStartDestination
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
import dev.phonebridge.app.ui.home.HomeTab
import dev.phonebridge.app.ui.remote.RemoteTab
import dev.phonebridge.app.ui.settings.SettingsTab
import dev.phonebridge.app.ui.timeline.TimelineTab

/** The four top-level destinations, in bar order. */
enum class Tab(val route: String, val label: String) {
    Home("home", "Home"),
    Timeline("timeline", "Timeline"),
    Remote("remote", "Remote"),
    Settings("settings", "Settings"),
}

/**
 * The tab a launch opens on: the `tab` extra's route (`am start ... --es tab timeline`), else Home.
 * A test hook like MainActivity's others: MainActivity is exported, so only debuggable builds honour it.
 */
fun startTab(route: String?, debuggable: Boolean): Tab =
    if (!debuggable) Tab.Home else Tab.entries.firstOrNull { it.route == route } ?: Tab.Home

fun startTab(intent: Intent?, debuggable: Boolean): Tab = startTab(intent?.getStringExtra(TAB_EXTRA), debuggable)

const val TAB_EXTRA = "tab"

/** Bumped by MainActivity on every resume: rows showing system settings re-read them. */
val LocalResumeCount = compositionLocalOf { 0 }

private fun Tab.icon(): ImageVector = when (this) {
    Tab.Home -> Icons.Filled.Home
    Tab.Timeline -> Icons.Filled.History
    Tab.Remote -> Icons.Filled.TouchApp
    Tab.Settings -> Icons.Filled.Settings
}

/**
 * The app shell: the bottom navigation bar over the four tabs. Home is the start destination, so
 * Back from another tab returns to Home and Back on Home leaves (the standard top-level pattern).
 * The selected tab lives in the nav controller's saved state, which survives rotation and process death.
 *
 * [start] is the tab requested by the launch; [startKey] changes when a new intent asks again
 * (onNewIntent). Each key is honoured once, so a rotation does not jump back to [start].
 */
@Composable
fun RelayNavHost(start: Tab, startKey: Int = 0) {
    val nav = rememberNavController()
    val entry by nav.currentBackStackEntryAsState()
    val current = Tab.entries.firstOrNull { t -> entry?.destination?.hierarchy?.any { it.route == t.route } == true } ?: Tab.Home
    val go: (Tab) -> Unit = { t ->
        nav.navigate(t.route) {
            popUpTo(nav.graph.findStartDestination().id) { saveState = true }
            launchSingleTop = true
            restoreState = true
        }
    }
    Scaffold(
        // The tabs' own Scaffolds handle the status bar and their top app bars.
        contentWindowInsets = WindowInsets(0),
        bottomBar = {
            NavigationBar {
                Tab.entries.forEach { t ->
                    NavigationBarItem(
                        selected = current == t,
                        onClick = { go(t) },
                        icon = { Icon(t.icon(), null) },
                        label = { Text(t.label) },
                    )
                }
            }
        },
    ) { pad ->
        val modes = LocalModes.current
        NavHost(
            nav,
            startDestination = Tab.Home.route,
            // The bar's height (with the gesture inset) is consumed, so the tabs do not pad for it twice.
            modifier = Modifier.padding(pad).consumeWindowInsets(pad),
            enterTransition = { fadeIn(tokenSpec(Tokens.Springs.Standard, modes)) },
            exitTransition = { fadeOut(tokenSpec(Tokens.Springs.Standard, modes)) },
        ) {
            composable(Tab.Home.route) { HomeTab(onTimeline = { go(Tab.Timeline) }) }
            composable(Tab.Timeline.route) { TimelineTab() }
            composable(Tab.Remote.route) { RemoteTab() }
            composable(Tab.Settings.route) { SettingsTab(onDone = { go(Tab.Home) }) }
        }
        // After the NavHost so its graph is set when this runs.
        var handled by rememberSaveable { mutableIntStateOf(-1) }
        LaunchedEffect(startKey) {
            if (handled != startKey) {
                handled = startKey
                if (start != current) go(start)
            }
        }
    }
}
