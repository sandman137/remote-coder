package com.remotecoder.ui

import android.graphics.Paint
import android.graphics.Typeface
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawWithCache
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import uniffi.remotecoder_engine.CellFfi
import uniffi.remotecoder_engine.ColorFfi
import uniffi.remotecoder_engine.GridSnapshotFfi

// CellAttrs bit layout mirrors engine::grid::CellAttrs (validated by the FFI
// cell_attr_bits() accessor; kept in sync here for a fast render path).
private const val ATTR_BOLD = 1
private const val ATTR_UNDERLINE = 1 shl 3
private const val ATTR_REVERSE = 1 shl 4

/**
 * Renders a [GridSnapshotFfi] with a monospace Canvas — one draw pass over the
 * flat cell array, using the theme's terminal palette (deep-space violet).
 * Reports its measured (cols, rows) via [onViewport] so the engine can reflow
 * the remote pane to the phone's viewport (§4.3).
 */
@Composable
fun GridView(
    grid: GridSnapshotFfi?,
    fontSizeSp: Float = 13f,
    modifier: Modifier = Modifier,
    onViewport: (cols: UShort, rows: UShort) -> Unit,
) {
    val term = LocalTerminalColors.current
    val density = LocalDensity.current
    val bgArgb = term.bg.toArgb()
    val fgArgb = term.defaultFg.toArgb()
    val cursorArgb = Astro.cyan.toArgb()
    val ansi = term.ansi16

    val textPaint = remember(fontSizeSp) {
        Paint().apply {
            typeface = Typeface.MONOSPACE
            textSize = with(density) { fontSizeSp.dp.toPx() }
            isAntiAlias = true
        }
    }
    val cellW = remember(textPaint) { textPaint.measureText("M") }
    val cellH = remember(textPaint) { textPaint.fontMetrics.let { it.descent - it.ascent } }

    Box(
        modifier
            .fillMaxSize()
            .clip(RoundedCornerShape(14.dp))
            .background(term.bg)
            .padding(6.dp)
            .drawWithCache {
                val cols = (size.width / cellW).toInt().coerceAtLeast(1)
                val rows = (size.height / cellH).toInt().coerceAtLeast(1)
                onViewport(cols.toUShort(), rows.toUShort())
                onDrawWithContent {
                    drawContent()
                    val g = grid ?: return@onDrawWithContent
                    val canvas = drawContext.canvas.nativeCanvas
                    val ascent = -textPaint.fontMetrics.ascent
                    for (row in 0 until g.rows.toInt()) {
                        val base = row * g.cols.toInt()
                        var col = 0
                        while (col < g.cols.toInt()) {
                            val cell = g.cells[base + col]
                            if (cell.wideContinuation) { col++; continue }
                            drawCell(canvas, textPaint, cell, col, row, cellW, cellH, ascent, fgArgb, bgArgb, ansi)
                            col++
                        }
                    }
                    g.cursor?.let { cur ->
                        textPaint.color = cursorArgb
                        canvas.drawRect(
                            cur.col.toInt() * cellW,
                            (cur.row.toInt() + 1) * cellH - cellH * 0.14f,
                            (cur.col.toInt() + 1) * cellW,
                            (cur.row.toInt() + 1) * cellH,
                            textPaint,
                        )
                    }
                }
            },
    )
}

private fun drawCell(
    canvas: android.graphics.Canvas,
    paint: Paint,
    cell: CellFfi,
    col: Int,
    row: Int,
    cellW: Float,
    cellH: Float,
    ascent: Float,
    defaultFg: Int,
    termBg: Int,
    ansi: IntArray,
) {
    var fg = resolve(cell.fg, default = defaultFg, ansi = ansi)
    var bg = resolve(cell.bg, default = 0, ansi = ansi)
    if (cell.attrs.toInt() and ATTR_REVERSE != 0) {
        val t = fg; fg = if (bg == 0) termBg else bg; bg = t
    }
    val x = col * cellW
    val y = row * cellH
    if (bg != 0) {
        paint.color = bg
        canvas.drawRect(x, y, x + cellW, y + cellH, paint)
    }
    paint.color = fg
    paint.isFakeBoldText = cell.attrs.toInt() and ATTR_BOLD != 0
    paint.isUnderlineText = cell.attrs.toInt() and ATTR_UNDERLINE != 0
    if (cell.ch.isNotBlank()) {
        canvas.drawText(cell.ch, x, y + ascent, paint)
    }
}

private fun resolve(c: ColorFfi, default: Int, ansi: IntArray): Int = when (c) {
    is ColorFfi.Default -> default
    is ColorFfi.Indexed -> indexed(c.index.toInt(), ansi)
    is ColorFfi.Rgb -> (0xFF shl 24) or (c.r.toInt() shl 16) or (c.g.toInt() shl 8) or c.b.toInt()
}

private fun indexed(i: Int, ansi: IntArray): Int = when {
    i < 16 -> ansi[i]
    i in 16..231 -> {
        val n = i - 16
        val r = n / 36; val g = (n % 36) / 6; val b = n % 6
        fun c(v: Int) = if (v == 0) 0 else v * 40 + 55
        (0xFF shl 24) or (c(r) shl 16) or (c(g) shl 8) or c(b)
    }
    else -> {
        val v = 8 + (i - 232) * 10
        (0xFF shl 24) or (v shl 16) or (v shl 8) or v
    }
}
