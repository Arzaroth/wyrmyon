package io.github.arzaroth.wyrmyon

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder

class TransferService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val title = intent?.getStringExtra(TITLE) ?: getString(R.string.transfer_running)
        startForeground(
            Notifications.ONGOING,
            Notifications.ongoing(this, title, null, 0, 0),
            ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC,
        )
        if (intent?.action == STOP && running == 0) {
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
        }
        return START_NOT_STICKY
    }

    override fun onTimeout(startId: Int, fgsType: Int) {
        stopSelf()
    }

    companion object {
        private const val TITLE = "title"
        private const val STOP = "stop"
        private var running = 0

        fun start(context: Context, title: String) {
            running++
            context.startForegroundService(Intent(context, TransferService::class.java).putExtra(TITLE, title))
        }

        fun progress(context: Context, title: String, text: String, bytes: Long, total: Long) {
            Notifications.manager(context)
                .notify(Notifications.ONGOING, Notifications.ongoing(context, title, text, bytes, total))
        }

        fun stop(context: Context, title: String, text: String?) {
            running = maxOf(0, running - 1)
            if (running == 0) halt(context, title)
            if (text != null) {
                Notifications.manager(context).notify(Notifications.ENDED, Notifications.ended(context, title, text))
            }
        }

        fun forget(context: Context) {
            if (running == 0) return
            running = 0
            halt(context, context.getString(R.string.transfer_running))
        }

        private fun halt(context: Context, title: String) {
            val stopping = Intent(context, TransferService::class.java).setAction(STOP).putExtra(TITLE, title)
            try {
                context.startService(stopping)
            } catch (e: IllegalStateException) {
                context.stopService(stopping)
            }
            Notifications.manager(context).cancel(Notifications.ONGOING)
        }
    }
}

object Notifications {
    const val ONGOING = 1
    const val ENDED = 2
    private const val RUNNING_CHANNEL = "running"
    private const val ENDED_CHANNEL = "ended"

    fun manager(context: Context): NotificationManager =
        context.getSystemService(NotificationManager::class.java)

    fun createChannels(context: Context) {
        manager(context).createNotificationChannels(
            listOf(
                NotificationChannel(
                    RUNNING_CHANNEL,
                    context.getString(R.string.channel_running),
                    NotificationManager.IMPORTANCE_LOW,
                ),
                NotificationChannel(
                    ENDED_CHANNEL,
                    context.getString(R.string.channel_ended),
                    NotificationManager.IMPORTANCE_DEFAULT,
                ),
            ),
        )
    }

    fun ongoing(context: Context, title: String, text: String?, bytes: Long, total: Long): Notification {
        val builder = Notification.Builder(context, RUNNING_CHANNEL)
            .setSmallIcon(android.R.drawable.stat_sys_upload)
            .setContentTitle(title)
            .setContentIntent(opening(context))
            .setOngoing(true)
            .setOnlyAlertOnce(true)
        text?.let(builder::setContentText)
        if (total > 0) {
            val scale = maxOf(1L, total / Int.MAX_VALUE + 1)
            builder.setProgress((total / scale).toInt(), (bytes / scale).toInt(), false)
        }
        return builder.build()
    }

    fun ended(context: Context, title: String, text: String): Notification =
        Notification.Builder(context, ENDED_CHANNEL)
            .setSmallIcon(android.R.drawable.stat_sys_download_done)
            .setContentTitle(title)
            .setContentText(text)
            .setContentIntent(opening(context))
            .setAutoCancel(true)
            .build()

    private fun opening(context: Context): PendingIntent =
        PendingIntent.getActivity(
            context,
            0,
            Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE,
        )
}
