package dev.randozart.lares.ui.hud

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/** Disabled control alpha. */
private const val DISABLED_ALPHA = 0.30f

/**
 * Wire-frame HUD control: hairline border, nearly square corners, compact
 * mono caps. Replaces chunky Material buttons.
 */
@Composable
fun HudButton(
    text: String,
    palette: HudPalette,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    filled: Boolean = false,
) {
    val alpha = if (enabled) 1f else DISABLED_ALPHA
    val border = palette.primary.copy(alpha = alpha)
    Box(
        modifier = modifier
            .border(HudStroke, border, RoundedCornerShape(HudCorner))
            .background(
                when {
                    filled && enabled -> palette.primary
                    filled -> palette.primary.copy(alpha = DISABLED_ALPHA * 0.5f)
                    else -> Color(0x66000000)
                },
                RoundedCornerShape(HudCorner),
            )
            .clickable(enabled = enabled) { onClick() }
            .padding(horizontal = 10.dp, vertical = 7.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            hud(text),
            style = TextStyle(
                fontFamily = HudFont,
                fontSize = 11.sp,
                letterSpacing = 0.1.sp,
                color = if (filled && enabled) HudBlack else palette.primary.copy(alpha = alpha),
            ),
            maxLines = 1,
        )
    }
}

/**
 * HUD toggle: bracketed state selector. Selected = filled amber, black text.
 */
@Composable
fun HudToggle(
    text: String,
    selected: Boolean,
    palette: HudPalette,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    onClick: () -> Unit,
) {
    val alpha = if (enabled) 1f else DISABLED_ALPHA
    Box(
        modifier = modifier
            .border(
                HudStroke,
                if (selected) palette.primary else palette.primary.copy(alpha = alpha * 0.55f),
                RoundedCornerShape(HudCorner),
            )
            .background(
                when {
                    selected -> palette.primary.copy(alpha = 0.85f)
                    else -> Color(0x66000000)
                },
                RoundedCornerShape(HudCorner),
            )
            .clickable(enabled = enabled) { onClick() }
            .padding(horizontal = 8.dp, vertical = 6.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            (if (selected) "▸ " else "") + hud(text),
            style = TextStyle(
                fontFamily = HudFont,
                fontSize = 10.sp,
                letterSpacing = 0.1.sp,
                color = if (selected) HudBlack else palette.primary.copy(alpha = alpha),
            ),
            maxLines = 1,
        )
    }
}
