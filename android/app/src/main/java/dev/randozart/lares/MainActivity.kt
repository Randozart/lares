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
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
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
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
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
import dev.randozart.lares.proto.BriefingItem
import dev.randozart.lares.proto.ChoreEntity
import dev.randozart.lares.proto.ChoreStatus
import dev.randozart.lares.proto.Occasion
import dev.randozart.lares.proto.Person
import dev.randozart.lares.proto.Preparation
import dev.randozart.lares.proto.PreparationKind
import dev.randozart.lares.proto.PreparationState
import dev.randozart.lares.proto.RecurrenceFreq
import dev.randozart.lares.proto.RoomArea
import dev.randozart.lares.notify.BriefingFormat
import dev.randozart.lares.notify.BriefingWorker
import dev.randozart.lares.notify.ReminderWorker
import dev.randozart.lares.sensing.SettleDetector
import dev.randozart.lares.ui.CameraPreview
import dev.randozart.lares.ui.LiveOverlay
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import kotlinx.coroutines.delay
import java.time.Duration
import java.time.LocalDateTime
import java.time.LocalTime
import java.util.concurrent.TimeUnit

/** Entry point: owns the camera controller and hosts the Compose UI. */
class MainActivity : ComponentActivity() {
    private lateinit var cameraController: CameraController

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        cameraController = CameraController(this)
        scheduleBriefingWork()
        setContent {
            LaresApp(cameraController)
        }
    }

    /** Schedule the daily 08:00 briefing + the 15-min reminder check. */
    private fun scheduleBriefingWork() {
        val now = LocalDateTime.now()
        var next = now.toLocalDate().atTime(LocalTime.of(8, 0))
        if (!next.isAfter(now)) {
            next = next.plusDays(1)
        }
        val delayMinutes = Duration.between(now, next).toMinutes()
        val briefing = PeriodicWorkRequestBuilder<BriefingWorker>(24, TimeUnit.HOURS)
            .setInitialDelay(delayMinutes, TimeUnit.MINUTES)
            .build()
        WorkManager.getInstance(this).enqueueUniquePeriodicWork(
            "lares-briefing",
            ExistingPeriodicWorkPolicy.KEEP,
            briefing,
        )
        val reminders = PeriodicWorkRequestBuilder<ReminderWorker>(15, TimeUnit.MINUTES).build()
        WorkManager.getInstance(this).enqueueUniquePeriodicWork(
            ReminderWorker.WORK_NAME,
            ExistingPeriodicWorkPolicy.KEEP,
            reminders,
        )
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
    var showPeople by remember { mutableStateOf(false) }
    var showBriefing by remember { mutableStateOf(false) }
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    var hasCamera by remember {
        mutableStateOf(
            ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) ==
                PackageManager.PERMISSION_GRANTED,
        )
    }
    val neededPermissions = buildList {
        add(Manifest.permission.CAMERA)
        if (android.os.Build.VERSION.SDK_INT >= 33) {
            add(Manifest.permission.POST_NOTIFICATIONS)
        }
    }
    val permissionLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions(),
    ) { grants -> hasCamera = grants[Manifest.permission.CAMERA] == true }

    LaunchedEffect(Unit) {
        val missing = neededPermissions.filter {
            ContextCompat.checkSelfPermission(context, it) != PackageManager.PERMISSION_GRANTED
        }
        if (missing.isNotEmpty()) {
            permissionLauncher.launch(missing.toTypedArray())
        }
    }

    // Persist connection settings whenever they change (worker reads prefs).
    LaunchedEffect(viewModel.serverUrl, viewModel.calendarUrl) {
        viewModel.persistPrefs()
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
            BriefingCard(
                items = viewModel.briefingItems,
                onExpand = { showBriefing = true },
            )
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
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                OutlinedButton(
                    onClick = { showChores = true },
                    modifier = Modifier.weight(1f),
                ) {
                    val count = viewModel.filteredChores.size
                    val total = viewModel.chores.size
                    Text(if (viewModel.showAllChores) "Chores ($total)" else "Chores ($count/$total)")
                }
                OutlinedButton(
                    onClick = { showPeople = true },
                    modifier = Modifier.weight(1f),
                ) {
                    Text("People (${viewModel.people.size})")
                }
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
                Text("Tasks", style = MaterialTheme.typography.titleSmall)
                TaskQuickAdd(viewModel)
                viewModel.tasks.forEach { task ->
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        val due = task.dueAtUnix?.let { epoch ->
                            java.time.Instant.ofEpochSecond(epoch).toString().take(10)
                        } ?: "no date"
                        Text(
                            "$due — ${task.action}",
                            style = MaterialTheme.typography.bodyMedium,
                            modifier = Modifier.weight(1f),
                        )
                        TextButton(onClick = { viewModel.completeTask(task) }) {
                            Text("Done")
                        }
                    }
                }
                HorizontalDivider()
                Text("Vision chores", style = MaterialTheme.typography.titleSmall)
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

    if (showPeople) {
        ModalBottomSheet(onDismissRequest = { showPeople = false }, sheetState = sheetState) {
            PeopleSheet(viewModel)
        }
    }

    if (showBriefing) {
        AlertDialog(
            onDismissRequest = { showBriefing = false },
            title = { Text("Coming up") },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    if (viewModel.briefingItems.isEmpty()) {
                        Text("Nothing on the horizon.", style = MaterialTheme.typography.bodySmall)
                    }
                    viewModel.briefingItems.forEach { item ->
                        Text(BriefingFormat.format(item), style = MaterialTheme.typography.bodyMedium)
                    }
                }
            },
            confirmButton = {
                TextButton(onClick = { showBriefing = false }) { Text("Done") }
            },
        )
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

/** Compact overlay card summarizing the next briefing items. */
@Composable
private fun BriefingCard(items: List<BriefingItem>, onExpand: () -> Unit) {
    if (items.isEmpty()) return
    Card(
        modifier = Modifier
            .fillMaxWidth()
            .clickable { onExpand() },
        colors = CardDefaults.cardColors(
            containerColor = Color.Black.copy(alpha = 0.55f),
        ),
    ) {
        Column(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            items.take(3).forEach { item ->
                Text(
                    BriefingFormat.format(item),
                    style = MaterialTheme.typography.bodySmall,
                    color = Color.White,
                )
            }
            if (items.size > 3) {
                Text(
                    "+${items.size - 3} more — tap to see all",
                    style = MaterialTheme.typography.labelSmall,
                    color = Color.White.copy(alpha = 0.7f),
                )
            }
        }
    }
}

/** Quick-add row for manual tasks inside the chores sheet. */
@Composable
private fun TaskQuickAdd(viewModel: MainViewModel) {
    var title by remember { mutableStateOf("") }
    var due by remember { mutableStateOf("") }
    var weekly by remember { mutableStateOf(false) }
    var biweekly by remember { mutableStateOf(false) }
    var weekday by remember { mutableStateOf(0) }
    var shopping by remember { mutableStateOf(false) }
    val freq = when {
        weekly -> RecurrenceFreq.RECURRENCE_FREQ_WEEKLY
        biweekly -> RecurrenceFreq.RECURRENCE_FREQ_BIWEEKLY
        else -> RecurrenceFreq.RECURRENCE_FREQ_NONE
    }
    val days = listOf("Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun")
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        OutlinedTextField(
            value = title,
            onValueChange = { title = it },
            label = { Text("New task") },
            singleLine = true,
            modifier = Modifier.weight(1.4f),
        )
        OutlinedTextField(
            value = due,
            onValueChange = { due = it },
            label = { Text("YYYY-MM-DD") },
            singleLine = true,
            modifier = Modifier.weight(1f),
        )
        Button(
            onClick = {
                viewModel.addTask(title, due, freq, weekday, shopping)
                title = ""
                due = ""
            },
            enabled = title.isNotBlank(),
        ) {
            Text("Add")
        }
    }
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        FilterChip(
            selected = weekly,
            onClick = { weekly = !weekly; if (weekly) biweekly = false },
            label = { Text("Weekly") },
        )
        FilterChip(
            selected = biweekly,
            onClick = { biweekly = !biweekly; if (biweekly) weekly = false },
            label = { Text("Biweekly") },
        )
        if (weekly || biweekly) {
            days.forEachIndexed { index, day ->
                FilterChip(
                    selected = weekday == index,
                    onClick = { weekday = index },
                    label = { Text(day) },
                )
            }
        }
        FilterChip(
            selected = shopping,
            onClick = { shopping = !shopping },
            label = { Text("Shopping") },
        )
    }
}

/** People management sheet: registry, occasions, calendar sync. */
@Composable
private fun PeopleSheet(viewModel: MainViewModel) {
    Column(
        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp).padding(bottom = 24.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text("People", style = MaterialTheme.typography.titleMedium)
        PersonAddRow(viewModel)
        HorizontalDivider()
        LazyColumn(
            modifier = Modifier.heightIn(max = 280.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            items(viewModel.people) { person ->
                PersonCard(person, viewModel)
            }
        }
        HorizontalDivider()
        OccasionAddRow(viewModel)
        HorizontalDivider()
        CalendarSyncRow(viewModel)
    }
}

/** Inline form to add a person. */
@Composable
private fun PersonAddRow(viewModel: MainViewModel) {
    var name by remember { mutableStateOf("") }
    var notes by remember { mutableStateOf("") }
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        OutlinedTextField(
            value = name,
            onValueChange = { name = it },
            label = { Text("Name") },
            singleLine = true,
            modifier = Modifier.weight(1f),
        )
        OutlinedTextField(
            value = notes,
            onValueChange = { notes = it },
            label = { Text("Notes") },
            singleLine = true,
            modifier = Modifier.weight(1f),
        )
        Button(
            onClick = {
                viewModel.addPerson(name, notes)
                name = ""
                notes = ""
            },
            enabled = name.isNotBlank(),
        ) {
            Text("Add")
        }
    }
}

/** One person with occasions and event preparations listed beneath. */
@Composable
private fun PersonCard(person: Person, viewModel: MainViewModel) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(
            modifier = Modifier.fillMaxWidth().padding(12.dp),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Text(person.name, style = MaterialTheme.typography.bodyLarge)
            if (person.notes.isNotEmpty()) {
                Text(person.notes, style = MaterialTheme.typography.bodySmall)
            }
            viewModel.occasions.filter { it.personId == person.id }.forEach { occasion ->
                Text(
                    "• ${occasion.title} — ${occasion.date}",
                    style = MaterialTheme.typography.bodySmall,
                    color = Color.Gray,
                )
            }
            viewModel.preparations.filter { it.personId == person.id }.forEach { preparation ->
                val stateLabel = when (preparation.state) {
                    PreparationState.PREPARATION_STATE_IDEA -> "idea"
                    PreparationState.PREPARATION_STATE_READY -> "ready"
                    PreparationState.PREPARATION_STATE_DONE -> "done"
                    else -> "?"
                }
                Text(
                    "☐ $stateLabel: ${preparation.title} — tap to advance",
                    style = MaterialTheme.typography.bodySmall,
                    modifier = Modifier.clickable { viewModel.advancePreparation(preparation) },
                )
            }
            PreparationAddRow(person, viewModel)
        }
    }
}

/** Inline form to add a preparation for a person. */
@Composable
private fun PreparationAddRow(person: Person, viewModel: MainViewModel) {
    var title by remember { mutableStateOf("") }
    var kind by remember { mutableStateOf(PreparationKind.PREPARATION_KIND_GIFT) }
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        OutlinedTextField(
            value = title,
            onValueChange = { title = it },
            label = { Text("Prep (gift, cake…)") },
            singleLine = true,
            modifier = Modifier.weight(1f),
        )
        Button(
            onClick = {
                viewModel.addPreparation(person, title, kind)
                title = ""
            },
            enabled = title.isNotBlank(),
        ) {
            Text("Add")
        }
    }
    Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        listOf(
            "Gift" to PreparationKind.PREPARATION_KIND_GIFT,
            "Cake" to PreparationKind.PREPARATION_KIND_CAKE,
            "Card" to PreparationKind.PREPARATION_KIND_CARD,
            "Decor" to PreparationKind.PREPARATION_KIND_DECOR,
        ).forEach { (label, value) ->
            FilterChip(
                selected = kind == value,
                onClick = { kind = value },
                label = { Text(label) },
            )
        }
    }
}

/** Inline form to add a dated occasion. */
@Composable
private fun OccasionAddRow(viewModel: MainViewModel) {
    var person by remember { mutableStateOf("") }
    var title by remember { mutableStateOf("") }
    var date by remember { mutableStateOf("") }
    Text("Add occasion", style = MaterialTheme.typography.titleSmall)
    OutlinedTextField(
        value = person,
        onValueChange = { person = it },
        label = { Text("Person (optional)") },
        singleLine = true,
        modifier = Modifier.fillMaxWidth(),
    )
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        OutlinedTextField(
            value = title,
            onValueChange = { title = it },
            label = { Text("Title") },
            singleLine = true,
            modifier = Modifier.weight(1.4f),
        )
        OutlinedTextField(
            value = date,
            onValueChange = { date = it },
            label = { Text("MM-DD") },
            singleLine = true,
            modifier = Modifier.weight(1f),
        )
        Button(
            onClick = {
                viewModel.addOccasion(person, title, date)
                title = ""
                date = ""
            },
            enabled = title.isNotBlank() && date.isNotBlank(),
        ) {
            Text("Add")
        }
    }
}

/** ICS calendar URL field with a manual sync button. */
@Composable
private fun CalendarSyncRow(viewModel: MainViewModel) {
    Text("Calendar sync", style = MaterialTheme.typography.titleSmall)
    OutlinedTextField(
        value = viewModel.calendarUrl,
        onValueChange = { viewModel.calendarUrl = it },
        label = { Text("Google Calendar iCal URL") },
        singleLine = true,
        modifier = Modifier.fillMaxWidth(),
    )
    Button(
        onClick = { viewModel.syncCalendar() },
        enabled = viewModel.calendarUrl.isNotBlank() && !viewModel.busy,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text("Sync now")
    }
}