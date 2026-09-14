package dev.randozart.lares

import android.Manifest
import android.content.pm.PackageManager
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.lifecycle.viewmodel.compose.viewModel
import dev.randozart.lares.capture.CameraController
import dev.randozart.lares.proto.AnalyzeMode
import dev.randozart.lares.proto.ChoreStatus
import dev.randozart.lares.sensing.SettleDetector
import dev.randozart.lares.ui.CameraPreview
import dev.randozart.lares.ui.LiveOverlay

/** Entry point: owns the camera controller and hosts the Compose UI. */
class MainActivity : ComponentActivity() {
    private lateinit var cameraController: CameraController

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        cameraController = CameraController(this)
        setContent {
            LaresApp(cameraController)
        }
    }

    override fun onDestroy() {
        super.onDestroy()
        cameraController.shutdown()
    }
}

/** Top-level UI for the live fast loop. */
@Composable
fun LaresApp(controller: CameraController) {
    val viewModel: MainViewModel = viewModel()
    val context = LocalContext.current
    var hasCamera by remember {
        mutableStateOf(
            ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) ==
                PackageManager.PERMISSION_GRANTED,
        )
    }
    val permissionLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission(),
    ) { granted -> hasCamera = granted }

    LaunchedEffect(Unit) {
        if (!hasCamera) {
            permissionLauncher.launch(Manifest.permission.CAMERA)
        }
    }

    // Feed every analysis frame into the tracker.
    LaunchedEffect(controller) {
        controller.analysisCallback = { viewModel.onAnalysisFrame(it) }
    }

    // Auto-scan on pan-settle, gated by the cost guards.
    val settleDetector = remember {
        SettleDetector(context) {
            if (viewModel.shouldAutoScan()) {
                viewModel.captureAndAnalyze(controller, bypassGates = false)
            }
        }
    }
    DisposableEffect(Unit) {
        settleDetector.start()
        onDispose { settleDetector.stop() }
    }

    Scaffold(topBar = {
        Row(
            modifier = Modifier.fillMaxWidth().padding(16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("Lares", style = MaterialTheme.typography.titleLarge)
            Spacer(Modifier.width(12.dp))
            Text(
                viewModel.statusLine,
                style = MaterialTheme.typography.bodySmall,
                modifier = Modifier.weight(1f),
            )
            if (viewModel.busy || viewModel.scanning) {
                CircularProgressIndicator(modifier = Modifier.height(20.dp).width(20.dp))
            }
        }
    }) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .padding(horizontal = 16.dp)
                .verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            SettingsPanel(viewModel)

            Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .aspectRatio(9f / 16f),
            ) {
                CameraPreview(
                    controller = controller,
                    modifier = Modifier.matchParentSize(),
                )
                LiveOverlay(
                    boxes = viewModel.trackedBoxes,
                    choresById = viewModel.choresById,
                    landmarks = viewModel.landmarks,
                    modifier = Modifier.matchParentSize(),
                )
            }

            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(
                    onClick = { viewModel.captureAndAnalyze(controller, bypassGates = true) },
                    enabled = hasCamera && !viewModel.busy,
                    modifier = Modifier.weight(1f),
                ) {
                    Text("Scan")
                }
                OutlinedButton(
                    onClick = { viewModel.captureReference(controller) },
                    enabled = hasCamera && !viewModel.busy,
                    modifier = Modifier.weight(1f),
                ) {
                    Text("Set Reference")
                }
            }

            ChoreList(viewModel)

            OutlinedButton(
                onClick = { viewModel.refreshChores() },
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text("Refresh")
            }
        }
    }
}

/** Configuration inputs for the server connection and analysis mode. */
@Composable
private fun SettingsPanel(viewModel: MainViewModel) {
    OutlinedTextField(
        value = viewModel.serverUrl,
        onValueChange = { viewModel.serverUrl = it },
        label = { Text("Server URL") },
        singleLine = true,
        modifier = Modifier.fillMaxWidth(),
    )
    OutlinedTextField(
        value = viewModel.roomId,
        onValueChange = { viewModel.roomId = it },
        label = { Text("Room") },
        singleLine = true,
        modifier = Modifier.fillMaxWidth(),
    )
    OutlinedTextField(
        value = viewModel.description,
        onValueChange = { viewModel.description = it },
        label = { Text("Reference description") },
        singleLine = true,
        modifier = Modifier.fillMaxWidth(),
    )
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        FilterChip(
            selected = viewModel.mode == AnalyzeMode.ANALYZE_MODE_DISCOVER,
            onClick = { viewModel.mode = AnalyzeMode.ANALYZE_MODE_DISCOVER },
            label = { Text("Discover") },
        )
        FilterChip(
            selected = viewModel.mode == AnalyzeMode.ANALYZE_MODE_DIFF,
            onClick = { viewModel.mode = AnalyzeMode.ANALYZE_MODE_DIFF },
            label = { Text("Diff vs reference") },
        )
    }
}

/** The persisted chore list with tap-to-done lifecycle transitions. */
@Composable
private fun ChoreList(viewModel: MainViewModel) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text("Chores", style = MaterialTheme.typography.titleMedium)
        if (viewModel.chores.isEmpty()) {
            Text(
                "No chores yet. Hold the camera still to auto-scan, or tap Scan.",
                style = MaterialTheme.typography.bodySmall,
            )
        }
        viewModel.chores.forEach { chore ->
            ChoreCard(chore) { status ->
                viewModel.setStatus(chore.id, status)
            }
        }
    }
}

/** A single chore with a done/restore toggle. */
@Composable
private fun ChoreCard(chore: dev.randozart.lares.proto.ChoreEntity, onToggle: (ChoreStatus) -> Unit) {
    val done = chore.status == ChoreStatus.CHORE_STATUS_DONE ||
        chore.status == ChoreStatus.CHORE_STATUS_DISMISSED
    Card(modifier = Modifier.fillMaxWidth()) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    chore.action,
                    style = MaterialTheme.typography.bodyLarge,
                )
                Text(
                    chore.subtasksList.joinToString(" → "),
                    style = MaterialTheme.typography.bodySmall,
                )
            }
            OutlinedButton(
                onClick = {
                    onToggle(if (done) ChoreStatus.CHORE_STATUS_DISCOVERED else ChoreStatus.CHORE_STATUS_DONE)
                },
            ) {
                Text(if (done) "Restore" else "Done")
            }
        }
    }
}