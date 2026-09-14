package dev.randozart.lares.annotate

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import dev.randozart.lares.proto.ChoreEntity

/** Chore box outline color. */
private val BoxColor = Color.rgb(0x66, 0xBB, 0x6A)

/**
 * Compose a snapshot image: the frame with the chore's bounding box drawn and
 * a how-to panel along the bottom. Returns a new bitmap; the input is untouched.
 */
fun annotateSnapshot(frame: Bitmap, chore: ChoreEntity): Bitmap {
    val out = frame.copy(Bitmap.Config.ARGB_8888, true)
    val canvas = Canvas(out)
    if (chore.hasBox()) {
        val box = chore.box
        drawBox(
            canvas,
            box.ymin.toFloat(),
            box.xmin.toFloat(),
            box.ymax.toFloat(),
            box.xmax.toFloat(),
            out.width.toFloat(),
            out.height.toFloat(),
        )
    }
    drawStepsPanel(canvas, out.width, chore)
    return out
}

/** Draw the normalized chore box outline. */
private fun drawBox(
    canvas: Canvas,
    ymin: Float,
    xmin: Float,
    ymax: Float,
    xmax: Float,
    width: Float,
    height: Float,
) {
    val left = xmin / 1000f * width
    val top = ymin / 1000f * height
    val right = xmax / 1000f * width
    val bottom = ymax / 1000f * height
    val paint = Paint().apply {
        color = BoxColor
        style = Paint.Style.STROKE
        strokeWidth = 8f
    }
    canvas.drawRect(left, top, right, bottom, paint)
}

/** Draw a translucent how-to panel along the bottom of the image. */
private fun drawStepsPanel(canvas: Canvas, width: Int, chore: ChoreEntity) {
    val steps = chore.howToList.ifEmpty { listOf(chore.action) }
    val titlePaint = Paint().apply {
        color = Color.WHITE
        textSize = 34f
        isFakeBoldText = true
    }
    val textPaint = Paint().apply { color = Color.WHITE; textSize = 30f }
    val lineHeight = 44f
    val titleHeight = 52f
    val wrappedSteps = steps.flatMap { wrapText(it, textPaint, width - 48) }
    val panelHeight = (titleHeight + wrappedSteps.size * lineHeight + 24f).toInt()
    val panelTop = canvas.height - panelHeight
    canvas.drawRect(0f, panelTop.toFloat(), width.toFloat(), canvas.height.toFloat(), Paint().apply {
        color = Color.argb(210, 0, 0, 0)
    })
    canvas.drawText(chore.action, 24f, panelTop + titleHeight - 14f, titlePaint)
    wrappedSteps.forEachIndexed { index, line ->
        val y = panelTop + titleHeight + index * lineHeight + 10f
        canvas.drawText(line, 24f, y, textPaint)
    }
}

/** Split a line into wrapped fragments that fit the max width. */
private fun wrapText(text: String, paint: Paint, maxWidth: Int): List<String> {
    if (paint.measureText(text) <= maxWidth) {
        return listOf(text)
    }
    val words = text.split(" ")
    val lines = mutableListOf<String>()
    val current = StringBuilder()
    for (word in words) {
        val candidate = if (current.isEmpty()) word else "$current $word"
        if (paint.measureText(candidate) <= maxWidth) {
            current.clear()
            current.append(candidate)
        } else {
            if (current.isNotEmpty()) {
                lines.add(current.toString())
                current.clear()
            }
            lines.add(word)
        }
    }
    if (current.isNotEmpty()) {
        lines.add(current.toString())
    }
    return lines
}