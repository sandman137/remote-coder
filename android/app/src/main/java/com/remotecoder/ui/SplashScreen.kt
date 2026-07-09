package com.remotecoder.ui

import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/**
 * Cosmic Coder splash: pixel astronaut floating over a pink→periwinkle sky
 * with a gold-ringed planet, the wordmark, and an animated "connecting to
 * tailnet" line. Shown on cold start while the app settles.
 */
@Composable
fun SplashScreen(status: String = "connecting to tailnet") {
    val sky = Brush.verticalGradient(listOf(Cosmic.sky1, Cosmic.sky2, Cosmic.sky3))
    val transition = rememberInfiniteTransition(label = "splash")
    val bob by transition.animateFloat(
        initialValue = 0f, targetValue = 1f,
        animationSpec = infiniteRepeatable(tween(1800, easing = LinearEasing), RepeatMode.Reverse),
        label = "bob",
    )
    val pip by transition.animateFloat(
        initialValue = 0f, targetValue = 3f,
        animationSpec = infiniteRepeatable(tween(1200, easing = LinearEasing)),
        label = "pip",
    )

    Box(Modifier.fillMaxSize().background(sky), contentAlignment = Alignment.TopCenter) {
        // planet + ring + stars behind everything
        Canvas(Modifier.fillMaxSize()) {
            val planetR = size.minDimension * 0.34f
            val cx = size.width / 2f
            val cy = size.height * 0.30f
            drawCircle(Cosmic.magenta.copy(alpha = 0.55f), planetR, Offset(cx, cy))
            // gold ring (an ellipse hint via thick arc)
            drawArc(
                color = Cosmic.gold,
                startAngle = 20f, sweepAngle = 300f, useCenter = false,
                topLeft = Offset(cx - planetR * 1.4f, cy + planetR * 0.35f),
                size = Size(planetR * 2.8f, planetR * 0.7f),
                style = androidx.compose.ui.graphics.drawscope.Stroke(width = 6f),
            )
            listOf(
                Offset(size.width * 0.16f, size.height * 0.14f) to Cosmic.cyan,
                Offset(size.width * 0.82f, size.height * 0.10f) to Color.White,
                Offset(size.width * 0.88f, size.height * 0.22f) to Cosmic.gold,
                Offset(size.width * 0.10f, size.height * 0.30f) to Color.White,
                Offset(size.width * 0.90f, size.height * 0.40f) to Cosmic.cyan,
            ).forEach { (p, c) -> drawCircle(c, 3.5f, p) }
        }

        Column(
            Modifier.fillMaxSize().padding(bottom = 40.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Spacer(Modifier.height(140.dp))
            PixelAstronaut(
                Modifier
                    .size(150.dp)
                    .padding(top = (bob * 8).dp, bottom = ((1 - bob) * 8).dp),
            )
            Spacer(Modifier.height(18.dp))
            Text(
                "REMOTE",
                style = TextStyle(fontFamily = FontFamily.Monospace, fontWeight = FontWeight.Black, fontSize = 26.sp, letterSpacing = 3.sp),
                color = Cosmic.ink, textAlign = TextAlign.Center,
            )
            Text(
                "CODER",
                style = TextStyle(fontFamily = FontFamily.Monospace, fontWeight = FontWeight.Black, fontSize = 26.sp, letterSpacing = 3.sp),
                color = Cosmic.magenta, textAlign = TextAlign.Center,
            )
            Spacer(Modifier.height(8.dp))
            Text(
                "CODE FROM ORBIT",
                style = TextStyle(fontFamily = FontFamily.Monospace, fontWeight = FontWeight.Bold, fontSize = 11.sp, letterSpacing = 3.sp),
                color = Cosmic.violet,
            )
            Spacer(Modifier.weight(1f))
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(
                    status,
                    style = TextStyle(fontFamily = FontFamily.Monospace, fontWeight = FontWeight.Bold, fontSize = 11.sp),
                    color = Cosmic.muted,
                )
                Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                    repeat(3) { i ->
                        val on = pip.toInt() == i
                        Box(
                            Modifier.size(6.dp).clip(RoundedCornerShape(2.dp))
                                .background(if (on) Cosmic.magenta else Cosmic.line),
                        )
                    }
                }
            }
        }
    }
}

/**
 * The 20×22 pixel astronaut mascot, drawn as flat rectangles (crisp, no AA).
 * Shared across splash, empty states, and the app icon foreground.
 */
@Composable
fun PixelAstronaut(modifier: Modifier = Modifier) {
    Canvas(modifier) {
        val cols = 20; val rows = 22
        val px = size.width / cols
        val py = size.height / rows
        for (y in MAP.indices) {
            val line = MAP[y]
            for (x in line.indices) {
                val c = colorFor(line[x]) ?: continue
                drawRect(c, topLeft = Offset(x * px, y * py), size = Size(px + 0.6f, py + 0.6f))
            }
        }
    }
}

private fun colorFor(ch: Char): Color? = when (ch) {
    'o' -> Cosmic.ink               // outline
    'w' -> Color.White              // suit
    's' -> Color(0xFFEFE4FF)        // suit shade
    'v' -> Cosmic.cyan              // visor
    'V' -> Color(0xFFC7F7F7)        // visor shine
    'c' -> Cosmic.gold              // antenna ball
    't' -> Cosmic.cyan              // side tanks
    'p' -> Cosmic.magenta           // laptop lid
    'h' -> Cosmic.gold              // sticker
    else -> null
}

private val MAP = listOf(
    ".........cc.........",
    ".........oo.........",
    "......oooooooo......",
    ".....owwwwwwwwo.....",
    "....owwwwwwwwwwo....",
    "....owvvvvvvvvwo....",
    "....owvVVvvvvvwo....",
    "....owvvvvvvvvwo....",
    "....owwvvvvvvwwo....",
    ".....owwwwwwwwo.....",
    "......oooooooo......",
    "...oowwwwwwwwwwoo...",
    "..otwwwwwwwwwwwwto..",
    "..otwwsswwwwsswwto..",
    "..owwwwwwwwwwwwwwo..",
    "...owwppppppppwwo...",
    "....oppppppppppo....",
    "....opppphhppppo....",
    "....oppppppppppo....",
    "...owwwwwwwwwwwwo...",
    "....oowwwwwwwwoo....",
    "......oooooooo......",
)
