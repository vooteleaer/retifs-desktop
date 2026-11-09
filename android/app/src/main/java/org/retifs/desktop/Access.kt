package org.retifs.desktop

import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Binder
import android.os.Build
import android.os.Bundle
import android.os.IBinder
import android.os.Parcel
import android.os.ParcelFileDescriptor
import android.os.Parcelable
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import org.json.JSONArray
import org.json.JSONObject
import java.security.MessageDigest

/**
 * Other apps on this phone using retifs (retifs-photos, retifs-map): retifsd's
 * sockets are in this app's files, out of their reach, so this app connects
 * for them and hands over the connection (as Prns's hub does for Reticulum).
 *
 * An app is let in once, by the user ([AccessActivity]): the libraries it may
 * use and whether it may change them. That makes it a retifs app with a token
 * of its own (retifs ADR-0004), which stays here: the app only ever gets
 * connections already past their Hello, so it can't take its token
 * elsewhere, and retifsd checks every request against it. An app is known by
 * its package and signing certificate: another app installed later under the
 * same name isn't let in.
 */
object Access {
    const val DESCRIPTOR = "org.retifs.IAccess"
    /** Connect: answers a status, then (on OK) the connection. */
    const val OPEN = IBinder.FIRST_CALL_TRANSACTION
    const val OK = 0
    const val NOT_LET_IN = 1
    const val NOT_RUNNING = 2

    private fun prefs(context: Context) = context.getSharedPreferences("access", Context.MODE_PRIVATE)

    /** What `pkg` was let in to: {token, cert, libraries, write}, or null. */
    fun grant(context: Context, pkg: String): JSONObject? = prefs(context).getString(pkg, null)?.let { runCatching { JSONObject(it) }.getOrNull() }

    fun save(context: Context, pkg: String, grant: JSONObject) {
        prefs(context).edit().putString(pkg, grant.toString()).apply()
    }

    fun forget(context: Context, pkg: String) {
        prefs(context).edit().remove(pkg).apply()
    }

    /** The apps let in: package → grant. */
    fun all(context: Context): Map<String, JSONObject> = prefs(context).all.mapNotNull { (k, v) -> (v as? String)?.let { runCatching { k to JSONObject(it) }.getOrNull() } }.toMap()

    /** The retifs app name of an Android app. */
    fun appName(pkg: String) = "android:$pkg"

    /** The SHA-256 of `pkg`'s signing certificate(s), or null. */
    @Suppress("DEPRECATION")
    fun cert(context: Context, pkg: String): String? = try {
        val sigs = if (Build.VERSION.SDK_INT >= 28) {
            val info = context.packageManager.getPackageInfo(pkg, PackageManager.GET_SIGNING_CERTIFICATES).signingInfo ?: return null
            if (info.hasMultipleSigners()) info.apkContentsSigners else info.signingCertificateHistory
        } else {
            context.packageManager.getPackageInfo(pkg, PackageManager.GET_SIGNATURES).signatures
        }
        val sha = MessageDigest.getInstance("SHA-256")
        sigs?.forEach { sha.update(it.toByteArray()) }
        sha.digest().joinToString("") { "%02x".format(it) }
    } catch (e: Exception) {
        null
    }

    fun label(context: Context, pkg: String): String = try {
        context.packageManager.getApplicationLabel(context.packageManager.getApplicationInfo(pkg, 0)).toString()
    } catch (e: Exception) {
        pkg
    }
}

/** The service other apps bind (action `org.retifs.action.ACCESS`) to get connections. */
class AccessService : Service() {
    override fun onBind(intent: Intent?): IBinder = object : Binder() {
        override fun onTransact(code: Int, data: Parcel, reply: Parcel?, flags: Int): Boolean {
            if (code != Access.OPEN) return super.onTransact(code, data, reply, flags)
            data.enforceInterface(Access.DESCRIPTOR)
            val r = reply ?: return true
            val uid = Binder.getCallingUid()
            val (status, pfd) = open(uid)
            r.writeInt(status)
            if (pfd != null) pfd.writeToParcel(r, Parcelable.PARCELABLE_WRITE_RETURN_VALUE)
            return true
        }
    }

    private fun open(uid: Int): Pair<Int, ParcelFileDescriptor?> {
        Core.ready(this)
        val pkgs = packageManager.getPackagesForUid(uid) ?: return Access.NOT_LET_IN to null
        for (pkg in pkgs) {
            val g = Access.grant(this, pkg) ?: continue
            if (g.optString("cert") != Access.cert(this, pkg)) {
                Log.w(TAG, "$pkg isn't the app that was let in (another signer): refused")
                continue
            }
            val fd = Core.appConnect(g.getString("token"))
            if (fd < 0) return Access.NOT_RUNNING to null
            return Access.OK to ParcelFileDescriptor.adoptFd(fd)
        }
        return Access.NOT_LET_IN to null
    }

    companion object {
        const val TAG = "retifs-access"
    }
}

/**
 * Asks the user to let an app in (started by that app for a result, action
 * `org.retifs.action.LET_IN`): which libraries it may use, and whether it
 * may change them. Extras: `for` (a purpose, ADR-0032: the libraries made
 * for it are ticked), `write` (whether it asks to change them), `why` (its
 * own words). Started again, it changes what the app has.
 */
class AccessActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Only an app asking for itself, for a result: then Android says who it is.
        val pkg = callingActivity?.packageName
        if (pkg == null || pkg == packageName) {
            finish()
            return
        }
        Core.ready(this)
        val purpose = intent.getStringExtra("for") ?: ""
        val asksWrite = intent.getBooleanExtra("write", false)
        val why = intent.getStringExtra("why") ?: ""
        val label = Access.label(this, pkg)
        setContent {
            MaterialTheme(colorScheme = if (isSystemInDarkTheme()) darkColorScheme() else lightColorScheme()) {
                LetIn(pkg, label, purpose, asksWrite, why)
            }
        }
    }

    @androidx.compose.runtime.Composable
    private fun LetIn(pkg: String, label: String, purpose: String, asksWrite: Boolean, why: String) {
        val scope = rememberCoroutineScope()
        val before = remember { Access.grant(this, pkg) }
        var libraries by remember { mutableStateOf<List<JSONObject>?>(null) }
        var error by remember { mutableStateOf("") }
        val chosen = remember { mutableStateListOf<String>() }
        var write by remember { mutableStateOf(before?.optBoolean("write") ?: asksWrite) }
        LaunchedEffect(Unit) {
            val s = JSONObject(core { Core.status() })
            if (!s.optBoolean("up")) {
                error = "retifs isn't running yet: " + s.optString("error")
                return@LaunchedEffect
            }
            val list = s.optJSONArray("libraries") ?: JSONArray()
            val all = (0 until list.length()).map { list.getJSONObject(it) }
            libraries = all
            val had = before?.optJSONArray("libraries")?.let { a -> (0 until a.length()).map { a.getString(it) } }
            chosen.addAll(had ?: all.filter { l ->
                val f = l.optJSONArray("for")
                // Made for what the app does, else every generic library (nothing chosen, ADR-0032).
                if (f == null || f.length() == 0) true else (0 until f.length()).any { f.getString(it) == purpose }
            }.map { it.getString("name") })
        }
        AlertDialog(
            onDismissRequest = { setResult(RESULT_CANCELED); finish() },
            title = { Text("Let $label use retifs?") },
            text = {
                Column(Modifier.heightIn(max = 480.dp).verticalScroll(rememberScrollState())) {
                    if (why.isNotEmpty()) Text(why, style = MaterialTheme.typography.bodySmall)
                    Text("It sees only the libraries ticked here.", style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(top = 4.dp, bottom = 8.dp))
                    if (error.isNotEmpty()) Text(error, color = MaterialTheme.colorScheme.error)
                    libraries?.forEach { l ->
                        val name = l.getString("name")
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Checkbox(checked = name in chosen, onCheckedChange = { if (it) chosen.add(name) else chosen.remove(name) })
                            Column {
                                Text(name)
                                Text(l.optString("access"), style = MaterialTheme.typography.bodySmall)
                            }
                        }
                    }
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(top = 8.dp)) {
                        Switch(checked = write, onCheckedChange = { write = it })
                        Text("May change them" + if (asksWrite) " (it asks to)" else "", Modifier.padding(start = 8.dp))
                    }
                }
            },
            confirmButton = {
                Button(enabled = chosen.isNotEmpty() && libraries != null, onClick = {
                    scope.launch {
                        val names = libraries!!.map { it.getString("name") }.filter { it in chosen }
                        val token = core { Core.appAdd(Access.appName(pkg), JSONArray(names).toString(), write) }
                        if (token.startsWith("error: ") || token.isEmpty()) {
                            error = token.removePrefix("error: ")
                            return@launch
                        }
                        val cert = Access.cert(this@AccessActivity, pkg)
                        if (cert == null) {
                            error = "can't tell who $label is"
                            return@launch
                        }
                        Access.save(this@AccessActivity, pkg, JSONObject().put("token", token).put("cert", cert).put("libraries", JSONArray(names)).put("write", write))
                        setResult(RESULT_OK)
                        finish()
                    }
                }) { Text("Allow") }
            },
            dismissButton = {
                Row {
                    if (before != null) {
                        TextButton(onClick = {
                            scope.launch {
                                core { Core.appRemove(Access.appName(pkg)) }
                                Access.forget(this@AccessActivity, pkg)
                                setResult(RESULT_CANCELED)
                                finish()
                            }
                        }) { Text("Remove access") }
                    }
                    TextButton(onClick = { setResult(RESULT_CANCELED); finish() }) { Text("Deny") }
                }
            },
        )
    }
}
