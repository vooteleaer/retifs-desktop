package org.retifs.desktop

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import android.os.IBinder
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/**
 * Keeps retifs running: Reticulum (the bundled prnsd) where no other app
 * runs a shared instance, then retifsd, both as programs from the app's
 * native libraries, restarted if they stop. Its notification is Android's
 * tray: the same state as the desktop icon, with Open and Stop.
 */
class RetifsService : Service() {
    private var prnsd: Process? = null
    private var retifsd: Process? = null
    @Volatile private var running = false
    private var supervisor: Thread? = null
    private var multicast: WifiManager.MulticastLock? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_STOP -> {
                stopSelf()
                return START_NOT_STICKY
            }
            ACTION_RESTART -> {
                retifsd?.destroy()
                return START_STICKY
            }
            ACTION_RESTART_RETICULUM -> {
                // retifsd follows: its link to the shared instance goes with it.
                prnsd?.destroy()
                retifsd?.destroy()
                supervisor?.interrupt()
                supervisor = Thread(::supervise, "retifs-supervisor").apply { start() }
                return START_STICKY
            }
        }
        if (running) return START_STICKY
        Core.ready(this)
        channel()
        val notification = notification("Starting…")
        if (Build.VERSION.SDK_INT >= 34) {
            startForeground(NOTIFICATION, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
        } else {
            startForeground(NOTIFICATION, notification)
        }
        // AutoInterface: others on the same Wi-Fi.
        multicast = (applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager)
            .createMulticastLock("retifs").apply { setReferenceCounted(false); acquire() }
        running = true
        supervisor = Thread(::supervise, "retifs-supervisor").apply { start() }
        return START_STICKY
    }

    override fun onDestroy() {
        running = false
        supervisor?.interrupt()
        retifsd?.destroy()
        prnsd?.destroy()
        multicast?.release()
        super.onDestroy()
    }

    private fun start(program: File, name: String, vararg args: String): Process {
        val p = Core.Paths(this)
        return ProcessBuilder(listOf(program.absolutePath) + args)
            .apply {
                environment()["HOME"] = p.home.absolutePath
                environment()["RETIFS_STORE"] = p.store.absolutePath
                redirectErrorStream(true)
                redirectOutput(ProcessBuilder.Redirect.appendTo(File(p.logs, "$name.log")))
            }
            .start()
    }

    /**
     * Reticulum's settings at first start. The shared instance on localhost
     * TCP (as on Windows): on Android Prns's clients look for it there, and
     * other apps (Sideband) can share it too.
     */
    private fun reticulumConfig(home: File) {
        val dir = File(home, ".reticulum")
        val config = File(dir, "config")
        if (config.exists()) return
        dir.mkdirs()
        config.writeText(
            """
            |[reticulum]
            |  share_instance = Yes
            |  shared_instance_type = tcp
            |  shared_instance_port = 37428
            |  instance_control_port = 37429
            |  enable_transport = No
            |
            |[interfaces]
            |  [[Default Interface]]
            |    type = AutoInterface
            |    enabled = Yes
            |""".trimMargin(),
        )
    }

    /** Starts what's missing, every few seconds, and says how it is. */
    private fun supervise() {
        val p = Core.Paths(this)
        reticulumConfig(p.home)
        var firstStart = true
        while (running) {
            try {
                if (prnsd?.isAlive != true && !Core.sharedInstanceUp()) {
                    // A new shared instance: retifsd's link to the old one is gone,
                    // and it doesn't attach again by itself.
                    retifsd?.destroy()
                    prnsd = start(p.prnsd, "prnsd", "run")
                    for (i in 0 until 30) {
                        if (Core.sharedInstanceUp()) break
                        Thread.sleep(500)
                    }
                }
                if (retifsd?.isAlive != true) {
                    retifsd = start(p.retifsd, "retifsd")
                    if (firstStart) phoneDefaults()
                    firstStart = false
                }
                update()
                Thread.sleep(15_000)
            } catch (e: InterruptedException) {
                return
            } catch (e: Exception) {
                update("retifs: ${e.message}")
                Thread.sleep(15_000)
            }
        }
    }

    /**
     * A phone is a small node (retifs ADR-0031): it holds nothing for others
     * unless the user says so, and keeps 1 GB free rather than the desktop's
     * 5 GB (a phone often has less than that free at all). Named after the
     * model. Set once, in its settings, which the user can change.
     */
    private fun phoneDefaults() {
        val prefs = getSharedPreferences("retifs", MODE_PRIVATE)
        if (prefs.getInt("phone-defaults-version", 0) >= PHONE_DEFAULTS) return
        Thread.sleep(3_000) // retifsd writes its settings file at first start
        val defaults = JSONObject()
            .put("placement.hold_for_others", "0")
            .put("store.min_free", "1G")
        val current = try { JSONArray(Core.settings()) } catch (e: Exception) { JSONArray() }
        for (i in 0 until current.length()) {
            val s = current.getJSONObject(i)
            if (s.optString("key") == "device" && s.isNull("value")) {
                defaults.put("node.device", Build.MODEL.lowercase().replace(Regex("[^a-z0-9]+"), "-").trim('-'))
            }
        }
        if (Core.saveSettings(defaults.toString()).isEmpty()) {
            prefs.edit().putInt("phone-defaults-version", PHONE_DEFAULTS).apply()
            retifsd?.destroy() // restarted by the loop, with the setting
        }
    }

    private fun update(note: String? = null) {
        val text = note ?: try {
            val s = JSONObject(Core.status())
            val r = JSONObject(Core.reticulum())
            val links = r.optJSONArray("connections")
            var up = 0
            for (i in 0 until (links?.length() ?: 0)) if (links!!.getJSONObject(i).optBoolean("up")) up++
            val retifs = if (s.optBoolean("up")) "retifs running" else "retifs starting"
            val ret = if (r.optBoolean("running")) "Reticulum: $up of ${links?.length() ?: 0} connection(s) up" else "no Reticulum"
            "$retifs · $ret"
        } catch (e: Exception) {
            "retifs: ${e.message}"
        }
        getSystemService(NotificationManager::class.java).notify(NOTIFICATION, notification(text))
    }

    private fun channel() {
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel(CHANNEL, "retifs running", NotificationManager.IMPORTANCE_LOW))
    }

    private fun notification(text: String): Notification {
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
        val stop = PendingIntent.getService(this, 1, Intent(this, RetifsService::class.java).setAction(ACTION_STOP), PendingIntent.FLAG_IMMUTABLE)
        return Notification.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_status)
            .setContentTitle("retifs")
            .setContentText(text)
            .setContentIntent(open)
            .setOngoing(true)
            .addAction(Notification.Action.Builder(null, "Stop", stop).build())
            .build()
    }

    companion object {
        const val CHANNEL = "retifs"
        const val NOTIFICATION = 1
        /** Raised when phoneDefaults() gains a setting, so existing installs get it. */
        const val PHONE_DEFAULTS = 2
        const val ACTION_STOP = "org.retifs.desktop.STOP"
        const val ACTION_RESTART = "org.retifs.desktop.RESTART"
        const val ACTION_RESTART_RETICULUM = "org.retifs.desktop.RESTART_RETICULUM"

        fun start(context: Context) {
            context.startForegroundService(Intent(context, RetifsService::class.java))
        }

        fun restartDaemon(context: Context) {
            context.startService(Intent(context, RetifsService::class.java).setAction(ACTION_RESTART))
        }

        /** After a change to Reticulum's connections. */
        fun restartReticulum(context: Context) {
            context.startService(Intent(context, RetifsService::class.java).setAction(ACTION_RESTART_RETICULUM))
        }

        fun stop(context: Context) {
            context.startService(Intent(context, RetifsService::class.java).setAction(ACTION_STOP))
        }
    }
}
