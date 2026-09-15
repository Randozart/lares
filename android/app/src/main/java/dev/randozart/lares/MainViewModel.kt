package dev.randozart.lares

import android.content.ContentValues
import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.os.Environment
import android.os.SystemClock
import android.provider.MediaStore
import android.util.Log
import androidx.camera.core.ImageProxy
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.randozart.lares.annotate.annotateSnapshot
import dev.randozart.lares.capture.CameraController
import dev.randozart.lares.net.LaresClient
import dev.randozart.lares.proto.AnalyzeMode
import dev.randozart.lares.proto.ChoreEntity
import dev.randozart.lares.proto.ChoreStatus
import dev.randozart.lares.proto.FingerprintKind
import dev.randozart.lares.proto.Landmark
import dev.randozart.lares.proto.RoomArea
import dev.randozart.lares.tracking.TrackingEngine
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.lares_tracking.GrayFrame
import uniffi.lares_tracking.TrackedBox
import java.io.ByteArrayOutputStream

/** Maximum fingerprint distance (of 64 bits) that still counts as "unchanged". */
private const val SCENE_CHANGE_BITS = 6u
/** Minimum milliseconds between auto-scans. */
private const val MIN_SCAN_INTERVAL_MS = 5000L
/** Maximum keyframes in a sweep. */
private const val MAX_SWEEP_FRAMES = 6

/** Log tag for the fast loop debug output. */
private const val TAG = "LaresVM"

/**
 * UI state for the fast loop: live tracked boxes, the cost-guard auto-scan
 * state machine, and the chore/landmark overlay derived from the slow loop.
 */
class MainViewModel : ViewModel() {
    private val client = LaresClient()
    private val engine = TrackingEngine(2u)

    var serverUrl by mutableStateOf("http://100.111.244.0:8787")
    var roomId by mutableStateOf("kitchen")
    var roomArea by mutableStateOf(RoomArea.ROOM_AREA_KITCHEN)
    var description by mutableStateOf("counters clear, room tidy")
    var mode by mutableStateOf(AnalyzeMode.ANALYZE_MODE_DISCOVER)

    var chores by mutableStateOf<List<ChoreEntity>>(emptyList())
    var trackedBoxes by mutableStateOf<List<TrackedBox>>(emptyList())
    var landmarks by mutableStateOf<List<Landmark>>(emptyList())
    var busy by mutableStateOf(false)
    var scanning by mutableStateOf(false)
    var sweeping by mutableStateOf(false)
    var autoScan by mutableStateOf(false)
    var statusLine by mutableStateOf("ready — tap Scan or Sweep")

    /** Expected object labels for the current room, synced from the server. */
    var expectedLabels by mutableStateOf<Set<String>>(emptySet())
        private set

    /** When true, all chores are shown; when false, expected ones are suppressed. */
    var showAllChores by mutableStateOf(false)

    /** Chore lookup by tracked-box id for overlay labels. */
    val choresById = mutableStateMapOf<String, ChoreEntity>()

    /** Inline-cropped thumbnails for each chore, keyed by chore id. */
    val choreThumbnails = mutableStateMapOf<String, Bitmap>()

    private var lastFingerprint: ByteArray? = null
    private var lastScanAt = 0L

    /** Sweep keyframes (downscaled JPEGs) and their anchor frames. */
    private val sweepFrames = mutableListOf<ByteArray>()
    private val sweepAnchors = mutableListOf<GrayFrame?>()

    /** Frames received since launch, for debugging the fast loop. */
    private var frameCount = 0

    /** A captured reference still, used for the reference flow. */
    var frozenReference: Bitmap? = null
        private set

    /** The frame the latest scan was based on, for annotated snapshots. */
    var lastFrameBitmap: Bitmap? = null
        private set

    /** Dimensions of the camera analysis frame (post-rotation), for overlay mapping. */
    var lastFrameWidth = 0
        private set
    var lastFrameHeight = 0
        private set

    /** Chores filtered by expected status. Shows all when [showAllChores] is true. */
    val filteredChores: List<ChoreEntity>
        get() {
            if (showAllChores) return chores
            return chores.filter { chore ->
                val label = chore.target.trim().lowercase()
                label !in expectedLabels
            }
        }

    /** Feed a CameraX analysis frame into the tracker. */
    fun onAnalysisFrame(proxy: ImageProxy) {
        try {
            val gray = proxy.toGrayFrame()
            lastFrameWidth = gray.width.toInt()
            lastFrameHeight = gray.height.toInt()
            engine.onFrame(gray)
            trackedBoxes = engine.boxes
            frameCount += 1
            if (frameCount <= 5 || frameCount % 60 == 0) {
                Log.d(TAG, "frames=$frameCount tracked=${engine.boxes.size}")
            }
        } catch (t: Throwable) {
            Log.e(TAG, "analysis error", t)
        } finally {
            proxy.close()
        }
    }

    /** Evaluate the auto-scan cost guards (G2 scene-change, G3 rate limit). */
    fun shouldAutoScan(): Boolean {
        if (busy || sweeping) return false
        val now = SystemClock.elapsedRealtime()
        if (now - lastScanAt < MIN_SCAN_INTERVAL_MS) return false
        val frame = engine.lastFrame ?: return false
        val fingerprint = engine.fingerprint(frame)
        val previous = lastFingerprint ?: return true
        return engine.fingerprintDistance(previous, fingerprint) > SCENE_CHANGE_BITS
    }

    /** Capture and analyze; [bypassGates] skips the cost guards (manual button). */
    fun captureAndAnalyze(controller: CameraController, bypassGates: Boolean) {
        if (busy) return
        if (!bypassGates && !shouldAutoScan()) {
            statusLine = "scene unchanged, skipping scan"
            return
        }
        val anchor = engine.lastFrame
        controller.captureJpeg { jpeg ->
            if (jpeg == null) {
                statusLine = "capture failed"
                return@captureJpeg
            }
            analyze(jpeg, anchor)
        }
    }

    /** Begin a panoramic sweep: capture keyframes until stopped or full. */
    fun startSweep() {
        sweeping = true
        sweepFrames.clear()
        sweepAnchors.clear()
        statusLine = "sweep 0/$MAX_SWEEP_FRAMES — pan the room"
    }

    /** Capture one keyframe for the active sweep. */
    fun captureSweepFrame(controller: CameraController) {
        if (!sweeping) return
        controller.captureJpeg { jpeg ->
            if (jpeg == null) {
                statusLine = "capture failed"
                return@captureJpeg
            }
            sweepFrames.add(downscaleJpeg(jpeg, 1280))
            sweepAnchors.add(engine.lastFrame)
            if (sweepFrames.size >= MAX_SWEEP_FRAMES) {
                endSweep()
            } else {
                statusLine = "sweep ${sweepFrames.size}/$MAX_SWEEP_FRAMES — keep panning"
            }
        }
    }

    /** Stop the sweep and analyze all captured frames in one call. */
    fun endSweep() {
        if (!sweeping) return
        sweeping = false
        if (sweepFrames.isEmpty()) {
            statusLine = "no sweep frames"
            return
        }
        analyzeSweep()
    }

    /** Capture a frame and store it as the room's agreed target state. */
    fun captureReference(controller: CameraController) {
        controller.captureJpeg { jpeg ->
            if (jpeg == null) {
                statusLine = "capture failed"
                return@captureJpeg
            }
            frozenReference = BitmapFactory.decodeByteArray(jpeg, 0, jpeg.size)
            viewModelScope.launch {
                busy = true
                statusLine = "setting reference..."
                withContext(Dispatchers.IO) {
                    runCatching { client.setReference(serverUrl, roomId, description, jpeg) }
                        .onSuccess { statusLine = "reference saved" }
                        .onFailure { statusLine = "error: ${it.message}" }
                }
                busy = false
            }
        }
    }

    /** Transition a chore's lifecycle state on the server. */
    fun setStatus(id: String, status: ChoreStatus) {
        viewModelScope.launch {
            withContext(Dispatchers.IO) {
                runCatching { client.setStatus(serverUrl, id, status) }
                    .onSuccess { refreshChores() }
                    .onFailure { statusLine = "error: ${it.message}" }
            }
        }
    }

    /** Reload the persisted chore list and landmark set from the server. */
    fun refreshChores() {
        viewModelScope.launch {
            withContext(Dispatchers.IO) {
                runCatching { client.listChores(serverUrl, roomId) }
                    .onSuccess { chores = it; rebuildChoreIndex(it) }
                    .onFailure { statusLine = "error: ${it.message}" }
            }
            withContext(Dispatchers.IO) {
                runCatching { client.getLandmarks(serverUrl, roomId) }
                    .onSuccess { landmarks = it }
                    .onFailure { }
            }
        }
    }

    /** Send the captured frame to the slow loop and store the response. */
    private fun analyze(jpeg: ByteArray, anchor: GrayFrame?) {
        viewModelScope.launch {
            busy = true
            scanning = true
            statusLine = "analyzing..."
            withContext(Dispatchers.IO) {
                val upload = downscaleJpeg(jpeg, 1280)
                lastFrameBitmap = BitmapFactory.decodeByteArray(upload, 0, upload.size)
                runCatching { client.analyze(serverUrl, roomId, upload, mode, roomArea) }
                    .onSuccess {
                        val responseChores = it.choresList
                        chores = responseChores
                        rebuildChoreIndex(responseChores)
                        buildThumbnails(lastFrameBitmap, responseChores)
                        landmarks = it.landmarksList
                        loadExpected()
                        if (anchor != null) {
                            engine.anchor(anchor, choreBoxes(responseChores))
                            lastFingerprint = engine.fingerprint(anchor)
                            Log.d(TAG, "anchored ${responseChores.size} boxes from ${it.model}")
                        }
                        statusLine =
                            "${responseChores.size} chores in ${it.latencyMs} ms (${it.model})"
                    }
                    .onFailure { statusLine = "error: ${it.message}" }
            }
            busy = false
            scanning = false
            lastScanAt = SystemClock.elapsedRealtime()
            postFingerprint()
        }
    }

    /** Upload the latest scene fingerprint for cross-session sameness. */
    private fun postFingerprint() {
        val fingerprint = lastFingerprint ?: return
        viewModelScope.launch {
            withContext(Dispatchers.IO) {
                runCatching {
                    client.setFingerprint(serverUrl, roomId, FingerprintKind.FINGERPRINT_KIND_LATEST, fingerprint)
                }.onFailure { }
            }
        }
    }

    /** Rebuild the id -> chore map for overlay labels. */
    private fun rebuildChoreIndex(items: List<ChoreEntity>) {
        choresById.clear()
        items.forEach { choresById[it.id] = it }
    }

    /** Crop each chore's bounding box from the frame into a 128x128 thumbnail. */
    private fun buildThumbnails(bitmap: Bitmap?, items: List<ChoreEntity>) {
        choreThumbnails.clear()
        if (bitmap == null) return
        val w = bitmap.width
        val h = bitmap.height
        for (chore in items) {
            if (!chore.hasBox()) continue
            val box = chore.box
            val l = (box.xmin / 1000f * w).toInt().coerceIn(0, w)
            val t = (box.ymin / 1000f * h).toInt().coerceIn(0, h)
            val r = (box.xmax / 1000f * w).toInt().coerceIn(0, w)
            val b = (box.ymax / 1000f * h).toInt().coerceIn(0, h)
            if (r > l && b > t) {
                val crop = Bitmap.createBitmap(bitmap, l, t, r - l, b - t)
                choreThumbnails[chore.id] = Bitmap.createScaledBitmap(crop, 128, 128, true)
            }
        }
    }

    /** Fetch expected object labels for the current room from the server. */
    private fun loadExpected() {
        viewModelScope.launch {
            withContext(Dispatchers.IO) {
                runCatching { client.listExpected(serverUrl, roomId) }
                    .onSuccess { expectedLabels = it.toSet() }
                    .onFailure { }
            }
        }
    }

    /** Mark an object label as expected in the current room. */
    fun setExpected(label: String) {
        viewModelScope.launch {
            withContext(Dispatchers.IO) {
                runCatching { client.addExpected(serverUrl, roomId, label) }
                    .onSuccess {
                        expectedLabels = expectedLabels + label
                        statusLine = "\"$label\" marked as expected"
                    }
                    .onFailure { statusLine = "error: ${it.message}" }
            }
        }
    }

    /** Remove an expected object label from the current room. */
    fun clearExpected(label: String) {
        viewModelScope.launch {
            withContext(Dispatchers.IO) {
                runCatching { client.removeExpected(serverUrl, roomId, label) }
                    .onSuccess {
                        expectedLabels = expectedLabels - label
                        statusLine = "\"$label\" no longer expected"
                    }
                    .onFailure { statusLine = "error: ${it.message}" }
            }
        }
    }

    /** Convert contract chore boxes into tracker boxes. */
    private fun choreBoxes(items: List<ChoreEntity>): List<TrackedBox> =
        items.mapNotNull { chore ->
            if (!chore.hasBox()) return@mapNotNull null
            val box = chore.box
            TrackedBox(
                id = chore.id,
                xmin = box.xmin.toFloat(),
                ymin = box.ymin.toFloat(),
                xmax = box.xmax.toFloat(),
                ymax = box.ymax.toFloat(),
                confidence = 1.0f,
            )
        }

    /** Send a sweep to the slow loop and store the merged response. */
    private fun analyzeSweep() {
        val frames = sweepFrames.toList()
        val anchors = sweepAnchors.toList()
        viewModelScope.launch {
            busy = true
            statusLine = "analyzing sweep..."
            withContext(Dispatchers.IO) {
                runCatching { client.analyzeSweep(serverUrl, roomId, frames, roomArea) }
                    .onSuccess {
                        val items = it.choresList
                        chores = items
                        rebuildChoreIndex(items)
                        landmarks = it.landmarksList
                        lastFrameBitmap = frames.lastOrNull()
                            ?.let { b -> BitmapFactory.decodeByteArray(b, 0, b.size) }
                        buildThumbnails(lastFrameBitmap, items)
                        loadExpected()
                        val anchorGray = anchors.lastOrNull()
                        if (anchorGray != null) {
                            engine.anchor(anchorGray, choreBoxes(items))
                            lastFingerprint = engine.fingerprint(anchorGray)
                        }
                        statusLine = "${items.size} chores (sweep)"
                    }
                    .onFailure { statusLine = "error: ${it.message}" }
            }
            busy = false
            scanning = false
            lastScanAt = SystemClock.elapsedRealtime()
            postFingerprint()
        }
    }

    /** Downscale a captured JPEG to at most [maxDim] on its longest side. */
    private fun downscaleJpeg(jpeg: ByteArray, maxDim: Int): ByteArray {
        val bitmap = BitmapFactory.decodeByteArray(jpeg, 0, jpeg.size) ?: return jpeg
        val longest = maxOf(bitmap.width, bitmap.height)
        if (longest <= maxDim) return jpeg
        val scale = maxDim.toFloat() / longest
        val scaled = Bitmap.createScaledBitmap(
            bitmap,
            (bitmap.width * scale).toInt().coerceAtLeast(1),
            (bitmap.height * scale).toInt().coerceAtLeast(1),
            true,
        )
        val out = ByteArrayOutputStream()
        scaled.compress(Bitmap.CompressFormat.JPEG, 85, out)
        return out.toByteArray()
    }

    /** Save an annotated snapshot of the last frame for a chore to the gallery. */
    fun saveSnapshot(context: Context, chore: ChoreEntity) {
        val frame = lastFrameBitmap
        if (frame == null) {
            statusLine = "no frame to save yet"
            return
        }
        viewModelScope.launch {
            withContext(Dispatchers.IO) {
                runCatching {
                    val snapshot = annotateSnapshot(frame, chore)
                    insertToGallery(context, snapshot, chore.id)
                }
                    .onSuccess { statusLine = "snapshot saved" }
                    .onFailure { statusLine = "save failed: ${it.message}" }
            }
        }
    }

    /** Write a bitmap into Pictures/Lares via the MediaStore. */
    private fun insertToGallery(context: Context, bitmap: Bitmap, choreId: String): Uri {
        val name = "lares_${choreId.take(8)}_${System.currentTimeMillis()}.png"
        val values = ContentValues().apply {
            put(MediaStore.Images.Media.DISPLAY_NAME, name)
            put(MediaStore.Images.Media.MIME_TYPE, "image/png")
            put(MediaStore.Images.Media.RELATIVE_PATH, Environment.DIRECTORY_PICTURES + "/Lares")
            put(MediaStore.Images.Media.IS_PENDING, 1)
        }
        val resolver = context.contentResolver
        val uri = resolver.insert(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values)
            ?: throw IllegalStateException("could not create media entry")
        resolver.openOutputStream(uri)?.use { output ->
            bitmap.compress(Bitmap.CompressFormat.PNG, 100, output)
        } ?: throw IllegalStateException("could not open output stream")
        values.clear()
        values.put(MediaStore.Images.Media.IS_PENDING, 0)
        resolver.update(uri, values, null, null)
        return uri
    }
}