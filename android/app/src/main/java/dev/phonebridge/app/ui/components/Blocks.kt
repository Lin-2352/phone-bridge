package dev.phonebridge.app.ui.settings

import android.content.Context
import android.provider.Settings
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.Clear
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.phonebridge.app.Radius
import dev.phonebridge.app.ui.isDark

/** True unless the system "remove animations" setting (animator scale 0) is on. */
fun animationsEnabled(ctx: Context): Boolean =
    runCatching { Settings.Global.getFloat(ctx.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f) > 0f }.getOrDefault(true)

/** The filled card colour: one step above the page, a touch brighter in light mode so it reads on the grey page. */
@Composable
fun cardColor(): Color =
    if (isDark()) MaterialTheme.colorScheme.surfaceContainer else MaterialTheme.colorScheme.surfaceContainerHigh

/** A section header: sentence case, quiet, above its card. */
@Composable
fun SettingsHeader(title: String) {
    Text(
        title,
        style = MaterialTheme.typography.titleSmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier
            .fillMaxWidth()
            .padding(start = 32.dp, end = 16.dp, top = 16.dp, bottom = 8.dp)
            .semantics { heading() },
    )
}

/** A filled, rounded surface that holds a section's rows. No outline: the tone carries the edge. */
@Composable
fun SettingsCard(modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    Column(
        modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 4.dp)
            .clip(RoundedCornerShape(Radius.card))
            .background(cardColor()),
    ) { content() }
}

/** A hairline between two rows, inset to line up with the text. */
@Composable
fun RowDivider(inset: Boolean = false) {
    HorizontalDivider(
        modifier = Modifier.padding(start = if (inset) 72.dp else 16.dp, end = 16.dp),
        color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.6f),
    )
}

/** A leading icon in a tinted circle. The icon is decorative: the row's text names it. */
@Composable
fun IconBadge(icon: ImageVector, tint: Color, modifier: Modifier = Modifier) {
    Box(
        modifier
            .size(40.dp)
            .clip(CircleShape)
            .background(tint.copy(alpha = if (isDark()) 0.22f else 0.14f)),
        contentAlignment = Alignment.Center,
    ) { Icon(icon, contentDescription = null, tint = tint, modifier = Modifier.size(22.dp)) }
}

/**
 * One row inside a [SettingsCard]. With [checked] set it is a switch row: the whole row toggles (unless
 * [onClick] is given, then only the Switch does) and TalkBack reads label, text and state as one switch.
 * [danger] paints the label in the error colour.
 */
@Composable
fun SettingItem(
    title: String,
    supporting: String? = null,
    modifier: Modifier = Modifier,
    checked: Boolean? = null,
    onChecked: ((Boolean) -> Unit)? = null,
    switchEnabled: Boolean = true,
    onClick: (() -> Unit)? = null,
    danger: Boolean = false,
    trailing: (@Composable () -> Unit)? = null,
    titleColor: Color = Color.Unspecified,
) {
    val rowToggles = checked != null && onClick == null && onChecked != null
    var m = modifier.heightIn(min = 56.dp)
    m = when {
        rowToggles -> m.toggleable(value = checked!!, enabled = switchEnabled, role = Role.Switch, onValueChange = onChecked!!)
        onClick != null -> m.clickable(role = Role.Button, onClick = onClick)
        else -> m
    }
    ListItem(
        headlineContent = {
            Text(
                title,
                color = when {
                    danger -> MaterialTheme.colorScheme.error
                    titleColor != Color.Unspecified -> titleColor
                    else -> MaterialTheme.colorScheme.onSurface
                },
            )
        },
        supportingContent = supporting?.let { { Text(it) } },
        trailingContent = if (trailing != null || checked != null) {
            {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    trailing?.invoke()
                    if (checked != null) Switch(checked, if (rowToggles) null else onChecked, enabled = switchEnabled)
                }
            }
        } else null,
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        modifier = if (rowToggles) m else m.semantics(mergeDescendants = true) {},
    )
}

/** A top-level category entry: tinted icon, title, live status, chevron. */
@Composable
fun CategoryRow(icon: ImageVector, tint: Color, title: String, status: String, onClick: () -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .heightIn(min = 72.dp)
            .clickable(role = Role.Button, onClick = onClick)
            .semantics(mergeDescendants = true) { contentDescription = "$title. $status" }
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        IconBadge(icon, tint)
        Column(Modifier.weight(1f).padding(horizontal = 16.dp)) {
            Text(title, style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.onSurface)
            Text(
                status,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
        }
        Icon(Icons.AutoMirrored.Filled.KeyboardArrowRight, null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

/** The compact pill search field: filled, fully rounded, with a clear button once there is text. */
@Composable
fun PillSearch(value: String, onValueChange: (String) -> Unit, modifier: Modifier = Modifier) {
    TextField(
        value = value,
        onValueChange = onValueChange,
        singleLine = true,
        placeholder = { Text("Search settings") },
        leadingIcon = { Icon(Icons.Filled.Search, null) },
        trailingIcon = {
            if (value.isNotEmpty()) IconButton(onClick = { onValueChange("") }) { Icon(Icons.Filled.Clear, "Clear search") }
        },
        shape = CircleShape,
        colors = TextFieldDefaults.colors(
            focusedContainerColor = cardColor(),
            unfocusedContainerColor = cardColor(),
            focusedIndicatorColor = Color.Transparent,
            unfocusedIndicatorColor = Color.Transparent,
            disabledIndicatorColor = Color.Transparent,
        ),
        modifier = modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp).heightIn(min = 48.dp),
    )
}
