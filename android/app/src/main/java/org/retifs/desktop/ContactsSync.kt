package org.retifs.desktop

import android.accounts.Account
import android.app.Service
import android.content.AbstractThreadedSyncAdapter
import android.content.ContentProviderClient
import android.content.ContentProviderOperation
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.content.SyncResult
import android.net.Uri
import android.os.Bundle
import android.os.IBinder
import android.provider.ContactsContract
import android.provider.ContactsContract.CommonDataKinds.Email
import android.provider.ContactsContract.CommonDataKinds.Event
import android.provider.ContactsContract.CommonDataKinds.Nickname
import android.provider.ContactsContract.CommonDataKinds.Note
import android.provider.ContactsContract.CommonDataKinds.Organization
import android.provider.ContactsContract.CommonDataKinds.Phone
import android.provider.ContactsContract.CommonDataKinds.StructuredName
import android.provider.ContactsContract.CommonDataKinds.StructuredPostal
import android.provider.ContactsContract.CommonDataKinds.Website
import android.provider.ContactsContract.Data
import android.provider.ContactsContract.RawContacts
import android.util.Log
import org.json.JSONArray
import org.json.JSONObject
import java.security.MessageDigest

/** The contacts sync adapter's service: the system binds it to sync. */
open class ContactsSyncService : Service() {
    override fun onBind(intent: Intent?): IBinder = adapter(this).syncAdapterBinder

    companion object {
        private var instance: ContactsSyncAdapter? = null

        @Synchronized
        fun adapter(context: Context): ContactsSyncAdapter = instance ?: ContactsSyncAdapter(context.applicationContext).also { instance = it }
    }
}

class ContactsSyncAdapter(context: Context) : AbstractThreadedSyncAdapter(context, true, true) {
    override fun onPerformSync(account: Account, extras: Bundle, authority: String, provider: ContentProviderClient, result: SyncResult) {
        Core.ready(context)
        try {
            if (account.type == Accounts.BOOK) {
                val writable = android.accounts.AccountManager.get(context).getUserData(account, Accounts.WRITABLE) == "1"
                ContactsSync(account, provider, result, writable).run()
            }
        } catch (e: Exception) {
            Log.w(ContactsSync.TAG, "contacts sync of ${account.name}: ${e.message}", e)
            result.stats.numIoExceptions++
        }
    }
}

/**
 * One library's contacts (ADR-0024, retifs-contacts' records) as an address
 * book of the system's Contacts: the account named after it. Both ways when
 * the library may be written here. Each contact is a raw contact whose
 * SOURCE_ID is "<library>/<record id>" and SYNC1 a hash of its values as
 * last synced, so a change on either side is seen:
 *
 * - changed in retifs, not on the phone: the phone's rows are rewritten;
 * - edited on the phone (DIRTY): written to retifs (a read-only library's
 *   address book offers no editing; should one be edited anyway, retifs'
 *   version comes back); if both changed, the phone's edit is the newer one;
 * - made in this address book: a new contact in its library; deleted on the
 *   phone: deleted in retifs; gone in retifs: gone from the phone.
 *
 * Keys the phone doesn't show (`retifs:node`, `lxmf`, an app's own) stay as
 * they are in retifs.
 */
class ContactsSync(private val account: Account, private val provider: ContentProviderClient, private val result: SyncResult, writable: Boolean) {
    private val ops = ArrayList<ContentProviderOperation>()
    private val library = account.name
    private val writable = mapOf(library to writable)

    fun run() {
        val remote = LinkedHashMap<String, JSONObject>()
        val list = parse(Core.contacts(JSONArray().put(library).toString())) as JSONArray
        for (i in 0 until list.length()) {
            val c = list.getJSONObject(i)
            if (!c.isNull("id")) remote["${c.getString("ns")}/${c.getString("id")}"] = c
        }
        visible()
        val seen = HashSet<String>()
        provider.query(sync(RawContacts.CONTENT_URI), arrayOf(RawContacts._ID, RawContacts.SOURCE_ID, RawContacts.DIRTY, RawContacts.DELETED, RawContacts.SYNC1), "${RawContacts.ACCOUNT_TYPE}=? AND ${RawContacts.ACCOUNT_NAME}=?", arrayOf(account.type, account.name), null)?.use { c ->
            while (c.moveToNext()) {
                val raw = c.getLong(0)
                val sid = c.getString(1)
                val dirty = c.getInt(2) != 0
                val deleted = c.getInt(3) != 0
                val synced = c.getString(4)
                val ns = sid?.substringBefore('/')
                val rc = sid?.let { remote[it] }
                when {
                    deleted -> {
                        if (rc != null && writable[ns] == true) error(Core.contactDelete(ns!!, sid.substringAfter('/')))
                        ops += ContentProviderOperation.newDelete(sync(ContentUris(RawContacts.CONTENT_URI, raw))).build()
                        result.stats.numDeletes++
                    }
                    sid == null -> if (dirty && writable[library] == true) {
                        // Made in this address book: a new contact in its library.
                        val values = read(raw)
                        if (values.length() > 0) {
                            val saved = parse(Core.contactSave(JSONObject().put("ns", library).put("values", values).toString(), "null")) as JSONObject
                            val newSid = "$library/${saved.getString("id")}"
                            seen += newSid
                            ops += ContentProviderOperation.newUpdate(sync(ContentUris(RawContacts.CONTENT_URI, raw)))
                                .withValue(RawContacts.SOURCE_ID, newSid).withValue(RawContacts.SYNC1, hash(saved.getJSONObject("values"))).withValue(RawContacts.DIRTY, 0).build()
                            result.stats.numInserts++
                        }
                    }
                    rc == null -> {
                        // Gone in retifs (or its library left): gone here.
                        ops += ContentProviderOperation.newDelete(sync(ContentUris(RawContacts.CONTENT_URI, raw))).build()
                        result.stats.numDeletes++
                    }
                    dirty && writable[ns] == true -> {
                        seen += sid
                        val before = rc.getJSONObject("values")
                        if (hash(before) != synced) Log.i(TAG, "$sid changed on the phone and in retifs: the phone's edit is kept")
                        val values = JSONObject()
                        before.keys().forEach { k -> if (!mapped(k)) values.put(k, before.getString(k)) }
                        val local = read(raw)
                        local.keys().forEach { k -> values.put(k, local.getString(k)) }
                        val saved = parse(Core.contactSave(JSONObject(rc.toString()).put("values", values).toString(), rc.toString())) as JSONObject
                        ops += ContentProviderOperation.newUpdate(sync(ContentUris(RawContacts.CONTENT_URI, raw)))
                            .withValue(RawContacts.SYNC1, hash(saved.getJSONObject("values"))).withValue(RawContacts.DIRTY, 0).build()
                        result.stats.numUpdates++
                    }
                    dirty || hash(rc.getJSONObject("values")) != synced -> {
                        // Changed in retifs (or edited here in a library only read here): retifs' version.
                        seen += sid
                        rewrite(raw, rc)
                        result.stats.numUpdates++
                    }
                    else -> seen += sid
                }
                flush(false)
            }
        }
        for ((sid, rc) in remote) {
            if (sid in seen) continue
            insert(sid, rc)
            result.stats.numInserts++
            flush(false)
        }
        flush(true)
    }

    /** Contacts of the account are shown, not only those in a group. */
    private fun visible() {
        val v = ContentValues().apply {
            put(ContactsContract.Settings.ACCOUNT_NAME, account.name)
            put(ContactsContract.Settings.ACCOUNT_TYPE, account.type)
            put(ContactsContract.Settings.UNGROUPED_VISIBLE, 1)
            put(ContactsContract.Settings.SHOULD_SYNC, 1)
        }
        provider.insert(sync(ContactsContract.Settings.CONTENT_URI), v)
    }

    private fun insert(sid: String, rc: JSONObject) {
        val back = ops.size
        ops += ContentProviderOperation.newInsert(sync(RawContacts.CONTENT_URI))
            .withValue(RawContacts.ACCOUNT_TYPE, account.type).withValue(RawContacts.ACCOUNT_NAME, account.name)
            .withValue(RawContacts.SOURCE_ID, sid).withValue(RawContacts.SYNC1, hash(rc.getJSONObject("values")))
            // A library only read here: its contacts shown, not offered for editing.
            .withValue(RawContacts.RAW_CONTACT_IS_READ_ONLY, if (writable[library] == true) 0 else 1).build()
        rows(rc.getJSONObject("values")).forEach { row ->
            ops += ContentProviderOperation.newInsert(sync(Data.CONTENT_URI)).withValueBackReference(Data.RAW_CONTACT_ID, back).withValues(row).build()
        }
    }

    private fun rewrite(raw: Long, rc: JSONObject) {
        ops += ContentProviderOperation.newDelete(sync(Data.CONTENT_URI)).withSelection("${Data.RAW_CONTACT_ID}=?", arrayOf(raw.toString())).build()
        rows(rc.getJSONObject("values")).forEach { row ->
            ops += ContentProviderOperation.newInsert(sync(Data.CONTENT_URI)).withValue(Data.RAW_CONTACT_ID, raw).withValues(row).build()
        }
        ops += ContentProviderOperation.newUpdate(sync(ContentUris(RawContacts.CONTENT_URI, raw)))
            .withValue(RawContacts.SYNC1, hash(rc.getJSONObject("values"))).withValue(RawContacts.DIRTY, 0).build()
    }

    private fun flush(all: Boolean) {
        // A batch at a time (the provider limits one), not mid-contact.
        if (ops.isEmpty() || (!all && ops.size < 300)) return
        provider.applyBatch(ops)
        ops.clear()
    }

    /** A raw contact's rows as retifs values (only the keys the phone shows). */
    private fun read(raw: Long): JSONObject {
        val values = JSONObject()
        val multi = HashMap<String, MutableList<Pair<String, String>>>()
        provider.query(sync(Data.CONTENT_URI), arrayOf(Data.MIMETYPE, Data.DATA1, Data.DATA2, Data.DATA3, Data.DATA4, Data.DATA5, Data.DATA6), "${Data.RAW_CONTACT_ID}=?", arrayOf(raw.toString()), null)?.use { c ->
            while (c.moveToNext()) {
                val (mime, d1, d2, d3, d4) = listOf(c.getString(0), c.getString(1), c.getString(2), c.getString(3), c.getString(4))
                val (d5, d6) = c.getString(5) to c.getString(6)
                fun put(k: String, v: String?) { if (!v.isNullOrBlank()) values.put(k, v.trim()) }
                fun add(kind: String, label: String, v: String?) { if (!v.isNullOrBlank()) multi.getOrPut(kind) { mutableListOf() } += label to v.trim() }
                when (mime) {
                    // DATA2 given, DATA3 family, DATA4 prefix, DATA5 middle, DATA6 suffix
                    StructuredName.CONTENT_ITEM_TYPE -> { put("given", d2); put("family", d3); put("prefix", d4); put("middle", d5); put("suffix", d6) }
                    Nickname.CONTENT_ITEM_TYPE -> put("name", d1)
                    Organization.CONTENT_ITEM_TYPE -> { put("org", d1); put("role", d4) } // DATA4 title
                    Event.CONTENT_ITEM_TYPE -> if (d2?.toIntOrNull() == Event.TYPE_BIRTHDAY) put("birthday", d1)
                    Website.CONTENT_ITEM_TYPE -> if (!values.has("url")) put("url", d1)
                    Note.CONTENT_ITEM_TYPE -> put("note", d1)
                    Phone.CONTENT_ITEM_TYPE -> add("phone", phoneLabel(d2?.toIntOrNull(), d3), d1)
                    Email.CONTENT_ITEM_TYPE -> add("email", mailLabel(d2?.toIntOrNull(), d3), d1)
                    StructuredPostal.CONTENT_ITEM_TYPE -> add("address", postalLabel(d2?.toIntOrNull(), d3), d1)
                }
            }
        }
        for ((kind, items) in multi) keys(kind, items).forEach { (k, v) -> values.put(k, v) }
        return values
    }

    /** retifs values as the rows the system's Contacts keeps. */
    private fun rows(values: JSONObject): List<ContentValues> {
        val out = ArrayList<ContentValues>()
        fun row(mime: String, vararg kv: Pair<String, Any?>) = out.add(ContentValues().apply {
            put(Data.MIMETYPE, mime)
            for ((k, v) in kv) when (v) { null -> {}; is Int -> put(k, v); else -> put(k, v.toString()) }
        })
        val get = { k: String -> values.optString(k).takeIf { it.isNotBlank() } }
        // A name row only for first and last names: given a display name
        // alone, the system splits it into those, and the next edit here
        // would bring them to retifs. A petname alone names it as a nickname.
        val full = listOfNotNull(get("prefix"), get("given"), get("middle"), get("family"), get("suffix")).joinToString(" ").ifEmpty { null }
        if (full != null) {
            row(
                StructuredName.CONTENT_ITEM_TYPE, StructuredName.DISPLAY_NAME to full, StructuredName.GIVEN_NAME to get("given"), StructuredName.FAMILY_NAME to get("family"),
                StructuredName.PREFIX to get("prefix"), StructuredName.MIDDLE_NAME to get("middle"), StructuredName.SUFFIX to get("suffix"),
            )
        }
        get("name")?.let { row(Nickname.CONTENT_ITEM_TYPE, Nickname.NAME to it) }
        if (get("org") != null || get("role") != null) row(Organization.CONTENT_ITEM_TYPE, Organization.COMPANY to get("org"), Organization.TITLE to get("role"))
        get("birthday")?.let { row(Event.CONTENT_ITEM_TYPE, Event.START_DATE to it, Event.TYPE to Event.TYPE_BIRTHDAY) }
        get("url")?.let { row(Website.CONTENT_ITEM_TYPE, Website.URL to it) }
        get("note")?.let { row(Note.CONTENT_ITEM_TYPE, Note.NOTE to it) }
        values.keys().forEach { k ->
            val v = values.getString(k).takeIf { it.isNotBlank() } ?: return@forEach
            val (kind, label) = (k.substringBefore(':') to (if (k.contains(':')) k.substringAfter(':') else ""))
            when (kind) {
                "phone" -> { val (t, l) = phoneType(label); row(Phone.CONTENT_ITEM_TYPE, Phone.NUMBER to v, Phone.TYPE to t, Phone.LABEL to l) }
                "email" -> { val (t, l) = mailType(label); row(Email.CONTENT_ITEM_TYPE, Email.ADDRESS to v, Email.TYPE to t, Email.LABEL to l) }
                "address" -> { val (t, l) = postalType(label); row(StructuredPostal.CONTENT_ITEM_TYPE, StructuredPostal.FORMATTED_ADDRESS to v, StructuredPostal.TYPE to t, StructuredPostal.LABEL to l) }
            }
        }
        return out
    }

    private fun sync(uri: Uri): Uri = uri.buildUpon()
        .appendQueryParameter(ContactsContract.CALLER_IS_SYNCADAPTER, "true")
        .appendQueryParameter(RawContacts.ACCOUNT_NAME, account.name)
        .appendQueryParameter(RawContacts.ACCOUNT_TYPE, account.type)
        .build()

    @Suppress("FunctionName")
    private fun ContentUris(base: Uri, id: Long): Uri = android.content.ContentUris.withAppendedId(base, id)

    companion object {
        const val TAG = "retifs-contacts"
        private val SINGLE = setOf("name", "given", "family", "prefix", "middle", "suffix", "org", "role", "birthday", "url", "note")
        private val MULTI = listOf("email", "phone", "address")

        /** A key the phone shows (and so writes back). */
        fun mapped(k: String) = k in SINGLE || MULTI.any { k == it || k.startsWith("$it:") }

        /** The values the phone shows, as one hash: what tells a change. */
        fun hash(values: JSONObject): String {
            val text = values.keys().asSequence().filter(::mapped).sorted().joinToString("\n") { "$it=${values.optString(it).trim()}" }
            return MessageDigest.getInstance("SHA-256").digest(text.toByteArray()).take(12).joinToString("") { "%02x".format(it) }
        }

        /** Labelled values as keys, as retifs-contacts makes them (`phone`, `phone:work`, `phone:work2`). */
        fun keys(kind: String, items: List<Pair<String, String>>): List<Pair<String, String>> {
            val used = HashSet<String>()
            return items.sortedBy { it.first.isNotEmpty() }.map { (label0, value) ->
                val label = label0.trim().lowercase().replace(' ', '-').replace(':', '-')
                var key = if (label.isEmpty()) kind else "$kind:$label"
                var n = 2
                while (!used.add(key)) key = "$kind:${label.ifEmpty { "other" }}${n++}"
                key to value
            }
        }

        private fun custom(label: String?) = label?.trim()?.lowercase().orEmpty()

        // The unlabelled number is a mobile one; the unlabelled address and email "other".
        fun phoneLabel(type: Int?, label: String?) = when (type) {
            Phone.TYPE_MOBILE -> ""; Phone.TYPE_HOME -> "home"; Phone.TYPE_WORK -> "work"; Phone.TYPE_OTHER -> "other"; else -> custom(label)
        }
        fun phoneType(label: String): Pair<Int, String?> = when (label.trimEnd { it.isDigit() }) {
            "", "mobile" -> Phone.TYPE_MOBILE to null; "home" -> Phone.TYPE_HOME to null; "work" -> Phone.TYPE_WORK to null; "other" -> Phone.TYPE_OTHER to null
            else -> Phone.TYPE_CUSTOM to label
        }
        fun mailLabel(type: Int?, label: String?) = when (type) {
            Email.TYPE_OTHER -> ""; Email.TYPE_HOME -> "home"; Email.TYPE_WORK -> "work"; Email.TYPE_MOBILE -> "mobile"; else -> custom(label)
        }
        fun mailType(label: String): Pair<Int, String?> = when (label.trimEnd { it.isDigit() }) {
            "", "other" -> Email.TYPE_OTHER to null; "home" -> Email.TYPE_HOME to null; "work" -> Email.TYPE_WORK to null; "mobile" -> Email.TYPE_MOBILE to null
            else -> Email.TYPE_CUSTOM to label
        }
        fun postalLabel(type: Int?, label: String?) = when (type) {
            StructuredPostal.TYPE_OTHER -> ""; StructuredPostal.TYPE_HOME -> "home"; StructuredPostal.TYPE_WORK -> "work"; else -> custom(label)
        }
        fun postalType(label: String): Pair<Int, String?> = when (label.trimEnd { it.isDigit() }) {
            "", "other" -> StructuredPostal.TYPE_OTHER to null; "home" -> StructuredPostal.TYPE_HOME to null; "work" -> StructuredPostal.TYPE_WORK to null
            else -> StructuredPostal.TYPE_CUSTOM to label
        }

        /** A core answer as JSON, `{"error"}` thrown. */
        fun parse(answer: String): Any {
            val v: Any = if (answer.trimStart().startsWith("[")) JSONArray(answer) else JSONObject(answer)
            if (v is JSONObject && v.has("error")) throw IllegalStateException(v.getString("error"))
            return v
        }

        fun error(e: String) {
            if (e.isNotEmpty()) throw IllegalStateException(e)
        }
    }
}
