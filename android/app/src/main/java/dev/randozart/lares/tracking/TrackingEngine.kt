package dev.randozart.lares.tracking

import android.os.SystemClock
import uniffi.lares_tracking.GrayFrame
import uniffi.lares_tracking.ImagePatch
import uniffi.lares_tracking.LaresTracker
import uniffi.lares_tracking.TrackedBox
import java.nio.ByteBuffer

/** Milliseconds between tracker updates while frames stream in. */
private const val TRACK_INTERVAL_MS = 66L

/**
 * Kotlin facade over the Rust `lares-tracking` tracker (UniFFI).
 *
 * Owns the Rust object, throttles per-frame tracking, and keeps the last gray
 * frame for anchoring and fingerprinting.
 */
class TrackingEngine(subsample: UInt = 2u) {
    private val tracker = LaresTracker(subsample)

    /** The most recent gray frame, used as the anchor on a VLM response. */
    @Volatile
    var lastFrame: GrayFrame? = null
        private set

    /** The latest tracked boxes (normalized 0..1000). */
    @Volatile
    var boxes: List<TrackedBox> = emptyList()
        private set

    private var lastTrackAt = 0L

    /** Feed a new frame; throttled tracking updates [boxes]. */
    fun onFrame(frame: GrayFrame) {
        lastFrame = frame
        val now = SystemClock.elapsedRealtime()
        if (now - lastTrackAt < TRACK_INTERVAL_MS) return
        lastTrackAt = now
        boxes = tracker.track(frame)
    }

    /** Anchor the tracker to a frame and box set (typically the VLM response). */
    fun anchor(frame: GrayFrame, boxes: List<TrackedBox>) {
        tracker.anchor(frame, boxes)
        this.boxes = boxes
    }

    /** Compute the 8-byte scene fingerprint for a frame. */
    fun fingerprint(frame: GrayFrame): ByteArray = tracker.fingerprint(frame)

    /** Hamming distance between two fingerprints (0 = identical scene). */
    fun fingerprintDistance(a: ByteArray, b: ByteArray): UInt =
        tracker.fingerprintDistance(ByteBuffer.wrap(a), ByteBuffer.wrap(b))

    /** Crop a landmark patch from a frame. */
    fun patch(frame: GrayFrame, box: TrackedBox): ImagePatch = tracker.patch(frame, box)

    /** Drop all anchored patches. */
    fun clear() {
        tracker.clear()
        boxes = emptyList()
    }
}