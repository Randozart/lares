package dev.randozart.lares.net

import com.google.protobuf.ByteString
import com.google.protobuf.util.JsonFormat
import dev.randozart.lares.proto.AnalyzeMode
import dev.randozart.lares.proto.AnalyzeSceneRequest
import dev.randozart.lares.proto.AnalyzeSceneResponse
import dev.randozart.lares.proto.Briefing
import dev.randozart.lares.proto.ChoreEntity
import dev.randozart.lares.proto.ChoreKind
import dev.randozart.lares.proto.ChoreStatus
import dev.randozart.lares.proto.FingerprintKind
import dev.randozart.lares.proto.InferRoomRequest
import dev.randozart.lares.proto.InferRoomResponse
import dev.randozart.lares.proto.ImportCalendarRequest
import dev.randozart.lares.proto.ImportCalendarResponse
import dev.randozart.lares.proto.Landmark
import dev.randozart.lares.proto.LandmarkList
import dev.randozart.lares.proto.ListChoresResponse
import dev.randozart.lares.proto.Occasion
import dev.randozart.lares.proto.OccasionList
import dev.randozart.lares.proto.Person
import dev.randozart.lares.proto.PersonList
import dev.randozart.lares.proto.Preparation
import dev.randozart.lares.proto.PreparationKind
import dev.randozart.lares.proto.PreparationList
import dev.randozart.lares.proto.PreparationState
import dev.randozart.lares.proto.Recurrence
import dev.randozart.lares.proto.RecurrenceFreq
import dev.randozart.lares.proto.ReferenceState
import dev.randozart.lares.proto.Reminder
import dev.randozart.lares.proto.ReminderList
import dev.randozart.lares.proto.RoomArea
import dev.randozart.lares.proto.SetChoreStatusRequest
import dev.randozart.lares.proto.SetFingerprintRequest
import dev.randozart.lares.proto.SetReferenceRequest
import com.google.gson.JsonParser
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import java.util.concurrent.TimeUnit

/**
 * Thin protojson client for the Lares server.
 *
 * Messages are built from the generated protobuf classes and serialized with
 * protobuf-java-util's JsonFormat, which is the same protojson dialect the Rust
 * server emits. No hand-written DTOs.
 */
class LaresClient {
    private val http = OkHttpClient.Builder()
        .connectTimeout(10, TimeUnit.SECONDS)
        .readTimeout(60, TimeUnit.SECONDS)
        .build()

    private val printer = JsonFormat.printer().omittingInsignificantWhitespace()
    private val parser = JsonFormat.parser().ignoringUnknownFields()

    private val jsonType = "application/json; charset=utf-8".toMediaType()

    /** Analyze a captured frame and return the discovered chores. */
    fun analyze(
        baseUrl: String,
        roomId: String,
        jpeg: ByteArray,
        mode: AnalyzeMode,
        roomArea: RoomArea = RoomArea.ROOM_AREA_UNSPECIFIED,
    ): AnalyzeSceneResponse {
        val request = AnalyzeSceneRequest.newBuilder()
            .setRoomId(roomId)
            .setFrameJpeg(ByteString.copyFrom(jpeg))
            .setMode(mode)
            .setRoomArea(roomArea)
            .build()
        val body = post("$baseUrl/v1/analyze", printer.print(request))
        val builder = AnalyzeSceneResponse.newBuilder()
        parser.merge(body, builder)
        return builder.build()
    }

    /** Analyze a pan sweep of consecutive frames in a single request. */
    fun analyzeSweep(
        baseUrl: String,
        roomId: String,
        jpegs: List<ByteArray>,
        roomArea: RoomArea = RoomArea.ROOM_AREA_UNSPECIFIED,
    ): AnalyzeSceneResponse {
        val request = AnalyzeSceneRequest.newBuilder()
            .setRoomId(roomId)
            .setMode(AnalyzeMode.ANALYZE_MODE_DISCOVER)
            .setRoomArea(roomArea)
        jpegs.forEach { request.addSweepJpegs(ByteString.copyFrom(it)) }
        val body = post("$baseUrl/v1/analyze", printer.print(request.build()))
        val builder = AnalyzeSceneResponse.newBuilder()
        parser.merge(body, builder)
        return builder.build()
    }

    /** Store a room's agreed target state. */
    fun setReference(
        baseUrl: String,
        roomId: String,
        description: String,
        jpeg: ByteArray,
    ) {
        val request = SetReferenceRequest.newBuilder()
            .setRoomId(roomId)
            .setDescription(description)
            .setFrameJpeg(ByteString.copyFrom(jpeg))
            .build()
        send("$baseUrl/v1/rooms/$roomId/reference", printer.print(request))
    }

    /** List chores, optionally filtered by room. */
    fun listChores(baseUrl: String, roomId: String?): List<ChoreEntity> {
        val path = if (roomId == null) "$baseUrl/v1/chores" else "$baseUrl/v1/chores?roomId=$roomId"
        val builder = ListChoresResponse.newBuilder()
        parser.merge(get(path), builder)
        return builder.build().choresList
    }

    /** Transition a chore's lifecycle state; returns the updated chore. */
    fun setStatus(baseUrl: String, id: String, status: ChoreStatus): ChoreEntity {
        val request = SetChoreStatusRequest.newBuilder()
            .setChoreId(id)
            .setStatus(status)
            .build()
        val body = post("$baseUrl/v1/chores/$id/status", printer.print(request))
        val builder = ChoreEntity.newBuilder()
        parser.merge(body, builder)
        return builder.build()
    }

    /** Fetch the agreed reference state for a room. */
    fun getReference(baseUrl: String, roomId: String): ReferenceState? {
        val builder = ReferenceState.newBuilder()
        parser.merge(get("$baseUrl/v1/rooms/$roomId/reference"), builder)
        return builder.build()
    }

    /** Fetch a room's current landmark set. */
    fun getLandmarks(baseUrl: String, roomId: String): List<Landmark> {
        val builder = LandmarkList.newBuilder()
        parser.merge(get("$baseUrl/v1/rooms/$roomId/landmarks"), builder)
        return builder.build().landmarksList
    }

    /** Store a room's scene fingerprint (latest/history/clean). */
    fun setFingerprint(baseUrl: String, roomId: String, kind: FingerprintKind, gridHash: ByteArray) {
        val request = SetFingerprintRequest.newBuilder()
            .setKind(kind)
            .setGridHash(ByteString.copyFrom(gridHash))
            .build()
        send("$baseUrl/v1/rooms/$roomId/fingerprint", printer.print(request))
    }

    /** List expected object labels for a room. */
    fun listExpected(baseUrl: String, roomId: String): List<String> {
        val json = get("$baseUrl/v1/rooms/$roomId/expected")
        val root = com.google.gson.JsonParser.parseString(json).asJsonObject
        return root.getAsJsonArray("labels").map { it.asString }
    }

    /** Mark an object label as expected in a room. */
    fun addExpected(baseUrl: String, roomId: String, label: String) {
        val body = """{"label":"${label.replace("\"", "\\\"")}"}"""
        send("$baseUrl/v1/rooms/$roomId/expected", body)
    }

    /** Remove an expected object label from a room. */
    fun removeExpected(baseUrl: String, roomId: String, label: String) {
        delete("$baseUrl/v1/rooms/$roomId/expected/$label")
    }

    /** Fetch the proactive briefing for the given horizon. */
    fun getBriefing(baseUrl: String, horizonDays: Int = 7): Briefing {
        val builder = Briefing.newBuilder()
        parser.merge(get("$baseUrl/v1/briefing?horizonDays=$horizonDays"), builder)
        return builder.build()
    }

    /** Create a manual TASK chore with optional due date and recurrence. */
    fun createTask(
        baseUrl: String,
        roomId: String,
        title: String,
        dueAtUnix: Long?,
        freq: RecurrenceFreq = RecurrenceFreq.RECURRENCE_FREQ_NONE,
        weekday: Int = 0,
        tags: List<String> = emptyList(),
    ): ChoreEntity {
        val builder = ChoreEntity.newBuilder()
            .setRoomId(roomId)
            .setTarget(title)
            .setAction(title)
            .setKind(ChoreKind.CHORE_KIND_TASK)
            .addAllContextTags(tags)
        dueAtUnix?.let { builder.dueAtUnix = it }
        if (freq != RecurrenceFreq.RECURRENCE_FREQ_NONE) {
            builder.recurrence = Recurrence.newBuilder()
                .setFreq(freq)
                .setWeekday(weekday)
                .build()
        }
        val body = post("$baseUrl/v1/chores", printer.print(builder.build()))
        val parsed = ChoreEntity.newBuilder()
        parser.merge(body, parsed)
        return parsed.build()
    }

    /** List all people. */
    fun listPeople(baseUrl: String): List<Person> {
        val builder = PersonList.newBuilder()
        parser.merge(get("$baseUrl/v1/people"), builder)
        return builder.build().peopleList
    }

    /** Add a person. */
    fun addPerson(baseUrl: String, name: String, notes: String): Person {
        val request = Person.newBuilder().setName(name).setNotes(notes).build()
        val body = post("$baseUrl/v1/people", printer.print(request))
        val parsed = Person.newBuilder()
        parser.merge(body, parsed)
        return parsed.build()
    }

    /** Delete a person. */
    fun deletePerson(baseUrl: String, id: String) {
        delete("$baseUrl/v1/people/$id")
    }

    /** List all occasions. */
    fun listOccasions(baseUrl: String): List<Occasion> {
        val builder = OccasionList.newBuilder()
        parser.merge(get("$baseUrl/v1/occasions"), builder)
        return builder.build().occasionsList
    }

    /** Add an occasion. Date format: "MM-DD" yearly or "YYYY-MM-DD" once. */
    fun addOccasion(baseUrl: String, personId: String, title: String, date: String): Occasion {
        val request = Occasion.newBuilder()
            .setPersonId(personId)
            .setTitle(title)
            .setDate(date)
            .build()
        val body = post("$baseUrl/v1/occasions", printer.print(request))
        val parsed = Occasion.newBuilder()
        parser.merge(body, parsed)
        return parsed.build()
    }

    /** Delete an occasion. */
    fun deleteOccasion(baseUrl: String, id: String) {
        delete("$baseUrl/v1/occasions/$id")
    }

    /** Import occasions from an ICS calendar URL (e.g. Google secret iCal). */
    fun importCalendar(baseUrl: String, url: String): ImportCalendarResponse {
        val request = ImportCalendarRequest.newBuilder().setUrl(url).build()
        val body = post("$baseUrl/v1/calendar/import", printer.print(request))
        val builder = ImportCalendarResponse.newBuilder()
        parser.merge(body, builder)
        return builder.build()
    }

    /** Store a full chore entity (used when filing a scan target). */
    fun createChore(baseUrl: String, chore: ChoreEntity): ChoreEntity {
        val body = post("$baseUrl/v1/chores", printer.print(chore))
        val parsed = ChoreEntity.newBuilder()
        parser.merge(body, parsed)
        return parsed.build()
    }

    /** List preparations, optionally filtered by person and/or occasion. */
    fun listPreparations(baseUrl: String, personId: String? = null, occasionId: String? = null): List<Preparation> {
        val params = buildList {
            personId?.let { add("personId=$it") }
            occasionId?.let { add("occasionId=$it") }
        }
        val suffix = if (params.isEmpty()) "" else "?" + params.joinToString("&")
        val builder = PreparationList.newBuilder()
        parser.merge(get("$baseUrl/v1/preparations$suffix"), builder)
        return builder.build().preparationsList
    }

    /** Add a preparation (gift, cake, card, decor, cleaning, custom). */
    fun addPreparation(
        baseUrl: String,
        personId: String,
        occasionId: String,
        title: String,
        kind: PreparationKind,
    ): Preparation {
        val request = Preparation.newBuilder()
            .setPersonId(personId)
            .setOccasionId(occasionId)
            .setTitle(title)
            .setKind(kind)
            .setState(PreparationState.PREPARATION_STATE_IDEA)
            .build()
        val body = post("$baseUrl/v1/preparations", printer.print(request))
        val parsed = Preparation.newBuilder()
        parser.merge(body, parsed)
        return parsed.build()
    }

    /** Advance a preparation's state (IDEA → READY → DONE). */
    fun updatePreparationState(baseUrl: String, id: String, state: PreparationState): Preparation {
        val request = Preparation.newBuilder().setState(state).build()
        val body = patch("$baseUrl/v1/preparations/$id", printer.print(request))
        val parsed = Preparation.newBuilder()
        parser.merge(body, parsed)
        return parsed.build()
    }

    /** Delete a preparation. */
    fun deletePreparation(baseUrl: String, id: String) {
        delete("$baseUrl/v1/preparations/$id")
    }

    /** List undelivered reminders. */
    fun listUndeliveredReminders(baseUrl: String): List<Reminder> {
        val builder = ReminderList.newBuilder()
        parser.merge(get("$baseUrl/v1/reminders?undelivered=true"), builder)
        return builder.build().remindersList
    }

    /** Infer which stored room reference a frame matches best. */
    fun inferRoom(baseUrl: String, frameJpeg: ByteArray): InferRoomResponse {
        val request = InferRoomRequest.newBuilder()
            .setFrameJpeg(ByteString.copyFrom(frameJpeg))
            .build()
        val body = post("$baseUrl/v1/rooms/infer", printer.print(request))
        val builder = InferRoomResponse.newBuilder()
        parser.merge(body, builder)
        return builder.build()
    }

    /** Mark a reminder delivered. */
    fun markReminderDelivered(baseUrl: String, id: String) {
        patch("$baseUrl/v1/reminders/$id/delivered", "{}")
    }

    /** PATCH a protojson body and return the raw response text. */
    private fun patch(url: String, body: String): String {
        val request = Request.Builder()
            .url(url)
            .patch(body.toRequestBody(jsonType))
            .build()
        return http.newCall(request).execute().use { response ->
            check(response.isSuccessful) { "HTTP ${response.code}: ${response.body?.string()}" }
            response.body?.string() ?: ""
        }
    }

    /** POST a protojson body and return the raw response text. */
    private fun post(url: String, body: String): String {
        val request = Request.Builder()
            .url(url)
            .post(body.toRequestBody(jsonType))
            .build()
        return http.newCall(request).execute().use { response ->
            check(response.isSuccessful) { "HTTP ${response.code}: ${response.body?.string()}" }
            response.body?.string() ?: ""
        }
    }

    /** Send a POST and ignore the response body (e.g. 204 No Content). */
    private fun send(url: String, body: String) {
        val request = Request.Builder()
            .url(url)
            .post(body.toRequestBody(jsonType))
            .build()
        http.newCall(request).execute().use { response ->
            check(response.isSuccessful) { "HTTP ${response.code}: ${response.body?.string()}" }
        }
    }

    /** GET a URL and return the raw response text. */
    private fun get(url: String): String {
        val request = Request.Builder().url(url).build()
        return http.newCall(request).execute().use { response ->
            check(response.isSuccessful) { "HTTP ${response.code}: ${response.body?.string()}" }
            response.body?.string() ?: ""
        }
    }

    /** DELETE a URL and ignore the response body. */
    private fun delete(url: String) {
        val request = Request.Builder().url(url).delete().build()
        http.newCall(request).execute().use { response ->
            check(response.isSuccessful) { "HTTP ${response.code}: ${response.body?.string()}" }
        }
    }
}