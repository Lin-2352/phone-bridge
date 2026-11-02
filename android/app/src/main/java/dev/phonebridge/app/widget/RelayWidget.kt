package dev.phonebridge.app.widget

import android.content.Context
import android.content.Intent
import android.util.Log
import androidx.compose.runtime.Composable
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.glance.GlanceId
import androidx.glance.GlanceModifier
import androidx.glance.GlanceTheme
import androidx.glance.action.clickable
import androidx.glance.appwidget.GlanceAppWidget
import androidx.glance.appwidget.action.actionStartActivity
import androidx.glance.appwidget.cornerRadius
import androidx.glance.appwidget.provideContent
import androidx.glance.appwidget.updateAll
import androidx.glance.background
import androidx.glance.layout.Alignment
import androidx.glance.layout.Box
import androidx.glance.layout.Column
import androidx.glance.layout.Row
import androidx.glance.layout.Spacer
import androidx.glance.layout.fillMaxSize
import androidx.glance.layout.fillMaxWidth
import androidx.glance.layout.height
import androidx.glance.layout.padding
import androidx.glance.layout.width
import androidx.glance.material3.ColorProviders
import androidx.glance.text.FontWeight
import androidx.glance.text.Text
import androidx.glance.text.TextStyle
import dev.phonebridge.app.Bridge
import dev.phonebridge.app.CameraShare
import dev.phonebridge.app.MainActivity
import dev.phonebridge.app.ScreenShare
import dev.phonebridge.app.SendClipboardActivity
import dev.phonebridge.app.ui.relayDarkScheme
import dev.phonebridge.app.ui.relayLightScheme
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import java.util.concurrent.atomic.AtomicBoolean

private const val TAG = "PhoneBridge"

/** The Relay tokens for the widget; light or dark follows the system. */
private val widgetColors by lazy { ColorProviders(light = relayLightScheme(), dark = relayDarkScheme()) }

/**
 * The 2 x 1 home-screen widget: the connection state (tap opens the app) over "Send clipboard" and
 * "Send file". State is read from [Bridge] at render time (never the node), so a process without the
 * service renders "Relay is off".
 */
class RelayWidget : GlanceAppWidget() {
    override suspend fun provideGlance(context: Context, id: GlanceId) {
        provideContent { GlanceTheme(colors = widgetColors) { Content(context) } }
    }

    @Composable
    private fun Content(ctx: Context) {
        // The node up, not only the service: until then the PC name is unknown (it would read "Not paired").
        val running = Bridge.running.value && Bridge.nodeReady.value
        val pc = Bridge.linked.value
        val connected = widgetConnected(
            running, pc, Bridge.liveSessions.intValue, Bridge.controlledBy.value,
            ScreenShare.sharingWith.value, CameraShare.usedBy.value,
        )
        Column(
            GlanceModifier.fillMaxSize().background(GlanceTheme.colors.surface).cornerRadius(16.dp).padding(8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                widgetLabel(running, pc, connected),
                GlanceModifier.fillMaxWidth().padding(horizontal = 4.dp)
                    .clickable(actionStartActivity(Intent(ctx, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))),
                style = TextStyle(
                    color = if (connected) GlanceTheme.colors.primary else GlanceTheme.colors.onSurface,
                    fontSize = 13.sp,
                    fontWeight = FontWeight.Medium,
                ),
                maxLines = 1,
            )
            Spacer(GlanceModifier.height(6.dp))
            Row(GlanceModifier.fillMaxWidth()) {
                // Started exactly as the Quick Settings tile does: the activity reads the clipboard once focused.
                WidgetButton(
                    "Send clipboard",
                    Intent(ctx, SendClipboardActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                    GlanceModifier.defaultWeight(),
                )
                Spacer(GlanceModifier.width(6.dp))
                WidgetButton(
                    "Send file",
                    Intent(ctx, PickFileActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                    GlanceModifier.defaultWeight(),
                )
            }
        }
    }

    @Composable
    private fun WidgetButton(label: String, intent: Intent, modifier: GlanceModifier) {
        Box(
            modifier.height(36.dp).background(GlanceTheme.colors.primaryContainer).cornerRadius(18.dp)
                .clickable(actionStartActivity(intent)),
            contentAlignment = Alignment.Center,
        ) {
            Text(
                label,
                style = TextStyle(color = GlanceTheme.colors.onPrimaryContainer, fontSize = 12.sp, fontWeight = FontWeight.Medium),
                maxLines = 1,
            )
        }
    }

    companion object {
        private val pending = AtomicBoolean(false)

        /**
         * Redraws every placed widget after a connection state change. A burst (a start sets running,
         * the node and the PC name) coalesces into one redraw, off the main thread, on the process-wide
         * scope so the "off" redraw from the service's onDestroy still runs.
         */
        fun update(ctx: Context) {
            val app = ctx.applicationContext
            if (!pending.compareAndSet(false, true)) return
            Bridge.scope.launch {
                delay(300)
                // Cleared before the redraw reads the state: a change from now on schedules another one.
                pending.set(false)
                runCatching { RelayWidget().updateAll(app) }.onFailure { Log.e(TAG, "WIDGET update failed: $it") }
            }
        }
    }
}
