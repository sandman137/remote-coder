package com.helm.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

private val Dark = darkColorScheme(
    primary = Color(0xFF7FDBFF),
    secondary = Color(0xFF2ECC40),
    error = Color(0xFFFF4136),
)
private val Light = lightColorScheme(
    primary = Color(0xFF0074D9),
    secondary = Color(0xFF2ECC40),
    error = Color(0xFFFF4136),
)

@Composable
fun HelmTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = if (isSystemInDarkTheme()) Dark else Light,
        content = content,
    )
}
