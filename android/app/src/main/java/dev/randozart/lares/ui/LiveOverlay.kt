package dev.randozart.lares.ui

import androidx.compose.foundation.Canvas
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.unit.dp
import dev.randozart.lares.proto.ChoreEntity
import dev.randozart.lares.proto.ChoreStatus
import dev.randozart.lares.proto.Landmark
import uniffi.lares_tracking.TrackedBox

/** Accent color for active chore boxes. */
private val BoxColor = Color(0xFF66BB6A)
/** Color for completed/dismissed boxes. */
private val MutedColor = Color(0xFF9E9E9E)
/** Landmark box color. */
private val LandmarkColor = Color(0xFF42A5F5)

/**
 * Draws tracked boxes and landmarks over the live preview. Frames are rotated
 * to display orientation before tracking, so normalized (0..1000) coordinates
 * map directly onto the canvas.
 */
@Composable
fun LiveOverlay(
    boxes: List<TrackedBox>,
    choresById: Map<String, ChoreEntity>,
    landmarks: List<Landmark>,
    modifier: Modifier = Modifier,
) {
    Canvas(modifier = modifier) {
        boxes.forEach { box ->
            val chore = choresById[box.id]
            val muted = chore?.let {
                it.status == ChoreStatus.CHORE_STATUS_DONE ||
                    it.status == ChoreStatus.CHORE_STATUS_DISMISSED
            } == true
            val base = if (muted) MutedColor else BoxColor
            val alpha = box.confidence.coerceIn(0.12f, 1f)
            drawBox(box, base.copy(alpha = alpha), chore?.action)
        }
        landmarks.forEach { landmark ->
            if (landmark.hasBox()) {
                val b = landmark.box
                drawNormBox(
                    b.ymin.toFloat(), b.xmin.toFloat(),
                    b.ymax.toFloat(), b.xmax.toFloat(),
                    LandmarkColor, landmark.label,
                )
            }
        }
    }
}

/** Draw a tracked box scaled into the canvas with an optional label. */
private fun DrawScope.drawBox(box: TrackedBox, color: Color, label: String?) {
    drawNormBox(box.ymin, box.xmin, box.ymax, box.xmax, color, label)
}

/** Draw a normalized (0..1000) box scaled into the canvas. */
private fun DrawScope.drawNormBox(
    ymin: Float,
    xmin: Float,
    ymax: Float,
    xmax: Float,
    color: Color,
    label: String?,
) {
    val left = xmin / 1000f * size.width
    val top = ymin / 1000f * size.height
    val right = xmax / 1000f * size.width
    val bottom = ymax / 1000f * size.height
    drawRect(
        color = color,
        topLeft = Offset(left, top),
        size = Size(right - left, bottom - top),
        style = Stroke(width = 3.dp.toPx()),
    )
    if (label != null && label.isNotEmpty()) {
        drawLabel(label, left, top)
    }
}

/** Draw a small label chip above a box. */
private fun DrawScope.drawLabel(text: String, left: Float, top: Float) {
    val width = (text.length * 7).dp.toPx()
    val height = 18.dp.toPx()
    drawRect(
        color = Color(0xCC000000),
        topLeft = Offset(left, top - height),
        size = Size(width, height),
    )
    drawContext.canvas.nativeCanvas.apply {
        val paint = android.graphics.Paint().apply {
            this.color = 0xFFFFFFFF.toInt()
            textSize = 12.dp.toPx()
        }
        drawText(text, left + 3.dp.toPx(), top - 4.dp.toPx(), paint)
    }
}