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

    external fun init(home: String, store: String)
    external fun status(): String
    /** Whether retifsd answers: a ping, nothing more (status asks about every library). */
    external fun up(): Boolean
    external fun sharedInstanceUp(): Boolean
    external fun settings(): String
    external fun saveSettings(values: String): String
    external fun heard(): String
    external fun join(address: String): String
    /** Joins with an invitation or `retifs:` link; `name` empty: the link's. */
    external fun openLink(link: String, name: String): String

    // Libraries as files (DocumentsProvider). Paths are a library's own ("/a/b.txt").
    /** `["name", …]` or `{"error"}`. */
    external fun docLibraries(): String
    /** `[{name, path, dir, size, mtime}]` or `{"error"}`. */
    external fun docList(ns: String, dir: String): String
    /** One entry, `null`, or `{"error"}`. */
    external fun docStat(ns: String, path: String): String
    external fun docRead(ns: String, path: String, dest: String): String
    external fun docWrite(ns: String, path: String, src: String): String
    external fun docMkdir(ns: String, path: String): String
    external fun docRemove(ns: String, path: String): String
    external fun docRename(ns: String, from: String, to: String): String

    // Contacts for the sync adapter (the system's Contacts).
    /** `[{"ns", "writable"}]` or `{"error"}`. */
    external fun contactLibraries(): String
    /** The contacts in `libraries` (a JSON array): `[{ns, id, values}]` or `{"error"}`. */
    external fun contacts(libraries: String): String
    /** Saves `{ns, id, values}` (`before` as it was, or "null"): as saved, or `{"error"}`. */
    external fun contactSave(contact: String, before: String): String
    external fun contactDelete(ns: String, id: String): String

    /**
     * Waits until one of the libraries in `known` (`{"name": change count or
     * null}`) changes, for at most `timeoutMs`: `{"changed", "known"}`, the
     * counts brought up to date (one retifsd has none for left out).
     */
    external fun waitChange(known: String, timeoutMs: Int): String

    // Events for the calendar sync adapter (the system's Calendar).
    /** `[{"ns", "writable", "color"}]` or `{"error"}`. */
    external fun calendarLibraries(): String
    /** Library `ns`'s events: `[{ns, id, values}]` or `{"error"}`. */
    external fun calendarEvents(ns: String): String
    /** Saves `{ns, id, values}` (`before` as it was, or "null"): as saved, or `{"error"}`. */
    external fun calendarSave(event: String, before: String): String
    external fun calendarDelete(ns: String, id: String): String
    external fun leave(ns: String): String
    /** The recovery kit (ADR-0033): `{words, covered, not_covered}` or `{error}`. */
    external fun recoveryCreate(): String
    /** Typed recovery words checked: `{ok, say}`. */
    external fun recoveryCheck(text: String): String
    /** Recovers from the words (takes minutes): `{libraries}` or `{error}`. */
    external fun recoveryRestore(text: String): String
    /** Deletes a library on every member's device (retifs ADR-0034; only one who may invite). */
    external fun deleteLibrary(ns: String): String
    /** A new library; `purposes` what it's for, "contacts,calendar" (ADR-0032; "": generic). */
    external fun create(name: String, public: Boolean, purposes: String): String
    /** Says what a library is for, as `create` takes it. */
    external fun setPurpose(ns: String, purposes: String): String
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

    /** Lets an app (by its retifs name) use `namespaces` (JSON array): its token, or "error: …". */
    external fun appAdd(name: String, namespaces: String, write: Boolean): String
    external fun appRemove(name: String): String
    /** A connection to retifsd as the app with `token`, past its Hello: a descriptor, or -1. */
    external fun appConnect(token: String): Int

    /** Where everything lives for this app. */
    class Paths(context: Context) {
        val home: File = context.filesDir
        val store: File = File(context.filesDir, "retifs")
        val logs: File = File(context.filesDir, "logs")
        private val native = File(context.applicationInfo.nativeLibraryDir)
        val retifsd: File = File(native, "libretifsd.so")
    }

    @Volatile private var ready = false

    /** Sets the core's paths once per process. */
    fun ready(context: Context) {
        if (ready) return
        val p = Paths(context)
        p.store.mkdirs()
        p.logs.mkdirs()
        init(p.home.absolutePath, p.store.absolutePath)
        ready = true
    }
}
