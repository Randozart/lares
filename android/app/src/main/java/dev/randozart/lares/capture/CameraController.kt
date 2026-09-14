package dev.randozart.lares.capture

import android.content.Context
import android.util.Size
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageCapture
import androidx.camera.core.ImageCaptureException
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.core.resolutionselector.ResolutionSelector
import androidx.camera.core.resolutionselector.ResolutionStrategy
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
    private val analysisExecutor: ExecutorService = Executors.newSingleThreadExecutor()

    /** The preview surface, wired during composition. */
    lateinit var previewView: PreviewView

    /** Called with each analysis frame on a background thread. */
    var analysisCallback: ((ImageProxy) -> Unit)? = null

    /** Shared analysis resolution (matches capture resolution). */
    private val analysisSize = Size(1280, 720)

    /** Bind the back camera to the given lifecycle owner. */
    fun bind(lifecycleOwner: LifecycleOwner) {
        val providerFuture = ProcessCameraProvider.getInstance(context)
        providerFuture.addListener({
            val provider = providerFuture.get()
            val preview = Preview.Builder()
                .setTargetRotation(previewView.display.rotation)
                .build()
                .also { it.setSurfaceProvider(previewView.surfaceProvider) }

            val resolution = ResolutionSelector.Builder()
                .setResolutionStrategy(ResolutionStrategy(analysisSize, ResolutionStrategy.FALLBACK_RULE_CLOSEST_LOWER_THEN_HIGHER))
                .build()
            imageCapture = ImageCapture.Builder()
                .setResolutionSelector(resolution)
                .setTargetRotation(previewView.display.rotation)
                .setCaptureMode(ImageCapture.CAPTURE_MODE_MINIMIZE_LATENCY)
                .build()
            val analysis = ImageAnalysis.Builder()
                .setResolutionSelector(resolution)
                .setTargetRotation(previewView.display.rotation)
                .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                .build()
                .also {
                    it.setAnalyzer(analysisExecutor) { proxy ->
                        analysisCallback?.invoke(proxy)
                    }
                }

            provider.unbindAll()
            provider.bindToLifecycle(
                lifecycleOwner,
                CameraSelector.DEFAULT_BACK_CAMERA,
                preview,
                imageCapture,
                analysis,
            )
        }, ContextCompat.getMainExecutor(context))
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