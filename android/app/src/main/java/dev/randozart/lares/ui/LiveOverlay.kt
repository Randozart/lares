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
 * Draws tracked boxes and landmarks over the live preview. Accounts for the
 * PreviewView.ScaleType.FILL_CENTER transform by computing the scale and crop
 * offset between the camera analysis frame and the screen canvas.
 */
@Composable
fun LiveOverlay(
    boxes: List<TrackedBox>,
    choresById: Map<String, ChoreEntity>,
    landmarks: List<Landmark>,
    frameW: Int,
    frameH: Int,
    modifier: Modifier = Modifier,
) {
    Canvas(modifier = modifier) {
        // Compute FILL_CENTER transform: camera frame → screen canvas.
        val fw = frameW.coerceAtLeast(1).toFloat()
        val fh = frameH.coerceAtLeast(1).toFloat()
        val screenRatio = size.width / size.height
        val frameRatio = fw / fh
        val scale: Float
        val offsetX: Float
        val offsetY: Float
        if (frameRatio > screenRatio) {
            // Frame wider than screen → crop left/right.
            scale = size.height / fh
            offsetX = (size.width - fw * scale) / 2f
            offsetY = 0f
        } else {
            // Frame taller than screen → crop top/bottom.
            scale = size.width / fw
            offsetX = 0f
            offsetY = (size.height - fh * scale) / 2f
        }

        boxes.filter { it.confidence >= 0.15f }.forEach { box ->
            val chore = choresById[box.id]
            val muted = chore?.let {
                it.status == ChoreStatus.CHORE_STATUS_DONE ||
                    it.status == ChoreStatus.CHORE_STATUS_DISMISSED
            } == true
            val base = if (muted) MutedColor else BoxColor
            val alpha = box.confidence.coerceIn(0.15f, 1f)
            drawBox(box, base.copy(alpha = alpha), chore?.action, fw, fh, scale, offsetX, offsetY)
        }
        landmarks.forEach { landmark ->
            if (landmark.hasBox()) {
                val b = landmark.box
                drawNormBox(
                    b.ymin.toFloat(), b.xmin.toFloat(),
                    b.ymax.toFloat(), b.xmax.toFloat(),
                    LandmarkColor, landmark.label,
                    fw, fh, scale, offsetX, offsetY,
                )
            }
        }
    }
}

/** Draw a tracked box scaled into the canvas with an optional label. */
private fun DrawScope.drawBox(
    box: TrackedBox,
    color: Color,
    label: String?,
    fw: Float,
    fh: Float,
    scale: Float,
    offsetX: Float,
    offsetY: Float,
) {
    drawNormBox(box.ymin, box.xmin, box.ymax, box.xmax, color, label, fw, fh, scale, offsetX, offsetY)
}

/** Draw a normalized (0..1000) box scaled into the canvas with FILL_CENTER transform. */
private fun DrawScope.drawNormBox(
    ymin: Float,
    xmin: Float,
    ymax: Float,
    xmax: Float,
    color: Color,
    label: String?,
    fw: Float,
    fh: Float,
    scale: Float,
    offsetX: Float,
    offsetY: Float,
) {
    val left = xmin / 1000f * fw * scale + offsetX
    val top = ymin / 1000f * fh * scale + offsetY
    val right = xmax / 1000f * fw * scale + offsetX
    val bottom = ymax / 1000f * fh * scale + offsetY
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