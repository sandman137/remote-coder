package com.helm.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.weight
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Mic
import androidx.compose.material.icons.filled.Send
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import uniffi.helm_engine.PaneInfoFfi

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PaneListScreen(
    session: String,
    panes: List<PaneInfoFfi>,
    attention: Set<String>,
    onOpen: (PaneInfoFfi) -> Unit,
    onRefresh: () -> Unit,
) {
    Scaffold(topBar = {
        TopAppBar(title = { Text("HELM — $session (${panes.size} panes)") })
    }) { pad ->
        LazyColumn(Modifier.padding(pad).fillMaxSize()) {
            items(panes, key = { it.id }) { pane ->
                val waiting = pane.id in attention
                Card(
                    Modifier
                        .fillMaxWidth()
                        .padding(8.dp),
                    onClick = { onOpen(pane) },
                ) {
                    Row(
                        Modifier.padding(16.dp).fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(if (waiting) "⚠" else "●", Modifier.padding(end = 12.dp))
                        Column(Modifier.weight(1f)) {
                            Text(
                                "${pane.windowName}  ·  ${pane.currentCommand}",
                                style = MaterialTheme.typography.titleMedium,
                            )
                            Text(
                                "${pane.session}:${pane.windowIndex}.${pane.paneIndex}  " +
                                    "${pane.width}×${pane.height}" +
                                    if (waiting) "  · needs input" else "",
                                style = MaterialTheme.typography.bodySmall,
                            )
                        }
                    }
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PaneScreen(
    state: UiState,
    onBack: () -> Unit,
    onButton: (String) -> Unit,
    onSend: (String) -> Unit,
    onScroll: (Int) -> Unit,
    onStt: () -> Unit,
    onViewport: (UShort, UShort) -> Unit,
) {
    val pane = (state.screen as? Screen.Pane)?.pane ?: return
    var input by remember { mutableStateOf("") }
    val chips = state.metadata[pane.id]?.entries
        ?.sortedBy { it.key }
        ?.joinToString(" ") { "${it.key}:${it.value}" }
        .orEmpty()

    Scaffold(
        topBar = {
            TopAppBar(title = {
                Column {
                    Text("${pane.session}:${pane.windowIndex}.${pane.paneIndex} — ${pane.currentCommand}")
                    if (chips.isNotEmpty()) {
                        Text(chips, style = MaterialTheme.typography.labelSmall)
                    }
                }
            })
        },
    ) { pad ->
        Column(Modifier.padding(pad).fillMaxSize()) {
            GridView(
                grid = state.grid,
                modifier = Modifier.weight(1f).fillMaxWidth(),
                onViewport = onViewport,
            )

            // Adapter/quick-action button row.
            Row(
                Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 4.dp),
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                state.buttons.take(4).forEach { b ->
                    Button(onClick = { onButton(b.label) }, Modifier.weight(1f)) {
                        Text(b.label, maxLines = 1)
                    }
                }
            }

            // Scrollback controls.
            Row(
                Modifier.fillMaxWidth().padding(horizontal = 8.dp),
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                Button(onClick = { onScroll(+10) }, Modifier.weight(1f)) { Text("▲ older") }
                Button(onClick = { onScroll(-10) }, Modifier.weight(1f)) { Text("▼ newer") }
                Button(onClick = onBack, Modifier.weight(1f)) { Text("Back") }
            }

            // Text + speech-to-text input line.
            Row(
                Modifier.fillMaxWidth().padding(8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                OutlinedTextField(
                    value = input,
                    onValueChange = { input = it },
                    modifier = Modifier.weight(1f),
                    singleLine = true,
                    placeholder = { Text("type or dictate…") },
                )
                IconButton(onClick = onStt) { Icon(Icons.Default.Mic, "dictate") }
                IconButton(onClick = {
                    if (input.isNotEmpty()) { onSend(input); input = "" } else onButton("Enter")
                }) { Icon(Icons.Default.Send, "send") }
            }

            if (state.status.isNotEmpty()) {
                Text(
                    state.status,
                    Modifier.padding(horizontal = 8.dp, vertical = 2.dp),
                    style = MaterialTheme.typography.labelSmall,
                )
            }
        }
    }
}

/** Unused import guard so FontFamily stays referenced for future styling. */
private val monospace = FontFamily.Monospace
