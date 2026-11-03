package dev.phonebridge.app.widget

import android.content.Context
import android.content.Intent
import android.util.Log
import androidx.compose.runtime.Composable
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.glance.GlanceId
import androidx.glance.GlanceModifier
import androidx.glance.GlanceTheme
import androidx.glance.LocalSize
import androidx.glance.action.clickable
import androidx.glance.appwidget.GlanceAppWidget
import androidx.glance.appwidget.SizeMode
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
import dev.phonebridge.app.AudioShare
import dev.phonebridge.app.ScreenShare
import dev.phonebridge.app.SendClipboardActivity
import dev.phonebridge.app.relayStatus
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
    // A one-row cell can be as low as the 40 dp minimum: a compact variant fits it; taller cells get room.
    override val sizeMode = SizeMode.Responsive(setOf(COMPACT, ROOMY))

    override suspend fun provideGlance(context: Context, id: GlanceId) {
        scheduleStale(context, Bridge.isConnected())
        provideContent { GlanceTheme(colors = widgetColors) { Content(context) } }
    }

    @Composable
    private fun Content(ctx: Context) {
        // The same answer as the status notification, tile and Home (relayStatus): "Starting" until the node is up.
        val pc = Bridge.linked.value
        val connected = widgetConnected(
            Bridge.running.value, pc, Bridge.liveSessions.intValue, Bridge.controlledBy.value,
            ScreenShare.sharingWith.value, CameraShare.usedBy.value, AudioShare.activePeer(),
        )
        val now = System.currentTimeMillis()
        val label = relayStatus(Bridge.running.value, Bridge.nodeReady.value, pc, connected, Bridge.lastSeenAny(ctx), now).line
        // "As of": a Connected drawn by a process that was killed later cannot be told from a live one otherwise.
        val asOf = if (connected) java.text.DateFormat.getTimeInstance(java.text.DateFormat.SHORT).format(java.util.Date(now)) else null
        // Compact: 2 + 14 + 2 + 22 = 40 dp; roomy: 4 + 17 + 4 + 30 + 4 = 59 dp.
        val compact = LocalSize.current.height < ROOMY.height
        val pad = if (compact) 2.dp else 4.dp
        Column(
            GlanceModifier.fillMaxSize().background(GlanceTheme.colors.surface).cornerRadius(16.dp)
                .padding(horizontal = 6.dp, vertical = pad),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                if (asOf != null && !compact) "$label \u00b7 $asOf" else label,
                GlanceModifier.fillMaxWidth().padding(horizontal = 4.dp)
                    .clickable(actionStartActivity(Intent(ctx, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))),
                style = TextStyle(
                    color = if (connected) GlanceTheme.colors.primary else GlanceTheme.colors.onSurface,
                    fontSize = if (compact) 11.sp else 13.sp,
                    fontWeight = FontWeight.Medium,
                ),
                maxLines = 1,
            )
            Spacer(GlanceModifier.height(pad))
            Row(GlanceModifier.fillMaxWidth()) {
                // Started exactly as the Quick Settings tile does: the activity reads the clipboard once focused.
                WidgetButton(
                    "Send clipboard",
                    Intent(ctx, SendClipboardActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                    compact,
                    GlanceModifier.defaultWeight(),
                )
                Spacer(GlanceModifier.width(6.dp))
                WidgetButton(
                    "Send file",
                    Intent(ctx, PickFileActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                    compact,
                    GlanceModifier.defaultWeight(),
                )
            }
        }
    }

    @Composable
    private fun WidgetButton(label: String, intent: Intent, compact: Boolean, modifier: GlanceModifier) {
        val h = if (compact) 22.dp else 30.dp
        Box(
            modifier.height(h).background(GlanceTheme.colors.primaryContainer).cornerRadius(h / 2)
                .clickable(actionStartActivity(intent)),
            contentAlignment = Alignment.Center,
        ) {
            Text(
                label,
                style = TextStyle(color = GlanceTheme.colors.onPrimaryContainer, fontSize = if (compact) 11.sp else 12.sp, fontWeight = FontWeight.Medium),
                maxLines = 1,
            )
        }
    }

    companion object {
        private val COMPACT = DpSize(110.dp, 40.dp)
        private val ROOMY = DpSize(110.dp, 60.dp)
        private val pending = AtomicBoolean(false)

        /** Broadcast to [RelayWidgetReceiver] by the staleness alarm. */
        const val ACTION_STALE = "dev.phonebridge.app.WIDGET_STALE"
        private const val STALE_MS = 3 * 60_000L

        /**
         * While the widget claims "Connected", an inexact alarm redraws it every few minutes: a live
         * process repaints the same thing, a killed one comes back as "Relay is off". Cancelled otherwise.
         */
        private fun scheduleStale(ctx: Context, connected: Boolean) = runCatching {
            val am = ctx.getSystemService(android.app.AlarmManager::class.java)
            val pi = android.app.PendingIntent.getBroadcast(
                ctx, 0, Intent(ctx, RelayWidgetReceiver::class.java).setAction(ACTION_STALE),
                android.app.PendingIntent.FLAG_IMMUTABLE or android.app.PendingIntent.FLAG_UPDATE_CURRENT,
            )
            if (connected) am.set(android.app.AlarmManager.RTC, System.currentTimeMillis() + STALE_MS, pi) else am.cancel(pi)
        }.onFailure { Log.e(TAG, "WIDGET stale alarm failed: $it") }

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
