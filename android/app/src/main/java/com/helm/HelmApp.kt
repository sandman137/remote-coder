package com.helm

import android.app.Application
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context

/**
 * Application entry. Creates the notification channels used by the foreground
 * service (persistent "connected" state, the iOS-Live-Activity equivalent)
 * and by attention pushes.
 */
class HelmApp : Application() {
    override fun onCreate() {
        super.onCreate()
        val nm = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        nm.createNotificationChannel(
            NotificationChannel(
                CHANNEL_STATUS,
                "Session status",
                NotificationManager.IMPORTANCE_LOW,
            ).apply { description = "Persistent connection + attention state" },
        )
        nm.createNotificationChannel(
            NotificationChannel(
                CHANNEL_ATTENTION,
                "Agent needs attention",
                NotificationManager.IMPORTANCE_HIGH,
            ).apply { description = "An agent is waiting for input" },
        )
    }

    companion object {
        const val CHANNEL_STATUS = "helm.status"
        const val CHANNEL_ATTENTION = "helm.attention"
    }
}
