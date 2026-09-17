package dev.randozart.lares.notify

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.ActivityCompat
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.work.CoroutineWorker
import androidx.work.WorkerParameters
import dev.randozart.lares.net.LaresClient
import dev.randozart.lares.proto.Reminder

/** Periodic worker: surfaces undelivered forgotten-task reminders. */
class ReminderWorker(
    context: Context,
    params: WorkerParameters,
) : CoroutineWorker(context, params) {
    /** Fetch undelivered reminders, notify, and mark them delivered. */
    override suspend fun doWork(): Result {
        val prefs = applicationContext.getSharedPreferences("lares", Context.MODE_PRIVATE)
        val serverUrl = prefs.getString("serverUrl", null) ?: return Result.success()
        val client = LaresClient()
        val reminders = runCatching { client.listUndeliveredReminders(serverUrl) }
            .getOrNull()
            ?: return Result.success()
        if (reminders.isEmpty()) {
            return Result.success()
        }
        postNotification(reminders)
        reminders.forEach { reminder ->
            runCatching { client.markReminderDelivered(serverUrl, reminder.id) }
        }
        return Result.success()
    }

    /** Post one summary notification for the fetched reminders. */
    private fun postNotification(reminders: List<Reminder>) {
        val context = applicationContext
        val manager = NotificationManagerCompat.from(context)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = android.app.NotificationChannel(
                CHANNEL_ID,
                "Forgotten tasks",
                android.app.NotificationManager.IMPORTANCE_HIGH,
            )
            manager.createNotificationChannel(channel)
        }
        val body = reminders.joinToString("\n") { it.reason }
        val notification = NotificationCompat.Builder(context, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.ic_dialog_info)
            .setContentTitle("Lares — did you forget something?")
            .setContentText(reminders.first().reason)
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
        const val CHANNEL_ID = "lares_reminders"
        /** Stable notification id. */
        const val NOTIFICATION_ID = 1002
        /** Unique WorkManager name. */
        const val WORK_NAME = "lares-reminders"
    }
}
