package dev.randozart.lares.ui.hud

import androidx.compose.foundation.Canvas
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.text.TextLayoutResult
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

/** One drawable target with its measured data block. */
private data class TargetCallout(
    val id: String,
    val chore: ChoreEntity,
    val rect: Rect,
    val engaged: Boolean,
    val color: Color,
    val lines: List<TextLayoutResult>,
    val blockSize: Size,
)

/** Final callout placement: packed block position plus its box anchor. */
private data class CalloutPlacement(
    val target: TargetCallout,
    val blockTopLeft: Offset,
    val blockEdgeCenter: Offset,
    val anchor: Offset,
)

/**
 * Avionics-style target overlay: corner-bracket designators, leader lines
 * to telegraphic data blocks (packed per side, never overlapping), dim
 * NAV-aid landmarks, boresight, and the kill-collapse animation.
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

        val targets = boxes.mapNotNull { box ->
            val chore = choresById[box.id] ?: return@mapNotNull null
            val retired = chore.status == dev.randozart.lares.proto.ChoreStatus.CHORE_STATUS_DONE ||
                chore.status == dev.randozart.lares.proto.ChoreStatus.CHORE_STATUS_DISMISSED
            if (retired && box.id != killId) return@mapNotNull null
            if (box.confidence < MIN_CONFIDENCE && box.id != killId) return@mapNotNull null
            val topLeft = transform.toScreen(box.xmin, box.ymin, frameW.toFloat(), frameH.toFloat())
            val bottomRight = transform.toScreen(box.xmax, box.ymax, frameW.toFloat(), frameH.toFloat())
            val engaged = box.id == engagedId
            val alpha = box.confidence.coerceIn(MIN_CONFIDENCE, 1f)
            TargetCallout(
                id = box.id,
                chore = chore,
                rect = Rect(topLeft, bottomRight),
                engaged = engaged,
                color = palette.primary.copy(alpha = if (engaged) 1f else alpha),
                lines = emptyList(),
                blockSize = Size.Zero,
            )
        }

        // Designators (and the kill effect) first, under the callouts.
        targets.forEach { target ->
            when {
                target.id == killId -> drawKill(target.rect, killEffect!!, palette, textMeasurer)
                target.engaged -> drawDesignator(target.rect, target.color, solid = true, pulse = true)
                else -> drawDesignator(target.rect, target.color, solid = false, pulse = false)
            }
        }

        // Measure, pack, and draw callouts for live (non-killed) targets.
        val live = targets.filter { it.id != killId }
        val measured = live.map { target ->
            val (lines, blockSize) = measureBlock(target.chore, target.color, target.engaged, textMeasurer)
            target.copy(lines = lines, blockSize = blockSize)
        }
        val placements = packCallouts(measured)
        placements.forEach { placement -> drawCallout(placement, textMeasurer) }

        landmarks.forEach { landmark ->
            if (!landmark.hasBox()) return@forEach
            val b = landmark.box
            val topLeft = transform.toScreen(b.xmin.toFloat(), b.ymin.toFloat(), frameW.toFloat(), frameH.toFloat())
            val bottomRight = transform.toScreen(b.xmax.toFloat(), b.ymax.toFloat(), frameW.toFloat(), frameH.toFloat())
            drawNavAid(Rect(topLeft, bottomRight), palette.dim, landmark.label, textMeasurer)
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

/** Draw one target: corner brackets. */
private fun DrawScope.drawDesignator(rect: Rect, color: Color, solid: Boolean, pulse: Boolean) {
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
    drawLine(color, Offset(l, t), Offset(l + arm, t), strokeWidth = stroke)
    drawLine(color, Offset(l, t), Offset(l, t + arm), strokeWidth = stroke)
    drawLine(color, Offset(r, t), Offset(r - arm, t), strokeWidth = stroke)
    drawLine(color, Offset(r, t), Offset(r, t + arm), strokeWidth = stroke)
    drawLine(color, Offset(l, b), Offset(l + arm, b), strokeWidth = stroke)
    drawLine(color, Offset(l, b), Offset(l, b - arm), strokeWidth = stroke)
    drawLine(color, Offset(r, b), Offset(r - arm, b), strokeWidth = stroke)
    drawLine(color, Offset(r, b), Offset(r, b - arm), strokeWidth = stroke)
}

/** Measure the telegraphic data block for one target. */
private fun DrawScope.measureBlock(
    chore: ChoreEntity,
    color: Color,
    engaged: Boolean,
    textMeasurer: androidx.compose.ui.text.TextMeasurer,
): Pair<List<TextLayoutResult>, Size> {
    val index = chore.objectIndex.takeIf { it > 0 } ?: 1
    val est = chore.estimatedSeconds.takeIf { it > 0 } ?: 30
    val lines = buildList {
        add("TGT %02d: %s".format(index, chore.target.uppercase()))
        add("DIR: %s".format(chore.action.uppercase()))
        add("EST: %02d:%02d".format(est / 60, est % 60))
        if (engaged) add("LOCK: SOLID")
    }
    val style = TextStyle(
        fontFamily = HudFont,
        fontSize = 9.sp,
        color = color,
        letterSpacing = 0.1.sp,
    )
    val measured = lines.map { textMeasurer.measure(it, style) }
    val gapPx = 1.dp.toPx()
    val width = measured.maxOf { it.size.width }
    val height = measured.sumOf { it.size.height } + (measured.size - 1) * gapPx
    return measured to Size(width.toFloat(), height.toFloat())
}

/**
 * Pack callout blocks on each side of the screen, monotonically top-down,
 * clamped inside the chrome insets so blocks never overlap or leave view.
 */
private fun DrawScope.packCallouts(targets: List<TargetCallout>): List<CalloutPlacement> {
    val gap = 6.dp.toPx()
    val topInset = 60.dp.toPx()
    val bottomInset = 96.dp.toPx()
    val lead = 16.dp.toPx()
    val cursors = mutableMapOf(false to topInset, true to topInset)
    val placements = mutableListOf<CalloutPlacement>()
    for (target in targets.sortedBy { it.rect.center.y }) {
        val onRight = target.rect.center.x < size.width / 2f
        val desired = target.rect.center.y - target.blockSize.height / 2f
        val maxY = (size.height - bottomInset - target.blockSize.height).coerceAtLeast(topInset)
        val y = desired.coerceIn(topInset, maxY).coerceAtLeast(cursors[onRight] ?: topInset)
        cursors[onRight] = y + target.blockSize.height + gap
        val blockX = if (onRight) {
            (target.rect.right + lead).coerceAtMost(size.width - target.blockSize.width - 4.dp.toPx())
        } else {
            (target.rect.left - lead - target.blockSize.width).coerceAtLeast(4.dp.toPx())
        }
        val blockTop = Offset(blockX, y)
        val edgeCenter = if (onRight) {
            Offset(blockX, y + target.blockSize.height / 2f)
        } else {
            Offset(blockX + target.blockSize.width, y + target.blockSize.height / 2f)
        }
        val anchor = if (onRight) {
            Offset(target.rect.right.coerceAtMost(size.width), target.rect.center.y)
        } else {
            Offset(target.rect.left.coerceAtLeast(0f), target.rect.center.y)
        }
        placements.add(CalloutPlacement(target, blockTop, edgeCenter, anchor))
    }
    return placements
}

/** Draw the leader line from the bracket to the block, then the block. */
private fun DrawScope.drawCallout(
    placement: CalloutPlacement,
    textMeasurer: androidx.compose.ui.text.TextMeasurer,
) {
    val target = placement.target
    drawLine(target.color, placement.anchor, placement.blockEdgeCenter, strokeWidth = HudFineStroke.toPx())
    drawCircle(target.color, radius = 2.dp.toPx(), center = placement.anchor)
    var y = placement.blockTopLeft.y
    val x = placement.blockTopLeft.x
    target.lines.forEach { line ->
        drawRect(
            color = Color(0x66000000),
            topLeft = Offset(x - 2.dp.toPx(), y - 1.dp.toPx()),
            size = Size(line.size.width + 4.dp.toPx(), line.size.height + 2.dp.toPx()),
        )
        drawText(line, topLeft = Offset(x, y))
        y += line.size.height + 1.dp.toPx()
    }
}

/** Draw the kill-collapse animation: brackets converge on the centroid. */
private fun DrawScope.drawKill(
    rect: Rect,
    effect: KillEffect,
    palette: HudPalette,
    textMeasurer: androidx.compose.ui.text.TextMeasurer,
) {
    val p = effect.progress.coerceIn(0f, 1f)
    val collapse = Rect(
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
    rect: Rect,
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
