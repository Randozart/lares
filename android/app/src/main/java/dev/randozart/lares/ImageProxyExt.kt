package dev.randozart.lares

import androidx.camera.core.ImageProxy
import uniffi.lares_tracking.GrayFrame

/**
 * Convert a CameraX frame's Y plane into a tracker gray frame, rotated to
 * display orientation so tracker coordinates match the preview and the VLM's
 * upright JPEG.
 */
fun ImageProxy.toGrayFrame(): GrayFrame {
    val plane = planes[0]
    val buffer = plane.buffer
    val bytes = ByteArray(buffer.remaining()).also { buffer.get(it) }
    val w = width
    val h = height
    val stride = plane.rowStride
    val rotated = rotateY(bytes, w, h, stride, imageInfo.rotationDegrees)
    return GrayFrame(
        width = rotated.first.toUInt(),
        height = rotated.second.toUInt(),
        rowStride = rotated.first.toUInt(),
        data = rotated.third,
    )
}

/** Rotate grayscale bytes by the given degrees (0/90/180/270). */
private fun rotateY(bytes: ByteArray, w: Int, h: Int, stride: Int, degrees: Int): Triple<Int, Int, ByteArray> {
    val src = fun(r: Int, c: Int): Byte = bytes.getOrElse(r * stride + c) { 0 }
    return when (degrees) {
        90 -> {
            val ow = h
            val oh = w
            val out = ByteArray(ow * oh)
            for (i in 0 until oh) {
                for (j in 0 until ow) {
                    out[i * ow + j] = src(h - 1 - j, i)
                }
            }
            Triple(ow, oh, out)
        }
        180 -> {
            val out = ByteArray(w * h)
            for (i in 0 until h) {
                for (j in 0 until w) {
                    out[i * w + j] = src(h - 1 - i, w - 1 - j)
                }
            }
            Triple(w, h, out)
        }
        270 -> {
            val ow = h
            val oh = w
            val out = ByteArray(ow * oh)
            for (i in 0 until oh) {
                for (j in 0 until ow) {
                    out[i * ow + j] = src(j, w - 1 - i)
                }
            }
            Triple(ow, oh, out)
        }
        else -> Triple(w, h, bytes)
    }
}