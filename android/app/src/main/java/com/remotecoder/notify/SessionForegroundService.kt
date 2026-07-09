package com.remotecoder.notify

import android.app.Notification
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.lifecycle.LifecycleService
import com.remotecoder.RemoteCoderApp
import com.remotecoder.MainActivity
import com.remotecoder.R

/**
 * Foreground service holding a persistent notification that mirrors
 * {agent, state} — the iOS-Live-Activity equivalent (DESIGN.md §9). Keeps the
 * process alive so streaming/attention survive backgrounding. The actual
 * push-on-attention decision is host-side (the notifier daemon); this service
 * reflects state and hosts the deep-link target.
 */
class SessionForegroundService : LifecycleService() {

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        super.onStartCommand(intent, flags, startId)
        val text = intent?.getStringExtra(EXTRA_TEXT) ?: "Connected"
        startForeground(NOTIF_ID, buildNotification(text))
        return START_STICKY
    }

    private fun buildNotification(text: String): Notification {
        val tap = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        return NotificationCompat.Builder(this, RemoteCoderApp.CHANNEL_STATUS)
            .setContentTitle("Remote Coder")
            .setContentText(text)
            .setSmallIcon(R.drawable.ic_stat_rocket)
            .setOngoing(true)
            .setContentIntent(tap)
            .build()
    }

    companion object {
        private const val NOTIF_ID = 1
        private const val EXTRA_TEXT = "text"

        fun start(context: Context, text: String? = null) {
            val intent = Intent(context, SessionForegroundService::class.java)
            text?.let { intent.putExtra(EXTRA_TEXT, it) }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                context.startForegroundService(intent)
            } else {
                context.startService(intent)
            }
        }
    }
}
