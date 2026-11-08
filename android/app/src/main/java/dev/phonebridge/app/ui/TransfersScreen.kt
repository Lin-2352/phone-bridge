package dev.phonebridge.app.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp

/** The Transfers screen: files moving now, waiting, and finished, fed from [Transfers.items]. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TransfersScreen(onBack: () -> Unit) {
    val rows = transferRows(Transfers.items.toList())
    val sizes = Transfers.items.associate { it.id to (it.done to it.size) }
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Transfers") },
                navigationIcon = {
                    IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back") }
                },
            )
        },
    ) { pad ->
        if (rows.isEmpty()) {
            Text(
                emptyTransfersText(),
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(pad).padding(24.dp),
            )
            return@Scaffold
        }
        LazyColumn(Modifier.padding(pad).fillMaxSize().padding(horizontal = 16.dp)) {
            items(rows, key = { it.id }) { r ->
                val (done, total) = sizes[r.id] ?: (0L to 0L)
                Column(
                    Modifier.fillMaxWidth().padding(vertical = 8.dp)
                        .semantics(mergeDescendants = true) { contentDescription = "${r.title}, ${r.status}, ${r.percentText}" },
                ) {
                    Text(r.title, style = MaterialTheme.typography.bodyLarge)
                    Text(r.status, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    if (r.state == TransferState.Running && total > 0) {
                        Spacer(Modifier.height(6.dp))
                        LinearProgressIndicator(progress = { done.toFloat() / total.toFloat() }, modifier = Modifier.fillMaxWidth())
                    }
                }
            }
        }
    }
}
