package dev.phonebridge.app.ui

/** Where the app can go to. Tabs are four of them; Pair is the scan screen. */
enum class Destination(val tab: Tab?) {
    Home(Tab.Home), Timeline(Tab.Timeline), Remote(Tab.Remote), Settings(Tab.Settings), Pair(null);
}

/** One control that leads to [dest], on screen [where]. A non-primary one must say [reason] it exists and word its label differently. */
data class EntryPoint(val dest: Destination, val where: String, val label: String, val primary: Boolean, val reason: String = "")

/** Every control that navigates, kept here so a duplicate is a visible decision and a test failure otherwise. */
val entryPoints: List<EntryPoint> = listOf(
    EntryPoint(Destination.Home, "Bottom bar", "Home", true),
    EntryPoint(Destination.Timeline, "Bottom bar", "Timeline", true),
    EntryPoint(Destination.Timeline, "Home, Recent", "See all", false, "Continues the three recent items it sits under"),
    EntryPoint(Destination.Remote, "Bottom bar", "Remote", true),
    EntryPoint(Destination.Remote, "Home, quick actions", "Touchpad", false, "Opens the full-screen touchpad directly instead of the tab"),
    EntryPoint(Destination.Settings, "Bottom bar", "Settings", true),
    EntryPoint(Destination.Pair, "Home, hero card", "Pair", true),
    EntryPoint(Destination.Pair, "Home, quick actions (only while a PC is paired)", "Pair another PC", false, "Hero button is gone once a PC is paired"),
    EntryPoint(Destination.Pair, "Remote, when unpaired", "Scan code", false, "The tab cannot work without a PC; other screens' buttons are not in view"),
    EntryPoint(Destination.Pair, "Settings, Connection", "Pair a new PC", false, "Where people look to add a PC"),
)
