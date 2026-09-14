package dev.randozart.lares.ui

import androidx.camera.view.PreviewView
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.compose.LocalLifecycleOwner
import dev.randozart.lares.capture.CameraController

/** The live camera viewfinder (fast loop display). */
@Composable
fun CameraPreview(
    controller: CameraController,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current

    AndroidView(
        factory = { ctx ->
            PreviewView(ctx).also {
                controller.previewView = it
                it.scaleType = PreviewView.ScaleType.FIT_CENTER
            }
        },
        modifier = modifier,
    )
    LaunchedEffect(lifecycleOwner) {
        controller.bind(lifecycleOwner)
    }
}