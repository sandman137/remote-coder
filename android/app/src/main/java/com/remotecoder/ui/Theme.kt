package com.remotecoder.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp

/**
 * "Cosmic Coder" — a light-first, vibrant-retro identity: pink→lavender sky,
 * magenta accent, gold + cyan pops, and a deep-space-violet terminal panel so
 * agent output glows. No dark chrome, no gray drop-shadows — flat color and
 * colored borders instead.
 */
object Cosmic {
    val ink = Color(0xFF2E2359)
    val muted = Color(0xFF8F7EC2)
    val bg = Color(0xFFF7F1FF)
    val card = Color(0xFFFFFFFF)
    val line = Color(0xFFE6D6FF)

    val magenta = Color(0xFFE93D82)
    val violet = Color(0xFF6C4BD8)
    val cyan = Color(0xFF35D6E0)
    val gold = Color(0xFFFFC24D)
    val mint = Color(0xFF35D6A0)

    val sky1 = Color(0xFFFFD1E8)
    val sky2 = Color(0xFFE3C9FF)
    val sky3 = Color(0xFFB9A6FF)

    val termBg = Color(0xFF352C6B)
    val termInk = Color(0xFFF2ECFF)
    val termDim = Color(0xFF9C8FD4)

    // ANSI 0..15, tuned to glow on the violet terminal.
    val ansi = intArrayOf(
        0xFF352C6B.toInt(), 0xFFFF5C8A.toInt(), 0xFF4BE3A4.toInt(), 0xFFFFC24D.toInt(),
        0xFF6FA8FF.toInt(), 0xFFFF8ADD.toInt(), 0xFF4DE0E0.toInt(), 0xFFF2ECFF.toInt(),
        0xFF6A5DA8.toInt(), 0xFFFF7CA3.toInt(), 0xFF6BF0BE.toInt(), 0xFFFFD37A.toInt(),
        0xFF97C2FF.toInt(), 0xFFFFA9E6.toInt(), 0xFF7DEBEB.toInt(), 0xFFFFFFFF.toInt(),
    )
}

/** Terminal colors handed to [GridView] via composition local. */
class TerminalColors(
    val bg: Color,
    val defaultFg: Color,
    val dim: Color,
    val ansi16: IntArray,
)

val LocalTerminalColors = staticCompositionLocalOf {
    TerminalColors(Cosmic.termBg, Cosmic.termInk, Cosmic.termDim, Cosmic.ansi)
}

private val CosmicLight = lightColorScheme(
    primary = Cosmic.magenta,
    onPrimary = Color.White,
    secondary = Cosmic.violet,
    onSecondary = Color.White,
    tertiary = Cosmic.cyan,
    background = Cosmic.bg,
    onBackground = Cosmic.ink,
    surface = Cosmic.card,
    onSurface = Cosmic.ink,
    surfaceVariant = Color(0xFFF1E9FF),
    onSurfaceVariant = Cosmic.muted,
    outline = Cosmic.line,
    error = Color(0xFFFF5C8A),
)

// OS dark mode: still violet-forward and bright — never black.
private val CosmicDark = darkColorScheme(
    primary = Color(0xFFFF6FAE),
    onPrimary = Color(0xFF2A0E24),
    secondary = Color(0xFF9E86FF),
    tertiary = Cosmic.cyan,
    background = Color(0xFF241C4A),
    onBackground = Color(0xFFF2ECFF),
    surface = Color(0xFF2E2559),
    onSurface = Color(0xFFF2ECFF),
    surfaceVariant = Color(0xFF3A2F6B),
    onSurfaceVariant = Color(0xFFB9AEE6),
    outline = Color(0xFF4A3E80),
    error = Color(0xFFFF7CA3),
)

private val mono = FontFamily.Monospace

private val CosmicType = Typography(
    headlineSmall = TextStyle(fontFamily = mono, fontWeight = FontWeight.Black, fontSize = 24.sp, letterSpacing = (-0.5).sp),
    titleLarge = TextStyle(fontFamily = mono, fontWeight = FontWeight.Black, fontSize = 20.sp, letterSpacing = (-0.3).sp),
    titleMedium = TextStyle(fontFamily = mono, fontWeight = FontWeight.Bold, fontSize = 15.sp),
    labelSmall = TextStyle(fontFamily = mono, fontWeight = FontWeight.Bold, fontSize = 11.sp, letterSpacing = 0.5.sp),
    bodyMedium = TextStyle(fontSize = 14.sp),
    bodySmall = TextStyle(fontFamily = mono, fontSize = 12.sp),
)

@Composable
fun RemoteCoderTheme(content: @Composable () -> Unit) {
    val dark = isSystemInDarkTheme()
    val terminal = TerminalColors(
        bg = if (dark) Color(0xFF241C4A) else Cosmic.termBg,
        defaultFg = Cosmic.termInk,
        dim = Cosmic.termDim,
        ansi16 = Cosmic.ansi,
    )
    CompositionLocalProvider(LocalTerminalColors provides terminal) {
        MaterialTheme(
            colorScheme = if (dark) CosmicDark else CosmicLight,
            typography = CosmicType,
            content = content,
        )
    }
}
