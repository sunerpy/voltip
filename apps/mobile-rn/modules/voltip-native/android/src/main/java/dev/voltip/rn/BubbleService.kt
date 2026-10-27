package dev.voltip.rn

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.content.res.Configuration
import android.os.Build
import android.os.IBinder
import android.provider.Settings
import android.util.Log
import androidx.core.content.ContextCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.json.JSONObject

/**
 * The floating bubble's foreground service (docs/dictation.md §20.12, docs/mobile-rn.md §11): it
 * keeps [BubbleOverlay] on the screen over other apps and a notification with 关闭 in the shade,
 * and holds the process with the microphone type, which lets the bubble record while another app
 * is in front. Android allows that type only to a service started while the app is in front, so
 * only the app starts it ([apply], from 设置 › 悬浮球 and when the app starts with the bubble on);
 * it is not restarted by the system (`START_NOT_STICKY`). The shell is [VoltipRuntime]'s, shared
 * with the app's JavaScript.
 */
class BubbleService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private lateinit var prefs: BubblePrefs
    private var overlay: BubbleOverlay? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        prefs = BubblePrefs(this)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            // 关闭 in the notification: off until the user switches it on again.
            prefs.disable()
            running = false
            stopSelf()
            return START_NOT_STICKY
        }
        if (!inForeground()) {
            running = false
            stopSelf()
            return START_NOT_STICKY
        }
        try {
            VoltipRuntime.start(this)
        } catch (e: Exception) {
            Log.w(TAG, "the shell did not start for the bubble: ${e.message}")
            running = false
            stopSelf()
            return START_NOT_STICKY
        }
        val shown = overlay ?: BubbleOverlay(this, prefs, scope).also {
            overlay = it
            it.show()
        }
        shown.refresh()
        return START_NOT_STICKY
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        // Another orientation or screen size: lay the bubble out for it.
        overlay?.refresh()
    }

    override fun onDestroy() {
        overlay?.close()
        overlay = null
        scope.cancel()
        running = false
        super.onDestroy()
    }

    /** Into the foreground with the bubble's notification, as the microphone's service. */
    private fun inForeground(): Boolean {
        val notification = notification()
        return try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE)
            } else {
                startForeground(NOTIFICATION_ID, notification)
            }
            true
        } catch (e: RuntimeException) {
            // Started from the background, or the microphone was refused meanwhile.
            Log.w(TAG, "the bubble did not enter the foreground: ${e.message}")
            false
        }
    }

    private fun notification(): Notification {
        val manager = getSystemService(NotificationManager::class.java)
        val label = applicationInfo.loadLabel(packageManager).toString()
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL_ID, prefs.string("channel", label), NotificationManager.IMPORTANCE_LOW).apply {
                setShowBadge(false)
            },
        )
        val flags = PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        val open = packageManager.getLaunchIntentForPackage(packageName)?.let {
            PendingIntent.getActivity(this, 0, it, flags)
        }
        val stop = PendingIntent.getService(this, 1, Intent(this, BubbleService::class.java).setAction(ACTION_STOP), flags)
        val body = if (prefs.mode == BubblePrefs.MODE_TAP) prefs.string("bodyTap", "") else prefs.string("bodyHold", "")
        return Notification.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.voltip_bubble_notification)
            .setContentTitle(prefs.string("title", label))
            .setContentText(body)
            .setOngoing(true)
            .setShowWhen(false)
            .setCategory(Notification.CATEGORY_SERVICE)
            .setContentIntent(open)
            .addAction(Notification.Action.Builder(null, prefs.string("close", label), stop).build())
            .apply {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                    setForegroundServiceBehavior(Notification.FOREGROUND_SERVICE_IMMEDIATE)
                }
            }
            .build()
    }

    companion object {
        private const val TAG = "VoltipBubble"
        private const val CHANNEL_ID = "voltip-bubble"
        private const val NOTIFICATION_ID = 0x766c
        private const val ACTION_STOP = "dev.voltip.rn.bubble.STOP"

        /** The app asked the service to run, and it has not stopped since. */
        @Volatile
        private var running = false

        /** The microphone and 显示在其他应用上层: without either the bubble cannot do its work. */
        private fun canRun(context: Context): Boolean =
            ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED &&
                Settings.canDrawOverlays(context)

        /** Show or hide the bubble as its options say: it runs once enabled with what it needs.
         *  The app calls this while it is in front. */
        fun apply(context: Context) {
            val app = context.applicationContext
            val intent = Intent(app, BubbleService::class.java)
            if (BubblePrefs(app).enabled && canRun(app)) {
                try {
                    ContextCompat.startForegroundService(app, intent)
                    running = true
                } catch (e: RuntimeException) {
                    // ForegroundServiceStartNotAllowedException: the app was not in front after all.
                    Log.w(TAG, "the bubble did not start: ${e.message}")
                    running = false
                }
            } else {
                app.stopService(intent)
                running = false
            }
        }

        /** `BubbleStatus` (src/backend/bubble.tsx). */
        fun status(context: Context): JSONObject {
            val app = context.applicationContext
            val prefs = BubblePrefs(app)
            val notifications = app.getSystemService(NotificationManager::class.java).areNotificationsEnabled()
            val permissions = JSONObject()
                .put("microphone", ContextCompat.checkSelfPermission(app, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED)
                .put("overlay", Settings.canDrawOverlays(app))
                .put("notifications", notifications)
            return JSONObject()
                .put("enabled", prefs.enabled)
                .put("running", running)
                .put("permissions", permissions)
                .put("mode", prefs.mode)
                .put("showResult", prefs.showResult)
        }
    }
}
