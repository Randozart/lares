package dev.randozart.lares

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.os.SystemClock
import androidx.camera.core.ImageProxy
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.randozart.lares.capture.CameraController
import dev.randozart.lares.net.LaresClient
import dev.randozart.lares.proto.AnalyzeMode
import dev.randozart.lares.proto.ChoreEntity
import dev.randozart.lares.proto.ChoreStatus
import dev.randozart.lares.proto.FingerprintKind
import dev.randozart.lares.proto.Landmark
import dev.randozart.lares.tracking.TrackingEngine
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.lares_tracking.GrayFrame
import uniffi.lares_tracking.TrackedBox

/** Maximum fingerprint distance (of 64 bits) that still counts as "unchanged". */
private const val SCENE_CHANGE_BITS = 6u
/** Minimum milliseconds between auto-scans. */
private const val MIN_SCAN_INTERVAL_MS = 5000L

/**
 * UI state for the fast loop: live tracked boxes, the cost-guard auto-scan
 * state machine, and the chore/landmark overlay derived from the slow loop.
 */
class MainViewModel : ViewModel() {
    private val client = LaresClient()
    private val engine = TrackingEngine(2u)

    var serverUrl by mutableStateOf("http://localhost:8787")
    var roomId by mutableStateOf("kitchen")
    var description by mutableStateOf("counters clear, room tidy")
    var mode by mutableStateOf(AnalyzeMode.ANALYZE_MODE_DISCOVER)

    var chores by mutableStateOf<List<ChoreEntity>>(emptyList())
    var trackedBoxes by mutableStateOf<List<TrackedBox>>(emptyList())
    var landmarks by mutableStateOf<List<Landmark>>(emptyList())
    var busy by mutableStateOf(false)
    var scanning by mutableStateOf(false)
    var statusLine by mutableStateOf("idle")

    /** Chore lookup by tracked-box id for overlay labels. */
    val choresById = mutableStateMapOf<String, ChoreEntity>()

    private var lastFingerprint: ByteArray? = null
    private var lastScanAt = 0L

    /** A captured reference still, used for the reference flow. */
    var frozenReference: Bitmap? = null
        private set

    /** Feed a CameraX analysis frame into the tracker. */
    fun onAnalysisFrame(proxy: ImageProxy) {
        val gray = proxy.toGrayFrame()
        engine.onFrame(gray)
        trackedBoxes = engine.boxes
    }

    /** Evaluate the auto-scan cost guards (G2 scene-change, G3 rate limit). */
    fun shouldAutoScan(): Boolean {
        if (busy) return false
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
                runCatching { client.analyze(serverUrl, roomId, jpeg, mode) }
                    .onSuccess {
                        val responseChores = it.choresList
                        chores = responseChores
                        rebuildChoreIndex(responseChores)
                        landmarks = it.landmarksList
                        if (anchor != null) {
                            engine.anchor(anchor, choreBoxes(responseChores))
                            lastFingerprint = engine.fingerprint(anchor)
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
            )
        }
}