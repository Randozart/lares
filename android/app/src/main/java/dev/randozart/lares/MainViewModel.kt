package dev.randozart.lares

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.randozart.lares.capture.CameraController
import dev.randozart.lares.net.LaresClient
import dev.randozart.lares.proto.AnalyzeMode
import dev.randozart.lares.proto.ChoreEntity
import dev.randozart.lares.proto.ChoreStatus
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * UI state for the fast loop: configuration, the frozen frame, and the chore
 * overlay derived from the slow loop's response.
 */
class MainViewModel : ViewModel() {
    private val client = LaresClient()

    var serverUrl by mutableStateOf("http://localhost:8787")
    var roomId by mutableStateOf("kitchen")
    var description by mutableStateOf("counters clear, room tidy")
    var mode by mutableStateOf(AnalyzeMode.ANALYZE_MODE_DISCOVER)
    var chores by mutableStateOf<List<ChoreEntity>>(emptyList())
    var frozen by mutableStateOf<Bitmap?>(null)
    var busy by mutableStateOf(false)
    var statusLine by mutableStateOf("idle")

    /** Capture a frame, freeze it, analyze it, and draw the resulting boxes. */
    fun captureAndAnalyze(controller: CameraController) {
        controller.captureJpeg { jpeg ->
            if (jpeg == null) {
                statusLine = "capture failed"
                return@captureJpeg
            }
            freeze(jpeg)
            analyze(jpeg)
        }
    }

    /** Capture a frame and store it as the room's agreed target state. */
    fun captureReference(controller: CameraController) {
        controller.captureJpeg { jpeg ->
            if (jpeg == null) {
                statusLine = "capture failed"
                return@captureJpeg
            }
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

    /** Clear the frozen frame and return to the live preview. */
    fun resume() {
        frozen = null
        statusLine = "idle"
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

    /** Reload the persisted chore list from the server. */
    fun refreshChores() {
        viewModelScope.launch {
            withContext(Dispatchers.IO) {
                runCatching { client.listChores(serverUrl, roomId) }
                    .onSuccess { chores = it; statusLine = "loaded ${it.size} chores" }
                    .onFailure { statusLine = "error: ${it.message}" }
            }
        }
    }

    /** Send the captured frame to the slow loop and store the response. */
    private fun analyze(jpeg: ByteArray) {
        viewModelScope.launch {
            busy = true
            statusLine = "analyzing..."
            withContext(Dispatchers.IO) {
                runCatching { client.analyze(serverUrl, roomId, jpeg, mode) }
                    .onSuccess {
                        chores = it.choresList
                        statusLine = "${it.choresList.size} chores in ${it.latencyMs} ms (${it.model})"
                    }
                    .onFailure { statusLine = "error: ${it.message}" }
            }
            busy = false
        }
    }

    /** Decode a JPEG into the frozen-frame bitmap. */
    private fun freeze(jpeg: ByteArray) {
        frozen = BitmapFactory.decodeByteArray(jpeg, 0, jpeg.size)
    }
}