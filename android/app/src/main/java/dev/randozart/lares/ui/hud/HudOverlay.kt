package dev.randozart.lares.ui.hud

import androidx.compose.foundation.Canvas
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.drawText
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.randozart.lares.proto.ChoreEntity
import dev.randozart.lares.proto.Landmark
import uniffi.lares_tracking.TrackedBox

/** An in-progress kill animation for one target. */
data class KillEffect(
    /** The chore id being neutralized. */
    val choreId: String,
    /** Target label for the SPLASH line. */
    val target: String,
    /** Animation progress, 0..1. */
    val progress: Float,
)

/** Fraction of confidence below which targets are not painted. */
private const val MIN_CONFIDENCE = 0.15f

/**
 * Avionics-style target overlay: corner-bracket designators, centroid
 * diamonds, leader lines to telegraphic target data blocks, dim NAV-aid
 * landmarks, and the kill-collapse animation.
 */
@Composable
fun HudOverlay(
    boxes: List<TrackedBox>,
    choresById: Map<String, ChoreEntity>,
    landmarks: List<Landmark>,
    frameW: Int,
    frameH: Int,
    palette: HudPalette,
    engagedId: String?,
    killEffect: KillEffect?,
    modifier: Modifier = Modifier,
) {
    val textMeasurer = rememberTextMeasurer()
    Canvas(modifier = modifier) {
        val transform = hudTransform(size.width, size.height, frameW, frameH)
        val killId = killEffect?.choreId
        drawBoresight(palette.dim)
        boxes.forEach { box ->
            val chore = choresById[box.id] ?: return@forEach
            // Done chores leave the overlay; the kill effect draws instead.
            val retired = chore.status == dev.randozart.lares.proto.ChoreStatus.CHORE_STATUS_DONE ||
                chore.status == dev.randozart.lares.proto.ChoreStatus.CHORE_STATUS_DISMISSED
            if (retired && box.id != killId) return@forEach
            if (box.confidence < MIN_CONFIDENCE && box.id != killId) return@forEach
            val topLeft = transform.toScreen(box.xmin, box.ymin, frameW.toFloat(), frameH.toFloat())
            val bottomRight = transform.toScreen(box.xmax, box.ymax, frameW.toFloat(), frameH.toFloat())
            val rect = androidx.compose.ui.geometry.Rect(topLeft, bottomRight)
            val engaged = box.id == engagedId
            val alpha = box.confidence.coerceIn(MIN_CONFIDENCE, 1f)
            val color = palette.primary.copy(alpha = if (engaged) 1f else alpha)
            when {
                box.id == killId -> drawKill(rect, killEffect!!, palette, textMeasurer)
                engaged -> drawDesignator(rect, color, solid = true, pulse = true)
                else -> drawDesignator(rect, color, solid = false, pulse = false)
            }
            if (box.id != killId) {
                drawDataBlock(rect, chore, color, engaged, textMeasurer)
            }
        }
        landmarks.forEach { landmark ->
            if (!landmark.hasBox()) return@forEach
            val b = landmark.box
            val topLeft = transform.toScreen(b.xmin.toFloat(), b.ymin.toFloat(), frameW.toFloat(), frameH.toFloat())
            val bottomRight = transform.toScreen(b.xmax.toFloat(), b.ymax.toFloat(), frameW.toFloat(), frameH.toFloat())
            val rect = androidx.compose.ui.geometry.Rect(topLeft, bottomRight)
            drawNavAid(rect, palette.dim, landmark.label, textMeasurer)
        }
    }
}

/** Draw the center boresight cross: aim reference when nothing is locked. */
private fun DrawScope.drawBoresight(color: Color) {
    val c = Offset(size.width / 2f, size.height / 2f)
    val arm = 10.dp.toPx()
    val gap = 3.dp.toPx()
    drawLine(color, Offset(c.x - arm, c.y), Offset(c.x - gap, c.y), strokeWidth = HudFineStroke.toPx())
    drawLine(color, Offset(c.x + gap, c.y), Offset(c.x + arm, c.y), strokeWidth = HudFineStroke.toPx())
    drawLine(color, Offset(c.x, c.y - arm), Offset(c.x, c.y - gap), strokeWidth = HudFineStroke.toPx())
    drawLine(color, Offset(c.x, c.y + gap), Offset(c.x, c.y + arm), strokeWidth = HudFineStroke.toPx())
}

/** Draw one target: corner brackets plus a centroid diamond. */
private fun DrawScope.drawDesignator(rect: androidx.compose.ui.geometry.Rect, color: Color, solid: Boolean, pulse: Boolean) {
    val arm = (rect.minDimension * 0.14f).coerceAtMost(18.dp.toPx()).coerceAtLeast(7.dp.toPx())
    val stroke = HudStroke.toPx()
    val inset = if (pulse) {
        2.dp.toPx() * (1f + 0.06f * kotlin.math.sin(System.nanoTime() / 100_000_000.0).toFloat())
    } else {
        0f
    }
    val l = rect.left + inset
    val t = rect.top + inset
    val r = rect.right - inset
    val b = rect.bottom - inset
    // Corner brackets: two strokes per corner.
    drawLine(color, Offset(l, t), Offset(l + arm, t), strokeWidth = stroke)
    drawLine(color, Offset(l, t), Offset(l, t + arm), strokeWidth = stroke)
    drawLine(color, Offset(r, t), Offset(r - arm, t), strokeWidth = stroke)
    drawLine(color, Offset(r, t), Offset(r, t + arm), strokeWidth = stroke)
    drawLine(color, Offset(l, b), Offset(l + arm, b), strokeWidth = stroke)
    drawLine(color, Offset(l, b), Offset(l, b - arm), strokeWidth = stroke)
    drawLine(color, Offset(r, b), Offset(r - arm, b), strokeWidth = stroke)
    drawLine(color, Offset(r, b), Offset(r, b - arm), strokeWidth = stroke)
    drawDiamond(rect.center, arm * 0.28f, color)
}

/** Draw the centroid diamond marker. */
private fun DrawScope.drawDiamond(center: Offset, radius: Float, color: Color) {
    val path = Path().apply {
        moveTo(center.x, center.y - radius)
        lineTo(center.x + radius, center.y)
        lineTo(center.x, center.y + radius)
        lineTo(center.x - radius, center.y)
        close()
    }
    drawPath(path, color, style = Stroke(width = HudStroke.toPx() * 0.75f))
}

/** Draw the leader line and telegraphic target data block. */
private fun DrawScope.drawDataBlock(
    rect: androidx.compose.ui.geometry.Rect,
    chore: ChoreEntity,
    color: Color,
    engaged: Boolean,
    textMeasurer: androidx.compose.ui.text.TextMeasurer,
) {
    val index = chore.objectIndex.takeIf { it > 0 } ?: 1
    val blockOnRight = rect.center.x < size.width / 2f
    val anchor = if (blockOnRight) {
        Offset(rect.right, rect.center.y)
    } else {
        Offset(rect.left, rect.center.y)
    }
    val lead = 18.dp.toPx()
    val lineEnd = if (blockOnRight) anchor + Offset(lead, 0f) else anchor - Offset(lead, 0f)
    drawLine(color, anchor, lineEnd, strokeWidth = HudFineStroke.toPx())
    val style = TextStyle(
        fontFamily = HudFont,
        fontSize = 9.sp,
        color = color,
        letterSpacing = 0.1.sp,
    )
    val est = chore.estimatedSeconds.takeIf { it > 0 } ?: 30
    val lines = buildList {
        add("TGT %02d: %s".format(index, chore.target.uppercase()))
        add("DIR: %s".format(chore.action.uppercase()))
        add("EST: %02d:%02d".format(est / 60, est % 60))
        if (engaged) add("LOCK: SOLID")
    }
    var y = lineEnd.y - lines.size * 12.dp.toPx() / 2f
    val blockX = if (blockOnRight) lineEnd.x + 4.dp.toPx() else lineEnd.x - 4.dp.toPx()
    lines.forEach { line ->
        val measured = textMeasurer.measure(line, style)
        val x = if (blockOnRight) blockX else blockX - measured.size.width
        // Backing rect for readability over the camera feed.
        drawRect(
            color = Color(0x66000000),
            topLeft = Offset(x - 2.dp.toPx(), y - 1.dp.toPx()),
            size = Size(measured.size.width + 4.dp.toPx(), measured.size.height + 2.dp.toPx()),
        )
        drawText(measured, topLeft = Offset(x, y))
        y += measured.size.height + 1.dp.toPx()
    }
}

/** Draw the kill-collapse animation: brackets converge on the centroid. */
private fun DrawScope.drawKill(
    rect: androidx.compose.ui.geometry.Rect,
    effect: KillEffect,
    palette: HudPalette,
    textMeasurer: androidx.compose.ui.text.TextMeasurer,
) {
    val p = effect.progress.coerceIn(0f, 1f)
    val collapse = androidx.compose.ui.geometry.Rect(
        rect.left + rect.width * 0.45f * p,
        rect.top + rect.height * 0.45f * p,
        rect.right - rect.width * 0.45f * p,
        rect.bottom - rect.height * 0.45f * p,
    )
    val flash = if (p in 0.40f..0.60f) palette.primary else palette.primary.copy(alpha = 1f - p)
    drawDesignator(collapse, flash, solid = true, pulse = false)
    val style = TextStyle(
        fontFamily = HudFont,
        fontSize = 11.sp,
        color = palette.primary,
    )
    val splash = "SPLASH 1 // ${effect.target.uppercase()} NEUTRALIZED"
    val measured = textMeasurer.measure(splash, style)
    val pos = Offset(
        (size.width - measured.size.width) / 2f,
        rect.center.y - measured.size.height / 2f,
    )
    drawRect(
        color = Color(0xB3000000),
        topLeft = Offset(pos.x - 4.dp.toPx(), pos.y - 2.dp.toPx()),
        size = Size(measured.size.width + 8.dp.toPx(), measured.size.height + 4.dp.toPx()),
    )
    drawText(measured, topLeft = pos)
}

/** Draw a dim dashed NAV-aid marker with a label. */
private fun DrawScope.drawNavAid(
    rect: androidx.compose.ui.geometry.Rect,
    color: Color,
    label: String,
    textMeasurer: androidx.compose.ui.text.TextMeasurer,
) {
    val dashed = PathEffect.dashPathEffect(floatArrayOf(10f, 8f))
    drawRect(
        color = color,
        topLeft = Offset(rect.left, rect.top),
        size = Size(rect.width, rect.height),
        style = Stroke(width = HudFineStroke.toPx(), pathEffect = dashed),
    )
    val style = TextStyle(fontFamily = HudFont, fontSize = 9.sp, color = color)
    val measured = textMeasurer.measure("NAV AID: ${label.uppercase()}", style)
    drawText(measured, topLeft = Offset(rect.left + 4.dp.toPx(), rect.top + 4.dp.toPx()))
}
