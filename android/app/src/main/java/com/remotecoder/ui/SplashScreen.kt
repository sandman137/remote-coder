package com.remotecoder.ui

import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
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
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.remotecoder.R

/**
 * Astro-HD splash (v1, full-bleed): the astronaut artwork fills the top of the
 * screen and fades into white where the wordmark and connecting indicator sit.
 */
@Composable
fun SplashScreen(status: String = "connecting to tailnet") {
    val transition = rememberInfiniteTransition(label = "splash")
    val pip by transition.animateFloat(
        initialValue = 0f, targetValue = 3f,
        animationSpec = infiniteRepeatable(tween(1200, easing = LinearEasing)),
        label = "pip",
    )

    Column(Modifier.fillMaxSize().background(Color.White)) {
        Box(Modifier.weight(1f).fillMaxWidth()) {
            Image(
                painter = painterResource(R.drawable.splash_astro),
                contentDescription = "Remote Coder",
                modifier = Modifier.fillMaxSize(),
                contentScale = ContentScale.Crop,
                alignment = Alignment.TopCenter,
            )
            // fade the bottom of the art into white so the wordmark reads cleanly
            Box(
                Modifier.fillMaxSize().background(
                    Brush.verticalGradient(
                        0.62f to Color.Transparent,
                        0.92f to Color.White,
                        1f to Color.White,
                    ),
                ),
            )
        }
        Column(
            Modifier.fillMaxWidth().background(Color.White).padding(bottom = 44.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Text(
                buildAnnotatedString {
                    withStyle(SpanInk) { append("Remote ") }
                    withStyle(SpanMagenta) { append("Coder") }
                },
                style = TextStyle(fontFamily = FontFamily.SansSerif, fontWeight = FontWeight.ExtraBold, fontSize = 30.sp, letterSpacing = (-0.8).sp),
            )
            Spacer(Modifier.height(4.dp))
            Text(
                "Your agents, anywhere.",
                style = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 13.sp, fontWeight = FontWeight.Medium),
                color = Astro.muted,
            )
            Spacer(Modifier.height(18.dp))
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(
                    status,
                    style = TextStyle(fontFamily = FontFamily.Monospace, fontWeight = FontWeight.Bold, fontSize = 11.sp),
                    color = Astro.muted,
                )
                Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                    repeat(3) { i ->
                        val on = pip.toInt() == i
                        Box(
                            Modifier.size(6.dp).clip(RoundedCornerShape(3.dp))
                                .background(if (on) Astro.magenta else Astro.line),
                        )
                    }
                }
            }
        }
    }
}

private val SpanInk = androidx.compose.ui.text.SpanStyle(color = Astro.ink)
private val SpanMagenta = androidx.compose.ui.text.SpanStyle(color = Astro.magenta)
