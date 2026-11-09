package org.retifs.desktop

import android.accounts.Account
import android.app.Service
import android.content.AbstractThreadedSyncAdapter
import android.content.ContentProviderClient
import android.content.ContentUris
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.content.SyncResult
import android.database.Cursor
import android.net.Uri
import android.os.Bundle
import android.os.IBinder
import android.provider.CalendarContract
import android.provider.CalendarContract.Calendars
import android.provider.CalendarContract.Events
import android.util.Log
import org.json.JSONArray
import org.json.JSONObject
import java.security.MessageDigest
import java.time.Instant
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.ZoneId
import java.time.ZoneOffset
import java.time.format.DateTimeFormatter

/** The calendar sync adapter's service: the system binds it to sync. */
class CalendarSyncService : Service() {
    override fun onBind(intent: Intent?): IBinder = adapter(this).syncAdapterBinder

    companion object {
        private var instance: CalendarSyncAdapter? = null

        @Synchronized
        fun adapter(context: Context): CalendarSyncAdapter = instance ?: CalendarSyncAdapter(context.applicationContext).also { instance = it }
    }
}

class CalendarSyncAdapter(context: Context) : AbstractThreadedSyncAdapter(context, true, false) {
    override fun onPerformSync(account: Account, extras: Bundle, authority: String, provider: ContentProviderClient, result: SyncResult) {
        Core.ready(context)
        try {
            CalendarSync(account, provider, result).run()
        } catch (e: Exception) {
            Log.w(CalendarSync.TAG, "calendar sync: ${e.message}", e)
            result.stats.numIoExceptions++
        }
    }
}

/**
 * retifs calendars (ADR-0024, retifs-calendar's events) in the system's
 * Calendar, under the "retifs" account: a calendar per library that is one
 * (holding events, a `retifs.calendar` record, or made for it, ADR-0032),
 * owned if this device may write it, else read only. An event's _SYNC_ID is
 * its record id, SYNC_DATA1 a hash of its values as last synced, so a change
 * on either side is seen, as for contacts (ContactsSync): the phone's edit
 * goes to retifs, retifs' changes come to the phone, the phone's newer edit
 * wins. A repeating event is one row with its rule; the system shows each
 * time. Keys the phone doesn't show (attendees, `uid`) stay in retifs.
 *
 * retifs keeps local times without a zone (`2026-11-04T18:00`) and all-day
 * dates (`2026-11-04`): this phone's zone is theirs.
 */
class CalendarSync(private val account: Account, private val provider: ContentProviderClient, private val result: SyncResult) {
    private val zone = ZoneId.systemDefault()

    fun run() {
        val libs = parse(Core.calendarLibraries()) as JSONArray
        val want = LinkedHashMap<String, JSONObject>()
        for (i in 0 until libs.length()) libs.getJSONObject(i).let { want[it.getString("ns")] = it }
        val calendars = calendars(want)
        for ((ns, id) in calendars) {
            events(ns, id, want[ns]!!.getBoolean("writable"))
        }
    }

    /** A calendar row per library (made, updated, removed with its events): library → row id. */
    private fun calendars(want: Map<String, JSONObject>): Map<String, Long> {
        val have = HashMap<String, Long>()
        provider.query(sync(Calendars.CONTENT_URI), arrayOf(Calendars._ID, Calendars._SYNC_ID), "${Calendars.ACCOUNT_TYPE}=? AND ${Calendars.ACCOUNT_NAME}=?", arrayOf(account.type, account.name), null)?.use { c ->
            while (c.moveToNext()) have[c.getString(1) ?: ""] = c.getLong(0)
        }
        for ((ns, id) in have) {
            if (ns !in want) {
                provider.delete(sync(ContentUris.withAppendedId(Calendars.CONTENT_URI, id)), null, null)
                Log.i(TAG, "calendar $ns removed")
            }
        }
        val out = LinkedHashMap<String, Long>()
        for ((ns, l) in want) {
            val v = ContentValues().apply {
                put(Calendars.NAME, ns)
                put(Calendars.CALENDAR_DISPLAY_NAME, ns)
                put(Calendars.CALENDAR_COLOR, color(ns, l.optString("color")))
                put(Calendars.CALENDAR_ACCESS_LEVEL, if (l.getBoolean("writable")) Calendars.CAL_ACCESS_OWNER else Calendars.CAL_ACCESS_READ)
                put(Calendars.VISIBLE, 1)
                put(Calendars.SYNC_EVENTS, 1)
            }
            val id = have[ns]
            if (id != null) {
                provider.update(sync(ContentUris.withAppendedId(Calendars.CONTENT_URI, id)), v, null, null)
                out[ns] = id
            } else {
                v.put(Calendars.ACCOUNT_NAME, account.name)
                v.put(Calendars.ACCOUNT_TYPE, account.type)
                v.put(Calendars._SYNC_ID, ns)
                v.put(Calendars.OWNER_ACCOUNT, account.name)
                v.put(Calendars.CALENDAR_TIME_ZONE, zone.id)
                val uri = provider.insert(sync(Calendars.CONTENT_URI), v) ?: continue
                out[ns] = ContentUris.parseId(uri)
                Log.i(TAG, "calendar $ns added")
            }
        }
        return out
    }

    private fun events(ns: String, calendar: Long, writable: Boolean) {
        val remote = LinkedHashMap<String, JSONObject>()
        val list = parse(Core.calendarEvents(ns)) as JSONArray
        for (i in 0 until list.length()) {
            val e = list.getJSONObject(i)
            if (!e.isNull("id")) remote[e.getString("id")] = e
        }
        val seen = HashSet<String>()
        val cols = arrayOf(Events._ID, Events._SYNC_ID, Events.DIRTY, Events.DELETED, Events.SYNC_DATA1, Events.ORIGINAL_ID) + FIELDS
        provider.query(sync(Events.CONTENT_URI), cols, "${Events.CALENDAR_ID}=?", arrayOf(calendar.toString()), null)?.use { c ->
            while (c.moveToNext()) {
                val row = c.getLong(0)
                val sid = c.getString(1)
                val dirty = c.getInt(2) != 0
                val deleted = c.getInt(3) != 0
                val synced = c.getString(4)
                // One time of a repeating event changed on the phone: not kept apart in retifs yet.
                if (!c.isNull(5)) continue
                val re = sid?.let { remote[it] }
                when {
                    deleted -> {
                        if (re != null && writable) error(Core.calendarDelete(ns, sid))
                        provider.delete(sync(ContentUris.withAppendedId(Events.CONTENT_URI, row)), null, null)
                        result.stats.numDeletes++
                    }
                    sid == null -> if (dirty && writable) {
                        // Made on the phone in this calendar: a new event in its library.
                        val saved = parse(Core.calendarSave(JSONObject().put("ns", ns).put("values", read(c)).toString(), "null")) as JSONObject
                        seen += saved.getString("id")
                        provider.update(sync(ContentUris.withAppendedId(Events.CONTENT_URI, row)), ContentValues().apply {
                            put(Events._SYNC_ID, saved.getString("id")); put(Events.SYNC_DATA1, hash(saved.getJSONObject("values"))); put(Events.DIRTY, 0)
                        }, null, null)
                        result.stats.numInserts++
                    }
                    re == null -> {
                        provider.delete(sync(ContentUris.withAppendedId(Events.CONTENT_URI, row)), null, null)
                        result.stats.numDeletes++
                    }
                    dirty && writable -> {
                        seen += sid
                        val before = re.getJSONObject("values")
                        if (hash(before) != synced) Log.i(TAG, "$ns/$sid changed on the phone and in retifs: the phone's edit is kept")
                        val values = JSONObject()
                        before.keys().forEach { k -> if (k !in MAPPED) values.put(k, before.getString(k)) }
                        val local = read(c)
                        local.keys().forEach { k -> values.put(k, local.getString(k)) }
                        val saved = parse(Core.calendarSave(JSONObject(re.toString()).put("values", values).toString(), re.toString())) as JSONObject
                        provider.update(sync(ContentUris.withAppendedId(Events.CONTENT_URI, row)), ContentValues().apply {
                            put(Events.SYNC_DATA1, hash(saved.getJSONObject("values"))); put(Events.DIRTY, 0)
                        }, null, null)
                        result.stats.numUpdates++
                    }
                    dirty || hash(re.getJSONObject("values")) != synced -> {
                        seen += sid
                        val v = row(re.getJSONObject("values")) ?: continue
                        v.put(Events.SYNC_DATA1, hash(re.getJSONObject("values")))
                        v.put(Events.DIRTY, 0)
                        provider.update(sync(ContentUris.withAppendedId(Events.CONTENT_URI, row)), v, null, null)
                        result.stats.numUpdates++
                    }
                    else -> seen += sid
                }
            }
        }
        for ((id, re) in remote) {
            if (id in seen) continue
            val v = row(re.getJSONObject("values")) ?: continue // no start: not an event the phone can show
            v.put(Events.CALENDAR_ID, calendar)
            v.put(Events._SYNC_ID, id)
            v.put(Events.SYNC_DATA1, hash(re.getJSONObject("values")))
            provider.insert(sync(Events.CONTENT_URI), v)
            result.stats.numInserts++
        }
    }

    /** An event's values as a row of the system's Calendar; null without a start. */
    private fun row(values: JSONObject): ContentValues? {
        val start = values.optString("start").trim()
        val end = values.optString("end").trim()
        val rrule = values.optString("rrule").trim()
        val v = ContentValues()
        v.put(Events.TITLE, values.optString("title"))
        v.put(Events.EVENT_LOCATION, values.optString("location"))
        v.put(Events.DESCRIPTION, values.optString("note"))
        val allDay = !start.contains('T')
        if (allDay) {
            val day = runCatching { LocalDate.parse(start) }.getOrNull() ?: return null
            val last = runCatching { LocalDate.parse(end.substringBefore('T')) }.getOrNull()?.takeIf { it.isAfter(day) } ?: day.plusDays(1)
            v.put(Events.ALL_DAY, 1)
            v.put(Events.EVENT_TIMEZONE, "UTC")
            v.put(Events.DTSTART, day.atStartOfDay(ZoneOffset.UTC).toInstant().toEpochMilli())
            if (rrule.isEmpty()) {
                v.put(Events.DTEND, last.atStartOfDay(ZoneOffset.UTC).toInstant().toEpochMilli())
            } else {
                v.put(Events.DURATION, "P${last.toEpochDay() - day.toEpochDay()}D")
            }
        } else {
            val from = runCatching { LocalDateTime.parse(start) }.getOrNull() ?: return null
            val until = runCatching { LocalDateTime.parse(end) }.getOrNull()?.takeIf { it.isAfter(from) } ?: from.plusHours(1)
            val a = from.atZone(zone).toInstant().toEpochMilli()
            val b = until.atZone(zone).toInstant().toEpochMilli()
            v.put(Events.ALL_DAY, 0)
            v.put(Events.EVENT_TIMEZONE, zone.id)
            v.put(Events.DTSTART, a)
            if (rrule.isEmpty()) v.put(Events.DTEND, b) else v.put(Events.DURATION, "P${(b - a) / 1000}S")
        }
        if (rrule.isEmpty()) v.putNull(Events.RRULE) else { v.put(Events.RRULE, rrule); v.putNull(Events.DTEND) }
        if (rrule.isEmpty()) v.putNull(Events.DURATION)
        return v
    }

    /** A row of the system's Calendar as an event's values (only the keys the phone shows). */
    private fun read(c: Cursor): JSONObject {
        val col = { name: String -> c.getColumnIndexOrThrow(name) }
        val values = JSONObject()
        fun put(k: String, v: String?) { if (!v.isNullOrBlank()) values.put(k, v.trim()) }
        put("title", c.getString(col(Events.TITLE)))
        put("location", c.getString(col(Events.EVENT_LOCATION)))
        put("note", c.getString(col(Events.DESCRIPTION)))
        val rrule = c.getString(col(Events.RRULE))
        put("rrule", rrule)
        val allDay = c.getInt(col(Events.ALL_DAY)) != 0
        val dtstart = c.getLong(col(Events.DTSTART))
        val dtend = if (c.isNull(col(Events.DTEND))) null else c.getLong(col(Events.DTEND))
        val duration = c.getString(col(Events.DURATION))
        val endMillis = dtend ?: duration?.let { dtstart + durationMillis(it) } ?: (dtstart + if (allDay) 86_400_000L else 3_600_000L)
        if (allDay) {
            val day = Instant.ofEpochMilli(dtstart).atZone(ZoneOffset.UTC).toLocalDate()
            val last = Instant.ofEpochMilli(endMillis).atZone(ZoneOffset.UTC).toLocalDate()
            put("start", day.toString())
            // One day: no end, as retifs-calendar writes it.
            if (last.isAfter(day.plusDays(1))) put("end", last.toString())
        } else {
            val tz = c.getString(col(Events.EVENT_TIMEZONE))?.let { runCatching { ZoneId.of(it) }.getOrNull() } ?: zone
            val fmt = DateTimeFormatter.ofPattern("yyyy-MM-dd'T'HH:mm")
            put("start", Instant.ofEpochMilli(dtstart).atZone(tz).withZoneSameInstant(zone).toLocalDateTime().format(fmt))
            put("end", Instant.ofEpochMilli(endMillis).atZone(tz).withZoneSameInstant(zone).toLocalDateTime().format(fmt))
        }
        return values
    }

    private fun sync(uri: Uri): Uri = uri.buildUpon()
        .appendQueryParameter(CalendarContract.CALLER_IS_SYNCADAPTER, "true")
        .appendQueryParameter(Calendars.ACCOUNT_NAME, account.name)
        .appendQueryParameter(Calendars.ACCOUNT_TYPE, account.type)
        .build()

    companion object {
        const val TAG = "retifs-calendar"
        private val FIELDS = arrayOf(Events.TITLE, Events.EVENT_LOCATION, Events.DESCRIPTION, Events.DTSTART, Events.DTEND, Events.DURATION, Events.ALL_DAY, Events.EVENT_TIMEZONE, Events.RRULE)
        /** The keys the phone shows (and so writes back). */
        private val MAPPED = setOf("title", "start", "end", "rrule", "location", "note")

        fun hash(values: JSONObject): String {
            val text = MAPPED.sorted().joinToString("\n") { "$it=${values.optString(it).trim()}" }
            return MessageDigest.getInstance("SHA-256").digest(text.toByteArray()).take(12).joinToString("") { "%02x".format(it) }
        }

        /** An RFC 5545 duration ("P1D", "PT1H30M", "P3600S", "P2W") in milliseconds. */
        fun durationMillis(d: String): Long {
            val m = Regex("""^([+-])?P(?:(\d+)W)?(?:(\d+)D)?(?:T?(?:(\d+)H)?(?:(\d+)M)?(?:(\d+)S)?)?$""").find(d.trim()) ?: return 0
            val (sign, w, days, h, min, s) = m.destructured
            val ms = ((w.toLongOrNull() ?: 0) * 7 * 86_400 + (days.toLongOrNull() ?: 0) * 86_400 + (h.toLongOrNull() ?: 0) * 3_600 + (min.toLongOrNull() ?: 0) * 60 + (s.toLongOrNull() ?: 0)) * 1000
            return if (sign == "-") -ms else ms
        }

        /** "#rrggbb" as a colour, else one made from the name (each library its own). */
        fun color(ns: String, hex: String): Int {
            val h = hex.removePrefix("#")
            if (h.length == 6) h.toLongOrNull(16)?.let { return (0xFF000000 or it).toInt() }
            val hue = (ns.hashCode() and 0x7fffffff) % 360
            return android.graphics.Color.HSVToColor(floatArrayOf(hue.toFloat(), 0.55f, 0.75f))
        }

        fun parse(answer: String): Any = ContactsSync.parse(answer)
        fun error(e: String) = ContactsSync.error(e)
    }
}
