package dev.phonebridge.app.widget

import android.content.Context
import android.content.Intent
import androidx.glance.appwidget.GlanceAppWidget
import androidx.glance.appwidget.GlanceAppWidgetReceiver

/** The home-screen widget's entry point for the launcher. */
class RelayWidgetReceiver : GlanceAppWidgetReceiver() {
    override val glanceAppWidget: GlanceAppWidget = RelayWidget()

    override fun onReceive(context: Context, intent: Intent) {
        // The staleness alarm (RelayWidget.scheduleStale): redraw from this process's real state.
        if (intent.action == RelayWidget.ACTION_STALE) RelayWidget.update(context) else super.onReceive(context, intent)
    }
}
