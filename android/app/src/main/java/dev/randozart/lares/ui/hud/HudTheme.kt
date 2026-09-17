package dev.randozart.lares.ui.hud

import androidx.compose.material3.ColorScheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.randozart.lares.R

/** FLIR cockpit amber — the default symbology color. */
val HudAmber = Color(0xFFFFB000)

/** Phosphor CRT green — selectable alternative. */
val HudGreen = Color(0xFF33FF33)

/** Absolute black: unpowered OLED pixels. */
val HudBlack = Color(0xFF000000)

/** Bracket / leader stroke width: hairline, Ace-Combat-style. */
val HudStroke: Dp = 1.dp

/** Fine stroke width for leaders and dim marks. */
val HudFineStroke: Dp = 1.dp

/** Corner radius for HUD control frames: nearly square. */
val HudCorner: Dp = 2.dp

/** Share Tech Mono (SIL OFL), vendored in res/font. */
val HudFont = FontFamily(
    Font(R.font.share_tech_mono, FontWeight.Normal),
)

/** Resolved symbology palette for the HUD. */
data class HudPalette(
    val primary: Color,
    val dim: Color,
    val background: Color,
)

/** Ambient palette holder so composables read one consistent theme. */
val LocalHudPalette = staticCompositionLocalOf {
    HudPalette(primary = HudAmber, dim = HudAmber.copy(alpha = 0.4f), background = HudBlack)
}

/** Resolve the palette from the user's stored color choice. */
fun hudPalette(colorName: String): HudPalette {
    val primary = if (colorName == "green") HudGreen else HudAmber
    return HudPalette(
        primary = primary,
        dim = primary.copy(alpha = 0.4f),
        background = HudBlack,
    )
}

/** Uppercase helper: every HUD string is telegraphic caps. */
fun hud(text: String): String = text.uppercase()

/** Material color scheme mapping the HUD palette onto all M3 components. */
fun hudColorScheme(palette: HudPalette): ColorScheme = darkColorScheme(
    primary = palette.primary,
    onPrimary = HudBlack,
    secondary = palette.primary,
    onSecondary = HudBlack,
    background = palette.background,
    onBackground = palette.primary,
    surface = palette.background,
    onSurface = palette.primary,
    surfaceVariant = Color(0xFF111111),
    onSurfaceVariant = palette.dim,
    outline = palette.dim,
)
