package com.remotecoder.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp

/**
 * "Astro HD" — clean white chrome with vibrant accents pulled from the
 * astronaut artwork. Pure-white surfaces, cool-gray hairlines, a
 * magenta→violet gradient reserved for primary actions and the wordmark, and
 * a deep-indigo terminal panel so agent output and the glow palette read like
 * the art. Geometric sans everywhere; monospace only inside the terminal.
 */
object Astro {
    val ink = Color(0xFF1B1038)
    val muted = Color(0xFF6E6A85)
    val bg = Color(0xFFFFFFFF)
    val card = Color(0xFFFFFFFF)
    val line = Color(0xFFECECF2)
    val surfaceSoft = Color(0xFFF6F5FA)

    val magenta = Color(0xFFE5518F)
    val violet = Color(0xFF6C2D87)
    val cyan = Color(0xFF59D9E8)
    val gold = Color(0xFFFFC24D)
    val mint = Color(0xFF2FD6A0)

    // magenta→violet, for the wordmark + primary buttons
    val brand = Brush.horizontalGradient(listOf(magenta, violet))
    val brandButton = Brush.horizontalGradient(listOf(Color(0xFFE5518F), Color(0xFFB23577)))

    // deep-indigo terminal
    val termBg = Color(0xFF241243)
    val termInk = Color(0xFFF2ECFF)
    val termDim = Color(0xFF8C7FC5)

    // ANSI 0..15 tuned to glow on indigo
    val ansi = intArrayOf(
        0xFF241243.toInt(), 0xFFFF5C8A.toInt(), 0xFF3DE8A0.toInt(), 0xFFFFC24D.toInt(),
        0xFF6FA8FF.toInt(), 0xFFFF7ADF.toInt(), 0xFF4DE0E0.toInt(), 0xFFF2ECFF.toInt(),
        0xFF6A5DA8.toInt(), 0xFFFF7CA3.toInt(), 0xFF6BF0BE.toInt(), 0xFFFFD37A.toInt(),
        0xFF97C2FF.toInt(), 0xFFFFA9E6.toInt(), 0xFF7DEBEB.toInt(), 0xFFFFFFFF.toInt(),
    )
}

class TerminalColors(
    val bg: Color,
    val defaultFg: Color,
    val dim: Color,
    val ansi16: IntArray,
)

val LocalTerminalColors = staticCompositionLocalOf {
    TerminalColors(Astro.termBg, Astro.termInk, Astro.termDim, Astro.ansi)
}

private val AstroLight = lightColorScheme(
    primary = Astro.magenta,
    onPrimary = Color.White,
    secondary = Astro.violet,
    onSecondary = Color.White,
    tertiary = Astro.cyan,
    background = Astro.bg,
    onBackground = Astro.ink,
    surface = Astro.card,
    onSurface = Astro.ink,
    surfaceVariant = Astro.surfaceSoft,
    onSurfaceVariant = Astro.muted,
    outline = Astro.line,
    error = Color(0xFFE5462E),
)

// OS dark mode: deep-indigo, still vibrant — never pure black.
private val AstroDark = darkColorScheme(
    primary = Color(0xFFFF6FAE),
    onPrimary = Color(0xFF2A0E24),
    secondary = Color(0xFFB07BE0),
    tertiary = Astro.cyan,
    background = Color(0xFF160C33),
    onBackground = Color(0xFFF2ECFF),
    surface = Color(0xFF1F1440),
    onSurface = Color(0xFFF2ECFF),
    surfaceVariant = Color(0xFF2A1D52),
    onSurfaceVariant = Color(0xFFB9AEE6),
    outline = Color(0xFF3B2E66),
    error = Color(0xFFFF7A5C),
)

private val sans = FontFamily.SansSerif
private val mono = FontFamily.Monospace

private val AstroType = Typography(
    headlineSmall = TextStyle(fontFamily = sans, fontWeight = FontWeight.ExtraBold, fontSize = 26.sp, letterSpacing = (-0.5).sp),
    titleLarge = TextStyle(fontFamily = sans, fontWeight = FontWeight.ExtraBold, fontSize = 20.sp, letterSpacing = (-0.4).sp),
    titleMedium = TextStyle(fontFamily = sans, fontWeight = FontWeight.Bold, fontSize = 15.5.sp, letterSpacing = (-0.2).sp),
    labelSmall = TextStyle(fontFamily = mono, fontWeight = FontWeight.Bold, fontSize = 11.sp),
    bodyMedium = TextStyle(fontFamily = sans, fontSize = 14.sp),
    bodySmall = TextStyle(fontFamily = mono, fontSize = 12.sp),
)

@Composable
fun RemoteCoderTheme(content: @Composable () -> Unit) {
    val dark = isSystemInDarkTheme()
    val terminal = TerminalColors(
        bg = if (dark) Color(0xFF1B0F3A) else Astro.termBg,
        defaultFg = Astro.termInk,
        dim = Astro.termDim,
        ansi16 = Astro.ansi,
    )
    CompositionLocalProvider(LocalTerminalColors provides terminal) {
        MaterialTheme(
            colorScheme = if (dark) AstroDark else AstroLight,
            typography = AstroType,
            content = content,
        )
    }
}
