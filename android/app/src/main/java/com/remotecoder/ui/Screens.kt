package com.remotecoder.ui

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Mic
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Send
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.remotecoder.R
import uniffi.remotecoder_engine.PaneInfoFfi

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PaneListScreen(
    session: String,
    panes: List<PaneInfoFfi>,
    attention: Set<String>,
    error: String? = null,
    onOpen: (PaneInfoFfi) -> Unit,
    onRefresh: () -> Unit,
) {
    Scaffold(topBar = {
        TopAppBar(
            colors = TopAppBarDefaults.topAppBarColors(containerColor = Color.White),
            actions = {
                IconButton(onClick = onRefresh) {
                    Icon(Icons.Filled.Refresh, contentDescription = "Refresh", tint = Astro.violet)
                }
            },
            title = {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Image(
                    painterResource(R.drawable.astro_bust),
                    contentDescription = null,
                    modifier = Modifier.size(32.dp).clip(CircleShape),
                )
                Column(Modifier.padding(start = 10.dp)) {
                    Text(
                        buildAnnotatedString {
                            withStyle(androidx.compose.ui.text.SpanStyle(color = Astro.ink)) { append("Remote ") }
                            withStyle(androidx.compose.ui.text.SpanStyle(color = Astro.magenta)) { append("Coder") }
                        },
                        fontFamily = FontFamily.SansSerif, fontWeight = FontWeight.ExtraBold, fontSize = 18.sp,
                    )
                    Text(
                        "$session · ${panes.size} panes · ssh",
                        fontFamily = FontFamily.Monospace, fontSize = 10.sp, color = Astro.muted,
                    )
                }
            }
        })
    }) { pad ->
        if (panes.isEmpty()) {
            // Empty session — tell the user exactly how to get an agent here.
            Column(
                Modifier.padding(pad).fillMaxSize().padding(28.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.Center,
            ) {
                Image(
                    painterResource(R.drawable.astro_bust),
                    contentDescription = null,
                    modifier = Modifier.size(96.dp).clip(CircleShape),
                )
                Text(
                    "No agents yet",
                    style = MaterialTheme.typography.headlineSmall,
                    modifier = Modifier.padding(top = 16.dp),
                )
                Text(
                    "Connected to '$session', but nothing is running in it.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = Astro.muted,
                    modifier = Modifier.padding(top = 6.dp),
                )
                Text(
                    "On your dev box:\n\n  tmux attach -t $session\n\nthen launch  claude  (or codex).\nEach tmux window appears here as a pane.",
                    fontFamily = FontFamily.Monospace, fontSize = 13.sp,
                    modifier = Modifier.padding(top = 14.dp),
                )
                if (error != null) {
                    Text(
                        error,
                        color = Astro.magenta,
                        style = MaterialTheme.typography.bodySmall,
                        modifier = Modifier.padding(top = 14.dp),
                    )
                }
                Button(
                    onClick = onRefresh,
                    colors = ButtonDefaults.buttonColors(containerColor = Astro.magenta),
                    modifier = Modifier.padding(top = 18.dp),
                ) { Text("Refresh") }
            }
            return@Scaffold
        }
        LazyColumn(Modifier.padding(pad).fillMaxSize()) {
            if (error != null) {
                item {
                    Text(
                        error,
                        color = Astro.magenta,
                        style = MaterialTheme.typography.bodySmall,
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 6.dp),
                    )
                }
            }
            items(panes, key = { it.id }) { pane ->
                val waiting = pane.id in attention
                Card(
                    onClick = { onOpen(pane) },
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 12.dp, vertical = 6.dp),
                    colors = CardDefaults.cardColors(containerColor = Color.White),
                    elevation = CardDefaults.cardElevation(defaultElevation = 0.dp),
                    border = BorderStroke(1.5.dp, if (waiting) Astro.magenta else Astro.line),
                ) {
                    Row(
                        Modifier.padding(16.dp).fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(
                            if (waiting) "⚠" else "●",
                            color = if (waiting) Astro.magenta else Astro.mint,
                            modifier = Modifier.padding(end = 12.dp),
                        )
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
            TopAppBar(colors = TopAppBarDefaults.topAppBarColors(containerColor = Color.White), title = {
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
