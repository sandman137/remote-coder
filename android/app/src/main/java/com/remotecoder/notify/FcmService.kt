package com.remotecoder.notify

import android.app.PendingIntent
import android.content.Intent
import android.net.Uri
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import com.remotecoder.RemoteCoderApp
import com.remotecoder.MainActivity
import com.remotecoder.R

/**
 * Receives the privacy-filtered pushes from the host notifier (DESIGN.md §8.5)
 * — data-only messages carrying {session, pane, state, agent} and NEVER code.
 * Builds a local notification whose tap deep-links straight to the pane.
 */
class FcmService : FirebaseMessagingService() {

    override fun onNewToken(token: String) {
        // Registration: the token is delivered to the host out-of-band during
        // pairing (or via a later authenticated call over the secure channel).
        // Persisted for the pairing/registration flow to pick up.
        getSharedPreferences("rcoder", MODE_PRIVATE).edit()
            .putString("fcm_token", token).apply()
    }

    override fun onMessageReceived(message: RemoteMessage) {
        val d = message.data
        val session = d["session"] ?: return
        val pane = d["pane"] ?: return
        val state = d["state"] ?: "waiting"
        val agent = d["agent"] ?: "agent"

        val deepLink = Uri.parse("remotecoder://pane/$session/$pane")
        val tap = PendingIntent.getActivity(
            this,
            pane.hashCode(),
            Intent(Intent.ACTION_VIEW, deepLink, this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )

        val title = when (state) {
            "done" -> "$agent finished"
            "error" -> "$agent errored"
            else -> "$agent needs input"
        }
        val notif = NotificationCompat.Builder(this, RemoteCoderApp.CHANNEL_ATTENTION)
            .setContentTitle(title)
            .setContentText("$session · $pane")
            .setSmallIcon(R.drawable.ic_stat_rocket)
            .setAutoCancel(true)
            .setContentIntent(tap)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .build()

        if (androidx.core.content.ContextCompat.checkSelfPermission(
                this,
                android.Manifest.permission.POST_NOTIFICATIONS,
            ) == android.content.pm.PackageManager.PERMISSION_GRANTED
        ) {
            NotificationManagerCompat.from(this).notify(pane.hashCode(), notif)
        }
    }
}
