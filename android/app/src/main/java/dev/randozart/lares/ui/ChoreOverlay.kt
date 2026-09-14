package dev.randozart.lares.ui

import android.graphics.Bitmap
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.dp
import dev.randozart.lares.proto.ChoreEntity
import dev.randozart.lares.proto.ChoreStatus

/** Accent color for chore boxes. */
private val BoxColor = Color(0xFF66BB6A)
/** Color for completed/dismissed chore boxes. */
private val MutedColor = Color(0xFF9E9E9E)

/**
 * The frozen frame with normalized chore boxes drawn over it.
 *
 * Coordinates arrive normalized to 0..1000 and are scaled to the displayed
 * image; the box aspect ratio is preserved so scaling is linear.
 */
@Composable
fun ChoreOverlay(
    bitmap: Bitmap,
    chores: List<ChoreEntity>,
    modifier: Modifier = Modifier,
) {
    val active = chores.filter { it.status != ChoreStatus.CHORE_STATUS_DONE &&
        it.status != ChoreStatus.CHORE_STATUS_DISMISSED }
    val done = chores.filter { it.status == ChoreStatus.CHORE_STATUS_DONE ||
        it.status == ChoreStatus.CHORE_STATUS_DISMISSED }

    Box(
        modifier = modifier.aspectRatio(bitmap.width.toFloat() / bitmap.height.toFloat()),
    ) {
        Image(
            bitmap = bitmap.asImageBitmap(),
            contentDescription = null,
            contentScale = ContentScale.Fit,
            modifier = Modifier.fillMaxSize(),
        )
        BoxOverlay(active, Modifier.fillMaxSize())
        if (done.isNotEmpty()) {
            BoxOverlay(done, Modifier.fillMaxSize(), muted = true)
        }
    }
}

/** Draw bounding boxes for the given chores scaled to the canvas. */
@Composable
private fun BoxOverlay(chores: List<ChoreEntity>, modifier: Modifier, muted: Boolean = false) {
    Canvas(modifier = modifier) {
        chores.forEach { chore ->
            if (!chore.hasBox()) return@forEach
            val box = chore.box
            val left = box.xmin / 1000f * size.width
            val top = box.ymin / 1000f * size.height
            val right = box.xmax / 1000f * size.width
            val bottom = box.ymax / 1000f * size.height
            drawRect(
                color = if (muted) MutedColor else BoxColor,
                topLeft = Offset(left, top),
                size = Size(right - left, bottom - top),
                style = Stroke(width = 4.dp.toPx()),
            )
        }
    }
}