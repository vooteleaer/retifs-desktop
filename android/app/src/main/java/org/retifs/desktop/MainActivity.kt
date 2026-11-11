package org.retifs.desktop

import android.Manifest
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Share
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

/**
 * retifs on Android: this device (the node, storage, libraries), Reticulum
 * (Prns's app, which retifs joins), and the settings. The same three views as the
 * desktop utility, over the same core.
 */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Core.ready(this)
        val wanted = mutableListOf(
            Manifest.permission.READ_CONTACTS, Manifest.permission.WRITE_CONTACTS,
            Manifest.permission.READ_CALENDAR, Manifest.permission.WRITE_CALENDAR,
        )
        if (Build.VERSION.SDK_INT >= 33) wanted += Manifest.permission.POST_NOTIFICATIONS
        val missing = wanted.filter { checkSelfPermission(it) != android.content.pm.PackageManager.PERMISSION_GRANTED }
        if (missing.isNotEmpty()) requestPermissions(missing.toTypedArray(), 1)
        sync()
        RetifsService.start(this)
        opened(intent)
        setContent {
            MaterialTheme(colorScheme = if (isSystemInDarkTheme()) darkColorScheme() else lightColorScheme()) {
                App()
            }
        }
    }

    override fun onNewIntent(intent: android.content.Intent) {
        super.onNewIntent(intent)
        opened(intent)
    }

    /** A retifs: link opened elsewhere: the join dialog shows it (nothing is joined until the user says so). */
    private fun opened(intent: android.content.Intent?) {
        val link = intent?.dataString ?: return
        if (link.startsWith("retifs:")) LinkOpened.link = link
    }

    private fun allowed(p: String) = checkSelfPermission(p) == android.content.pm.PackageManager.PERMISSION_GRANTED

    /** retifs contacts and calendars in the system's own: the account, and a sync now (what's allowed). */
    private fun sync() {
        val contacts = allowed(Manifest.permission.WRITE_CONTACTS)
        val calendar = allowed(Manifest.permission.WRITE_CALENDAR)
        if (!contacts && !calendar) return
        Accounts.ensure(this)
        if (contacts) Accounts.syncContacts(this)
        if (calendar) Accounts.syncCalendars()
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        sync()
    }
}

/** Calls the core off the main thread. */
suspend fun <T> core(work: () -> T): T = withContext(Dispatchers.IO) { work() }

fun human(n: Long): String {
    val units = listOf("B", "KB", "MB", "GB", "TB")
    var v = n.toDouble()
    var i = 0
    while (v >= 1024 && i < units.size - 1) { v /= 1024; i++ }
    return if (i == 0) "$n B" else String.format("%.1f %s", v, units[i])
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun App() {
    var tab by remember { mutableIntStateOf(0) }
    var status by remember { mutableStateOf(JSONObject()) }
    var reticulum by remember { mutableStateOf(false) }
    var tick by remember { mutableIntStateOf(0) }
    var note by remember { mutableStateOf("") }
    // A retifs: link opened elsewhere, wherever the app is.
    LinkOpened.link?.let { given -> JoinLinkDialog({ note = it }, given) { LinkOpened.link = null } }
    LaunchedEffect(tick) {
        while (true) {
            status = JSONObject(core { Core.status() })
            reticulum = core { Core.sharedInstanceUp() }
            // Often while it starts, so its steps and progress move.
            delay(if (status.optBoolean("up")) 5_000 else 1_000)
        }
    }
    val refresh = { tick++ }
    Scaffold(
        topBar = { TopAppBar(title = { Text(listOf("retifs", "Announces", "Settings")[tab]) }) },
        bottomBar = {
            NavigationBar {
                NavigationBarItem(selected = tab == 0, onClick = { tab = 0 }, icon = { Icon(Icons.Filled.Home, null) }, label = { Text("This device") })
                NavigationBarItem(selected = tab == 1, onClick = { tab = 1 }, icon = { Icon(Icons.Filled.Share, null) }, label = { Text("Announces") })
                NavigationBarItem(selected = tab == 2, onClick = { tab = 2 }, icon = { Icon(Icons.Filled.Settings, null) }, label = { Text("Settings") })
            }
        },
    ) { inner ->
        Column(Modifier.padding(inner).fillMaxSize()) {
            if (note.isNotEmpty()) {
                Text(note, Modifier.padding(horizontal = 16.dp, vertical = 4.dp), style = MaterialTheme.typography.bodySmall)
            }
            when (tab) {
                0 -> Overview(status, reticulum) { note = it; refresh() }
                1 -> Announces(status) { note = it; refresh() }
                else -> SettingsView { note = it; refresh() }
            }
        }
    }
}

@Composable
fun Overview(s: JSONObject, reticulum: Boolean, said: (String) -> Unit) {
    val context = LocalContext.current
    var newLibrary by remember { mutableStateOf(false) }
    var joinLink by remember { mutableStateOf(false) }
    if (newLibrary) NewLibraryDialog(said) { newLibrary = false }
    if (joinLink) JoinLinkDialog(said) { joinLink = false }
    LazyColumn(Modifier.fillMaxSize().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        item {
            Card(Modifier.fillMaxWidth()) {
                Column(Modifier.padding(16.dp)) {
                    DaemonState(s, reticulum)
                    s.optJSONObject("usage")?.let { u ->
                        Spacer(Modifier.padding(4.dp))
                        Text("Own data: ${human(u.optLong("pinned"))} of ${human(u.optLong("store_limit"))}")
                        Text("Held for others: ${human(u.optLong("leased"))}")
                        Text("Cache: ${human(u.optLong("cache"))} of ${human(u.optLong("cache_max"))}")
                        Text("Free on the phone: ${human(u.optLong("disk_available"))}")
                    }
                    Reticulum(reticulum)
                    Row(Modifier.padding(top = 12.dp)) {
                        Button(onClick = { RetifsService.start(context); said("Starting retifs") }) { Text("Start") }
                        Spacer(Modifier.width(8.dp))
                        OutlinedButton(onClick = { RetifsService.restartDaemon(context); said("Restarting retifs") }) { Text("Restart") }
                        Spacer(Modifier.width(8.dp))
                        OutlinedButton(onClick = { RetifsService.stop(context); said("retifs stopped") }) { Text("Stop") }
                    }
                }
            }
        }
        val libraries = s.optJSONArray("libraries") ?: JSONArray()
        item {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Libraries", style = MaterialTheme.typography.titleSmall, modifier = Modifier.weight(1f))
                TextButton(onClick = { joinLink = true }) { Text("Join with link") }
                IconButton(onClick = { newLibrary = true }) { Icon(Icons.Filled.Add, "New library") }
            }
        }
        if (libraries.length() == 0) {
            item { Text("None yet.", style = MaterialTheme.typography.bodySmall) }
        }
        items((0 until libraries.length()).map { libraries.getJSONObject(it) }) { l -> LibraryCard(l, said) }
    }
}

/** The public libraries announced on the network, to join. */
@Composable
fun Announces(s: JSONObject, said: (String) -> Unit) {
    val scope = rememberCoroutineScope()
    var heard by remember { mutableStateOf(JSONArray()) }
    var heardError by remember { mutableStateOf("") }
    LaunchedEffect(s) {
        val answer = core { Core.heard() }
        if (answer.startsWith("[")) {
            heard = JSONArray(answer)
            heardError = ""
        } else {
            heardError = JSONObject(answer).optString("error", answer)
        }
    }
    LazyColumn(Modifier.fillMaxSize().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        if (heardError.isNotEmpty()) {
            item { Text(heardError, style = MaterialTheme.typography.bodySmall) }
        } else if (heard.length() == 0) {
            item { Text("No public library heard yet. Announces come every few minutes once connected.", style = MaterialTheme.typography.bodySmall) }
        }
        items((0 until heard.length()).map { heard.getJSONObject(it) }) { h ->
            Card(Modifier.fillMaxWidth()) {
                Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text(h.optString("name"), style = MaterialTheme.typography.titleMedium)
                        if (h.optString("about").isNotEmpty()) Text(h.optString("about"), style = MaterialTheme.typography.bodySmall)
                        Text("${h.optInt("hops")} hop(s) away", style = MaterialTheme.typography.bodySmall)
                    }
                    when {
                        !h.isNull("here") -> Text("Joined", style = MaterialTheme.typography.labelLarge)
                        !h.optBoolean("readable", true) -> Text("Needs a newer retifs", style = MaterialTheme.typography.labelSmall)
                        else -> Button(onClick = {
                            scope.launch { said(core { Core.join(h.optString("address")) }.ifEmpty { "Joined ${h.optString("name")}" }) }
                        }) { Text("Join") }
                    }
                }
            }
        }
    }
}

/**
 * Reticulum on the phone is Prns's app (or another one with a shared
 * instance): retifs only joins it. Its connections are set up there.
 */
/**
 * What retifs is doing: running, starting (the step retifsd is at, with a
 * progress bar), waiting for Reticulum, stopped by itself (why), or not
 * started. Never a bare "not running" while it starts.
 */
@Composable
fun DaemonState(s: JSONObject, reticulum: Boolean) {
    val starting = s.optJSONObject("starting")
    val title = MaterialTheme.typography.titleMedium
    val small = MaterialTheme.typography.bodySmall
    when {
        s.optBoolean("up") -> {
            Text("retifs is running", style = title)
            Text("Node ${s.optString("node")}", style = small)
        }
        starting != null || RetifsService.daemonAlive -> {
            Text("retifs is starting…", style = title)
            val progress = starting?.takeIf { !it.isNull("progress") }?.optDouble("progress")
            Spacer(Modifier.padding(4.dp))
            if (progress != null) {
                LinearProgressIndicator(progress = { progress.toFloat() }, modifier = Modifier.fillMaxWidth())
            } else {
                LinearProgressIndicator(Modifier.fillMaxWidth())
            }
            Spacer(Modifier.padding(4.dp))
            val step = starting?.optString("text")?.takeIf { it.isNotEmpty() } ?: "starting the daemon"
            val since = starting?.optLong("since_ms") ?: 0L
            val took = if (since > 0) (System.currentTimeMillis() - since) / 1000 else 0L
            Text(step.replaceFirstChar { it.uppercase() } + if (took >= 10) " (${took / 60} min ${took % 60} s)" else "", style = small)
            if (starting?.optString("step")?.startsWith("checking") == true) {
                Text("retifs wasn't stopped cleanly last time, so it checks its store first. On a large store this takes a few minutes.", style = small)
            }
        }
        RetifsService.waitingForReticulum || !reticulum -> {
            Text("Waiting for Reticulum", style = title)
            Text("retifs starts as soon as Reticulum is running.", style = small)
        }
        RetifsService.lastExit != null -> {
            Text("retifs stopped", style = title, color = MaterialTheme.colorScheme.error)
            Text(RetifsService.lastExit ?: "", style = small)
            Text("It's started again in a few seconds; Restart tries at once.", style = small)
        }
        else -> {
            Text("retifs isn't running", style = title)
            Text("Start starts it.", style = small)
        }
    }
}

@Composable
fun Reticulum(up: Boolean) {
    val context = LocalContext.current
    val prns = context.packageManager.getLaunchIntentForPackage(RetifsService.PRNS_APP)
    Row(Modifier.padding(top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(
            when {
                up -> "Reticulum is running"
                prns != null -> "Reticulum isn't running: start it in Prns"
                else -> "Reticulum isn't running: start a Reticulum app with a shared instance (Prns or another)"
            },
            Modifier.weight(1f),
            style = MaterialTheme.typography.bodyMedium,
        )
        if (prns != null) {
            TextButton(onClick = { context.startActivity(prns) }) { Text("Open Prns") }
        }
    }
}

@Composable
fun SettingsView(said: (String) -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var list by remember { mutableStateOf(JSONArray()) }
    val values = remember { mutableStateMapOf<String, String>() }
    LaunchedEffect(Unit) {
        list = JSONArray(core { Core.settings() })
        for (i in 0 until list.length()) {
            val s = list.getJSONObject(i)
            values["${s.getString("section")}.${s.getString("key")}"] = if (s.isNull("value")) "" else s.getString("value")
        }
    }
    LazyColumn(Modifier.fillMaxSize().padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        item { RecoveryCard(said) }
        item { Text("This node's retifs.toml, filled with the defaults this device gets until you change them.", style = MaterialTheme.typography.bodySmall) }
        items((0 until list.length()).map { list.getJSONObject(it) }) { s ->
            val id = "${s.getString("section")}.${s.getString("key")}"
            if (s.optBoolean("switch")) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(s.getString("what"), Modifier.weight(1f))
                    Switch(checked = values[id] == "true", onCheckedChange = { values[id] = if (it) "true" else "" })
                }
            } else {
                val unit = s.optString("unit", "")
                OutlinedTextField(values[id] ?: "", { values[id] = it }, label = { Text(s.getString("what")) }, singleLine = true, modifier = Modifier.fillMaxWidth(),
                    suffix = if (unit.isNotEmpty()) ({ Text(unit) }) else null)
            }
        }
        item {
            Button(onClick = {
                val json = JSONObject()
                for ((k, v) in values) json.put(k, v)
                scope.launch {
                    val e = core { Core.saveSettings(json.toString()) }
                    if (e.isEmpty()) RetifsService.restartDaemon(context)
                    said(e.ifEmpty { "Settings saved; retifs restarted" })
                }
            }) { Text("Save and restart retifs") }
        }
    }
}
