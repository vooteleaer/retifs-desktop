package org.retifs.desktop

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/**
 * Keeps retifsd running (a program from the app's native libraries),
 * restarted if it stops. Reticulum is Prns's app (or another one with a
 * shared instance): retifsd joins it once it's up, as on the desktop. Its notification is Android's
 * tray: the same state as the desktop icon, with Open and Stop.
 */
class RetifsService : Service() {
    private var retifsd: Process? = null
    @Volatile private var running = false
    /** Wakes the supervisor before its next round (Restart). */
    private val wake = java.util.concurrent.Semaphore(0)
    private var supervisor: Thread? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_STOP -> {
                stopSelf()
                return START_NOT_STICKY
            }
            ACTION_RESTART -> {
                restarting = true
                lastExit = null
                // Started again at once, not at the supervisor's next round.
                val old = retifsd
                Thread {
                    old?.destroy()
                    old?.waitFor()
                    wake.release()
                }.start()
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
        running = true
        supervisor = Thread(::supervise, "retifs-supervisor").apply { start() }
        // The address books in the system's Contacts, after a boot or an
        // update too (once the user has allowed it).
        val calendarAllowed = checkSelfPermission(android.Manifest.permission.WRITE_CALENDAR) == android.content.pm.PackageManager.PERMISSION_GRANTED
        if (contactsAllowed() || calendarAllowed) Accounts.ensure(this)
        if (contactsAllowed()) Accounts.syncContacts(this)
        if (calendarAllowed) Accounts.syncCalendars()
        return START_STICKY
    }

    private fun contactsAllowed() = checkSelfPermission(android.Manifest.permission.WRITE_CONTACTS) == android.content.pm.PackageManager.PERMISSION_GRANTED

    override fun onDestroy() {
        running = false
        daemonAlive = false
        supervisor?.interrupt()
        retifsd?.destroy()
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
     * The libraries a new install joins (retifs' packaging/default-libraries,
     * an asset of the app), as a file retifsd reads: it joins each once.
     */
    private fun defaultLibraries(): File {
        val f = File(filesDir, "default-libraries")
        runCatching { assets.open("default-libraries").use { input -> f.outputStream().use { input.copyTo(it) } } }
        return f
    }

    private fun prnsApp(): Boolean = try {
        packageManager.getPackageInfo(PRNS_APP, 0)
        true
    } catch (e: Exception) {
        false
    }

    /** Starts what's missing, every few seconds, and says how it is. */
    private fun supervise() {
        val p = Core.Paths(this)
        var firstStart = true
        var booksAt = System.currentTimeMillis()
        var downs = 0
        while (running) {
            try {
                // A library that came (or went) with contacts: its address book, hourly.
                if (System.currentTimeMillis() - booksAt > 3_600_000 && contactsAllowed()) {
                    booksAt = System.currentTimeMillis()
                    Accounts.syncContacts(this)
                }
                // At boot retifs may come before Prns: retifsd starts once the
                // shared instance is up (it can't join one that isn't).
                val up = Core.sharedInstanceUp()
                // Reticulum went away and came back (Prns restarted): retifsd
                // doesn't attach to a new shared instance by itself. Away for
                // two checks running: one missed check is a busy phone.
                if (up && downs >= 2 && retifsd?.isAlive == true) {
                    restarting = true
                    retifsd?.destroy()
                    retifsd?.waitFor()
                }
                downs = if (up) 0 else downs + 1
                // Stopped by itself (not by Restart): why, for the app to show.
                retifsd?.let { old ->
                    if (!old.isAlive && !restarting) lastExit = "exit code ${old.exitValue()}" + logTail(File(p.logs, "retifsd.log")).let { if (it.isEmpty()) "" else ":\n$it" }
                }
                if (retifsd?.isAlive != true && up) {
                    restarting = false
                    retifsd = start(p.retifsd, "retifsd", "--default-libraries", defaultLibraries().absolutePath)
                    if (firstStart) phoneDefaults()
                    firstStart = false
                }
                daemonAlive = retifsd?.isAlive == true
                waitingForReticulum = !up
                when {
                    up -> update()
                    prnsApp() -> update("waiting for Reticulum (Prns)")
                    else -> update("no Reticulum: install Prns")
                }
                wake.tryAcquire(if (retifsd?.isAlive == true) 15_000L else 3_000L, java.util.concurrent.TimeUnit.MILLISECONDS)
                wake.drainPermits()
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
            restarting = true
            retifsd?.destroy() // restarted by the loop, with the setting
        }
    }

    /** The last lines of a log, for saying why retifsd stopped. */
    private fun logTail(f: File, lines: Int = 4): String =
        runCatching { f.readLines().takeLast(lines).joinToString("\n") }.getOrDefault("")

    private fun update(note: String? = null) {
        val text = note ?: try {
            val s = JSONObject(Core.status())
            when {
                s.optBoolean("up") -> "retifs running"
                else -> s.optJSONObject("starting")?.optString("text")?.let { "retifs starting: $it" } ?: "retifs starting"
            }
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
        const val PRNS_APP = "org.personal.hopspot"
        const val ACTION_STOP = "org.retifs.desktop.STOP"
        const val ACTION_RESTART = "org.retifs.desktop.RESTART"

        /** Whether retifsd's process runs (it may not answer yet: starting). */
        @Volatile var daemonAlive = false
        /** Waiting for Reticulum before retifsd can start. */
        @Volatile var waitingForReticulum = false
        /** Why retifsd last stopped by itself: its exit and last log lines. */
        @Volatile var lastExit: String? = null
        @Volatile private var restarting = false

        fun start(context: Context) {
            context.startForegroundService(Intent(context, RetifsService::class.java))
        }

        fun restartDaemon(context: Context) {
            context.startService(Intent(context, RetifsService::class.java).setAction(ACTION_RESTART))
        }

        fun stop(context: Context) {
            context.startService(Intent(context, RetifsService::class.java).setAction(ACTION_STOP))
        }
    }
}
