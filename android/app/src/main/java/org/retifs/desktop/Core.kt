package org.retifs.desktop

import android.content.Context
import java.io.File

/**
 * The shared core (Rust, `android/jni`), the same as the GTK and Windows
 * front ends use. Every call blocks: call it off the main thread. Answers
 * are JSON; errors are a string, "" when fine.
 */
object Core {
    init {
        System.loadLibrary("retifs_desktop")
    }

    external fun init(home: String, store: String, prnsd: String)
    external fun status(): String
    external fun reticulum(): String
    external fun sharedInstanceUp(): Boolean
    external fun addTcp(name: String, host: String, port: Int): String
    external fun setInterface(name: String, enabled: Boolean): String
    external fun removeInterface(name: String): String
    external fun settings(): String
    external fun saveSettings(values: String): String
    external fun heard(): String
    external fun join(address: String): String
    external fun leave(ns: String): String
    external fun create(name: String, public: Boolean): String
    external fun keep(ns: String, all: Boolean): String
    external fun favorite(ns: String, on: Boolean): String
    external fun announce(ns: String, on: Boolean): String
    /** A library's link, or "error: …". */
    external fun link(ns: String): String
    external fun members(ns: String): String
    external fun invites(ns: String): String
    /** An invitation's link (role: read, write, invite), or "error: …". */
    external fun invite(ns: String, role: String, days: Int): String
    external fun cancelInvite(ns: String, id: String): String
    external fun removeMember(ns: String, writer: String): String

    /** Where everything lives for this app. */
    class Paths(context: Context) {
        val home: File = context.filesDir
        val store: File = File(context.filesDir, "retifs")
        val logs: File = File(context.filesDir, "logs")
        private val native = File(context.applicationInfo.nativeLibraryDir)
        val retifsd: File = File(native, "libretifsd.so")
        val prnsd: File = File(native, "libprnsd.so")
    }

    @Volatile private var ready = false

    /** Sets the core's paths once per process. */
    fun ready(context: Context) {
        if (ready) return
        val p = Paths(context)
        p.store.mkdirs()
        p.logs.mkdirs()
        init(p.home.absolutePath, p.store.absolutePath, p.prnsd.absolutePath)
        ready = true
    }
}
