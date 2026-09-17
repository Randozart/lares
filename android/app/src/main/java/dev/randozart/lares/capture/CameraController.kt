package dev.randozart.lares.capture

import android.content.Context
import android.util.Log
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraManager
import androidx.camera.core.Camera
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageCapture
import androidx.camera.core.ImageCaptureException
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleOwner
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors

/**
 * Owns the CameraX preview, image capture, and frame analysis for the fast
 * loop. Capture and analysis share one resolution so VLM boxes and the tracker
 * live in the same coordinate space.
 */
class CameraController(private val context: Context) {
    private var imageCapture: ImageCapture? = null
    private var camera: Camera? = null
    private val analysisExecutor: ExecutorService = Executors.newSingleThreadExecutor()

    /** The preview surface, wired during composition. */
    lateinit var previewView: PreviewView

    /** Called with each analysis frame on a background thread. */
    var analysisCallback: ((ImageProxy) -> Unit)? = null

    /** Frames delivered by the analysis use-case, for debugging. */
    private var analysisFrames = 0

    /** Bind the back camera to the given lifecycle owner. */
    fun bind(lifecycleOwner: LifecycleOwner) {
        val providerFuture = ProcessCameraProvider.getInstance(context)
        providerFuture.addListener({
            val provider = providerFuture.get()
            val preview = Preview.Builder()
                .setTargetRotation(previewView.display.rotation)
                .build()
                .also { it.setSurfaceProvider(previewView.surfaceProvider) }

            // No forced resolution: forcing capture+analysis to the same size
            // breaks the stream combo on some devices (e.g. analysis resolves
            // to 960x960, capture to 2448x2448) and the session never starts.
            imageCapture = ImageCapture.Builder()
                .setTargetRotation(previewView.display.rotation)
                .setCaptureMode(ImageCapture.CAPTURE_MODE_MINIMIZE_LATENCY)
                .build()
            val analysis = ImageAnalysis.Builder()
                .setTargetRotation(previewView.display.rotation)
                .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                .build()
                .also {
                    it.setAnalyzer(analysisExecutor) { proxy ->
                        analysisFrames += 1
                        if (analysisFrames <= 5 || analysisFrames % 60 == 0) {
                            Log.d(
                                "LaresCam",
                                "frame #$analysisFrames ${proxy.width}x${proxy.height} " +
                                    "rot=${proxy.imageInfo.rotationDegrees} cb=${analysisCallback != null}",
                            )
                        }
                        analysisCallback?.invoke(proxy)
                    }
                }

            provider.unbindAll()
            camera = provider.bindToLifecycle(
                lifecycleOwner,
                CameraSelector.DEFAULT_BACK_CAMERA,
                preview,
                imageCapture,
                analysis,
            )
            Log.d("LaresCam", "bound preview+capture+analysis (default resolutions)")
        }, ContextCompat.getMainExecutor(context))
    }

    /** Torch state: 0 = off, 1 = low, 2 = max. Low exists only when throttleable. */
    var torchState by androidx.compose.runtime.mutableIntStateOf(0)
        private set

    /** Whether the flash hardware supports brightness levels. */
    var torchThrottleable by androidx.compose.runtime.mutableStateOf(false)
        private set

    private var strengthMax = 1

    /** Probe flash capabilities for the back camera. */
    fun probeTorch() {
        runCatching {
            val manager = context.getSystemService(Context.CAMERA_SERVICE) as CameraManager
            val id = manager.cameraIdList.firstOrNull { cid ->
                manager.getCameraCharacteristics(cid)
                    .get(CameraCharacteristics.LENS_FACING) ==
                    CameraCharacteristics.LENS_FACING_BACK
            } ?: return
            val characteristics = manager.getCameraCharacteristics(id)
            val max = characteristics.get(
                CameraCharacteristics.FLASH_INFO_STRENGTH_MAXIMUM_LEVEL,
            ) ?: 1
            strengthMax = max
            torchThrottleable = max > 1
        }
    }

    /** Cycle torch OFF -> (LOW) -> MAX -> OFF. */
    fun cycleTorch() {
        val next = when {
            torchState == 0 -> if (torchThrottleable) 1 else 2
            torchState == 1 -> 2
            else -> 0
        }
        applyTorch(next)
    }

    /** Apply a torch level through the camera, falling back to CameraX torch. */
    private fun applyTorch(state: Int) {
        val flashOn = state > 0
        var applied = false
        runCatching {
            val manager = context.getSystemService(Context.CAMERA_SERVICE) as CameraManager
            val id = manager.cameraIdList.firstOrNull { cid ->
                manager.getCameraCharacteristics(cid)
                    .get(CameraCharacteristics.LENS_FACING) ==
                    CameraCharacteristics.LENS_FACING_BACK
            } ?: return
            when {
                state == 0 -> manager.setTorchMode(id, false)
                strengthMax > 1 ->
                    manager.turnOnTorchWithStrengthLevel(
                        id,
                        if (state == 1) 1 else strengthMax,
                    )
                else -> manager.setTorchMode(id, true)
            }
            applied = true
        }
        if (!applied) {
            camera?.cameraControl?.enableTorch(flashOn)
        }
        torchState = state
    }

    /** Capture a JPEG frame and hand the bytes to [onResult] on the main thread. */
    fun captureJpeg(onResult: (ByteArray?) -> Unit) {
        val capture = imageCapture
        if (capture == null) {
            onResult(null)
            return
        }
        capture.takePicture(
            ContextCompat.getMainExecutor(context),
            object : ImageCapture.OnImageCapturedCallback() {
                override fun onCaptureSuccess(image: ImageProxy) {
                    val buffer = image.planes[0].buffer
                    val bytes = ByteArray(buffer.remaining())
                    buffer.get(bytes)
                    image.close()
                    onResult(bytes)
                }

                override fun onError(exception: ImageCaptureException) {
                    onResult(null)
                }
            },
        )
    }

    /** Stop the analysis executor when the app is destroyed. */
    fun shutdown() {
        analysisExecutor.shutdown()
    }
}