package dev.randozart.lares.net

import com.google.protobuf.ByteString
import com.google.protobuf.util.JsonFormat
import dev.randozart.lares.proto.AnalyzeMode
import dev.randozart.lares.proto.AnalyzeSceneRequest
import dev.randozart.lares.proto.AnalyzeSceneResponse
import dev.randozart.lares.proto.ChoreEntity
import dev.randozart.lares.proto.ChoreStatus
import dev.randozart.lares.proto.ListChoresResponse
import dev.randozart.lares.proto.ReferenceState
import dev.randozart.lares.proto.SetChoreStatusRequest
import dev.randozart.lares.proto.SetReferenceRequest
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
    ): AnalyzeSceneResponse {
        val request = AnalyzeSceneRequest.newBuilder()
            .setRoomId(roomId)
            .setFrameJpeg(ByteString.copyFrom(jpeg))
            .setMode(mode)
            .build()
        val body = post("$baseUrl/v1/analyze", printer.print(request))
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
}