package dev.randozart.lares

import android.Manifest
import android.content.pm.PackageManager
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.lifecycle.viewmodel.compose.viewModel
import dev.randozart.lares.capture.CameraController
import dev.randozart.lares.proto.AnalyzeMode
import dev.randozart.lares.proto.ChoreEntity
import dev.randozart.lares.proto.ChoreStatus
import dev.randozart.lares.proto.RoomArea
import dev.randozart.lares.sensing.SettleDetector
import dev.randozart.lares.ui.CameraPreview
import dev.randozart.lares.ui.LiveOverlay
import kotlinx.coroutines.delay

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

/** Full-bleed immersive UI: live preview fills the screen, controls overlay it. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LaresApp(controller: CameraController) {
    val viewModel: MainViewModel = viewModel()
    val context = LocalContext.current
    var howChore by remember { mutableStateOf<ChoreEntity?>(null) }
    var showSettings by remember { mutableStateOf(false) }
    var showChores by remember { mutableStateOf(false) }
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
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

    LaunchedEffect(controller) {
        controller.analysisCallback = { viewModel.onAnalysisFrame(it) }
    }

    val settleDetector = remember {
        SettleDetector(context) {
            if (viewModel.autoScan && viewModel.shouldAutoScan()) {
                viewModel.captureAndAnalyze(controller, bypassGates = false)
            }
        }
    }
    DisposableEffect(viewModel.autoScan) {
        if (viewModel.autoScan) settleDetector.start()
        onDispose { settleDetector.stop() }
    }

    // Sweep: capture a keyframe every 1.2s while sweeping.
    LaunchedEffect(viewModel.sweeping) {
        while (viewModel.sweeping) {
            viewModel.captureSweepFrame(controller)
            delay(1200)
        }
    }

    Box(Modifier.fillMaxSize().background(Color.Black)) {
        CameraPreview(controller = controller, modifier = Modifier.matchParentSize())
        LiveOverlay(
            boxes = viewModel.trackedBoxes,
            choresById = viewModel.choresById,
            landmarks = viewModel.landmarks,
            frameW = viewModel.lastFrameWidth,
            frameH = viewModel.lastFrameHeight,
            modifier = Modifier.matchParentSize(),
        )

        // Top overlay: title + status + settings.
        Column(
            modifier = Modifier
                .align(Alignment.TopCenter)
                .fillMaxWidth()
                .padding(10.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text("Lares", style = MaterialTheme.typography.titleLarge, color = Color.White)
                Spacer(Modifier.width(10.dp))
                Text(
                    viewModel.statusLine,
                    style = MaterialTheme.typography.bodySmall,
                    color = Color.White.copy(alpha = 0.85f),
                    modifier = Modifier.weight(1f),
                )
                if (viewModel.busy || viewModel.scanning || viewModel.sweeping) {
                    CircularProgressIndicator(modifier = Modifier.height(18.dp).width(18.dp))
                }
                IconButton(onClick = { showSettings = true }) {
                    Icon(Icons.Default.Settings, contentDescription = "Settings", tint = Color.White)
                }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                AreaDropdown(viewModel)
                FilterChip(
                    selected = viewModel.mode == AnalyzeMode.ANALYZE_MODE_DISCOVER,
                    onClick = { viewModel.mode = AnalyzeMode.ANALYZE_MODE_DISCOVER },
                    label = { Text("Discover") },
                )
                FilterChip(
                    selected = viewModel.mode == AnalyzeMode.ANALYZE_MODE_DIFF,
                    onClick = { viewModel.mode = AnalyzeMode.ANALYZE_MODE_DIFF },
                    label = { Text("Diff") },
                )
                FilterChip(
                    selected = viewModel.autoScan,
                    onClick = { viewModel.autoScan = !viewModel.autoScan },
                    label = { Text("Auto") },
                )
            }
        }

        // Bottom overlay: sweep / scan / reference + chores sheet.
        Column(
            modifier = Modifier
                .align(Alignment.BottomCenter)
                .fillMaxWidth()
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Button(
                    onClick = {
                        if (viewModel.sweeping) viewModel.endSweep()
                        else viewModel.startSweep()
                    },
                    enabled = hasCamera && !viewModel.busy,
                    modifier = Modifier.weight(1f),
                ) {
                    Text(if (viewModel.sweeping) "Stop" else "Sweep")
                }
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
                    Text("Ref")
                }
            }
            OutlinedButton(
                onClick = { showChores = true },
                modifier = Modifier.fillMaxWidth(),
            ) {
                val count = viewModel.filteredChores.size
                val total = viewModel.chores.size
                Text(if (viewModel.showAllChores) "Chores ($total)" else "Chores ($count/$total)")
            }
        }
    }

    if (showSettings) {
        SettingsDialog(
            viewModel = viewModel,
            onDismiss = { showSettings = false },
        )
    }

    howChore?.let { chore ->
        HowDialog(
            chore = chore,
            onDismiss = { howChore = null },
            onSave = {
                viewModel.saveSnapshot(context, chore)
                howChore = null
            },
        )
    }

    if (showChores) {
        ModalBottomSheet(onDismissRequest = { showChores = false }, sheetState = sheetState) {
            Column(
                modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp).padding(bottom = 24.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.SpaceBetween,
                ) {
                    Text("Chores", style = MaterialTheme.typography.titleMedium)
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text("All", style = MaterialTheme.typography.labelSmall, color = Color.Gray)
                        Switch(
                            checked = viewModel.showAllChores,
                            onCheckedChange = { viewModel.showAllChores = it },
                        )
                    }
                }
                val display = viewModel.filteredChores
                if (display.isEmpty()) {
                    Text(
                        if (viewModel.chores.isEmpty()) "No chores yet. Scan or sweep the room."
                        else "All chores are expected here.",
                        style = MaterialTheme.typography.bodySmall,
                    )
                }
                LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    items(display) { chore ->
                        ChoreCard(
                            chore = chore,
                            viewModel = viewModel,
                            onHow = { howChore = chore },
                            onToggle = { viewModel.setStatus(chore.id, it) },
                        )
                    }
                }
            }
        }
    }
}

/** Dropdown for selecting the room area type. */
@Composable
private fun AreaDropdown(viewModel: MainViewModel) {
    var expanded by remember { mutableStateOf(false) }
    Box {
        FilterChip(
            selected = false,
            onClick = { expanded = true },
            label = { Text(viewModel.roomArea.displayName()) },
        )
        DropdownMenu(expanded = expanded, onDismissRequest = { expanded = false }) {
            RoomArea.entries.filter { it != RoomArea.ROOM_AREA_UNSPECIFIED }.forEach { area ->
                DropdownMenuItem(
                    text = { Text(area.displayName()) },
                    onClick = {
                        viewModel.roomArea = area
                        expanded = false
                    },
                )
            }
        }
    }
}

/** Human-readable name for a room area. */
private fun RoomArea.displayName(): String = when (this) {
    RoomArea.ROOM_AREA_KITCHEN -> "Kitchen"
    RoomArea.ROOM_AREA_BATHROOM -> "Bathroom"
    RoomArea.ROOM_AREA_BEDROOM -> "Bedroom"
    RoomArea.ROOM_AREA_LIVING_ROOM -> "Living Room"
    RoomArea.ROOM_AREA_DINING_ROOM -> "Dining Room"
    RoomArea.ROOM_AREA_OFFICE -> "Office"
    RoomArea.ROOM_AREA_GARAGE -> "Garage"
    RoomArea.ROOM_AREA_LAUNDRY -> "Laundry"
    RoomArea.ROOM_AREA_HALLWAY -> "Hallway"
    RoomArea.ROOM_AREA_KIDS_ROOM -> "Kids Room"
    RoomArea.ROOM_AREA_PATIO -> "Patio"
    RoomArea.ROOM_AREA_OTHER -> "Other"
    else -> "Room"
}

/** Settings dialog: server URL, room, and reference description. */
@Composable
private fun SettingsDialog(viewModel: MainViewModel, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Settings") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(
                    value = viewModel.serverUrl,
                    onValueChange = { viewModel.serverUrl = it },
                    label = { Text("Server URL") },
                    singleLine = true,
                )
                OutlinedTextField(
                    value = viewModel.roomId,
                    onValueChange = { viewModel.roomId = it },
                    label = { Text("Room") },
                    singleLine = true,
                )
                OutlinedTextField(
                    value = viewModel.description,
                    onValueChange = { viewModel.description = it },
                    label = { Text("Reference description") },
                    singleLine = true,
                )
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text("Done") } },
    )
}

/** How-to dialog for a chore, with an optional annotated snapshot save. */
@Composable
private fun HowDialog(chore: ChoreEntity, onDismiss: () -> Unit, onSave: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(chore.action) },
        text = {
            val steps = chore.howToList.ifEmpty { listOf(chore.action) }
            Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                steps.forEachIndexed { index, step ->
                    Text("${index + 1}. $step", style = MaterialTheme.typography.bodyMedium)
                }
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text("Done") } },
        dismissButton = { TextButton(onClick = onSave) { Text("Save snapshot") } },
    )
}

/** A single chore with thumbnail, how-to, expected toggle, and done/restore. */
@Composable
private fun ChoreCard(
    chore: ChoreEntity,
    viewModel: MainViewModel,
    onHow: () -> Unit,
    onToggle: (ChoreStatus) -> Unit,
) {
    val done = chore.status == ChoreStatus.CHORE_STATUS_DONE ||
        chore.status == ChoreStatus.CHORE_STATUS_DISMISSED
    val label = chore.target.trim().lowercase()
    val isExpected = label in viewModel.expectedLabels
    Card(modifier = Modifier.fillMaxWidth()) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(12.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            viewModel.choreThumbnails[chore.id]?.let { bmp ->
                Image(
                    bitmap = bmp.asImageBitmap(),
                    contentDescription = chore.target,
                    modifier = Modifier.size(64.dp),
                    contentScale = ContentScale.Crop,
                )
            }
            Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                val idx = chore.objectIndex
                val title = if (idx > 0) "[$idx] ${chore.action}" else chore.action
                Text(title, style = MaterialTheme.typography.bodyLarge)
                Text(
                    chore.subtasksList.joinToString(" → "),
                    style = MaterialTheme.typography.bodySmall,
                )
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    OutlinedButton(onClick = onHow, modifier = Modifier.weight(1f)) {
                        Text("How")
                    }
                    OutlinedButton(
                        onClick = {
                            onToggle(if (done) ChoreStatus.CHORE_STATUS_DISCOVERED else ChoreStatus.CHORE_STATUS_DONE)
                        },
                        modifier = Modifier.weight(1f),
                    ) {
                        Text(if (done) "Restore" else "Done")
                    }
                }
            }
            // Mark-as-expected toggle on the right edge.
            Column(
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.Center,
                modifier = Modifier.padding(top = 4.dp),
            ) {
                Text("Expected", style = MaterialTheme.typography.labelSmall, color = Color.Gray)
                Switch(
                    checked = isExpected,
                    onCheckedChange = {
                        if (isExpected) viewModel.clearExpected(label)
                        else viewModel.setExpected(label)
                    },
                )
            }
        }
    }
}