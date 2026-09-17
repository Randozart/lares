package dev.randozart.lares.ui.hud

import androidx.compose.ui.geometry.Offset
import kotlin.math.abs

/**
 * The PreviewView.FILL_CENTER transform between camera-frame normalized
 * coordinates (0..1000) and screen pixels.
 *
 * Shared by the overlay draw and the tap hit-test so both agree exactly.
 */
data class HudTransform(
    val scale: Float,
    val offsetX: Float,
    val offsetY: Float,
) {
    /** Map a normalized (0..1000) point to screen pixels. */
    fun toScreen(xNorm: Float, yNorm: Float, frameW: Float, frameH: Float): Offset {
        val x = xNorm / 1000f * frameW * scale + offsetX
        val y = yNorm / 1000f * frameH * scale + offsetY
        return Offset(x, y)
    }

    /** Map a screen pixel point back to normalized (0..1000). */
    fun toNorm(screenX: Float, screenY: Float, frameW: Float, frameH: Float): Offset {
        val x = (screenX - offsetX) / (frameW * scale) * 1000f
        val y = (screenY - offsetY) / (frameH * scale) * 1000f
        return Offset(x, y)
    }
}

/** Compute the FILL_CENTER transform for the given screen and frame sizes. */
fun hudTransform(screenW: Float, screenH: Float, frameW: Int, frameH: Int): HudTransform {
    val fw = frameW.coerceAtLeast(1).toFloat()
    val fh = frameH.coerceAtLeast(1).toFloat()
    val screenRatio = screenW / screenH
    val frameRatio = fw / fh
    return if (frameRatio > screenRatio) {
        val scale = screenH / fh
        HudTransform(scale, (screenW - fw * scale) / 2f, 0f)
    } else {
        val scale = screenW / fw
        HudTransform(scale, 0f, (screenH - fh * scale) / 2f)
    }
}

/** Whether a normalized point falls inside a normalized box (0..1000). */
fun pointInBox(x: Float, y: Float, xmin: Float, ymin: Float, xmax: Float, ymax: Float): Boolean {
    return x >= xmin && x <= xmax && y >= ymin && y <= ymax && abs(x) <= 2000f && abs(y) <= 2000f
}
