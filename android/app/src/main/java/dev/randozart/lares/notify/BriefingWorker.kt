package dev.randozart.lares.notify

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.ActivityCompat
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.work.CoroutineWorker
import androidx.work.WorkerParameters
import dev.randozart.lares.net.LaresClient
import dev.randozart.lares.proto.BriefingItem

/** Formats briefing items for cards and notifications. */
object BriefingFormat {
    /** Human one-liner for a briefing item, e.g. "Emma's Birthday: in 5d — gift". */
    fun format(item: BriefingItem): String {
        val day = when {
            item.daysUntil < 0 -> "${-item.daysUntil}d overdue"
            item.daysUntil == 0 -> "today"
            else -> "in ${item.daysUntil}d"
        }
        val flags = item.flagsList.mapNotNull { flag ->
            when (flag) {
                dev.randozart.lares.proto.LeadFlag.LEAD_FLAG_GIFT -> "gift"
                dev.randozart.lares.proto.LeadFlag.LEAD_FLAG_CAKE -> "cake"
                dev.randozart.lares.proto.LeadFlag.LEAD_FLAG_CARD -> "card"
                else -> null
            }
        }
        val flagText = if (flags.isEmpty()) "" else " — ${flags.joinToString("/")}"
        return "${item.title}: $day$flagText"
    }
}

/** Daily worker: fetches the briefing and posts a local notification. */
class BriefingWorker(
    context: Context,
    params: WorkerParameters,
) : CoroutineWorker(context, params) {
    /** Fetch the briefing and notify when there is something to act on. */
    override suspend fun doWork(): Result {
        val prefs = applicationContext.getSharedPreferences("lares", Context.MODE_PRIVATE)
        val serverUrl = prefs.getString("serverUrl", null) ?: return Result.success()
        val briefing = runCatching { LaresClient().getBriefing(serverUrl) }.getOrNull()
            ?: return Result.success()
        if (briefing.itemsList.isEmpty()) {
            return Result.success()
        }
        postNotification(briefing.itemsList)
        return Result.success()
    }

    /** Post the digest as a BigTextStyle notification. */
    private fun postNotification(items: List<BriefingItem>) {
        val context = applicationContext
        val manager = NotificationManagerCompat.from(context)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                CHANNEL_ID,
                "Daily briefing",
                NotificationManager.IMPORTANCE_DEFAULT,
            )
            manager.createNotificationChannel(channel)
        }
        val body = items.joinToString("\n") { BriefingFormat.format(it) }
        val notification = NotificationCompat.Builder(context, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.ic_dialog_info)
            .setContentTitle("Lares — ${items.size} thing${if (items.size == 1) "" else "s"} coming up")
            .setContentText(items.firstOrNull()?.let { BriefingFormat.format(it) } ?: "")
            .setStyle(NotificationCompat.BigTextStyle().bigText(body))
            .build()
        val granted = ActivityCompat.checkSelfPermission(
            context,
            Manifest.permission.POST_NOTIFICATIONS,
        ) == PackageManager.PERMISSION_GRANTED
        if (granted || Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
            manager.notify(NOTIFICATION_ID, notification)
        }
    }

    companion object {
        /** Notification channel id. */
        const val CHANNEL_ID = "lares_briefing"
        /** Stable notification id for the daily digest. */
        const val NOTIFICATION_ID = 1001
    }
}
