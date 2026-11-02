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
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Share
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
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
 * and its connections, and the settings. The same three views as the
 * desktop utility, over the same core.
 */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Core.ready(this)
        if (Build.VERSION.SDK_INT >= 33) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
        }
        RetifsService.start(this)
        setContent {
            MaterialTheme(colorScheme = if (isSystemInDarkTheme()) darkColorScheme() else lightColorScheme()) {
                App()
            }
        }
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
    var reticulum by remember { mutableStateOf(JSONObject()) }
    var tick by remember { mutableIntStateOf(0) }
    var note by remember { mutableStateOf("") }
    LaunchedEffect(tick) {
        while (true) {
            status = JSONObject(core { Core.status() })
            reticulum = JSONObject(core { Core.reticulum() })
            delay(5_000)
        }
    }
    val refresh = { tick++ }
    Scaffold(
        topBar = { TopAppBar(title = { Text(listOf("retifs", "Network", "Settings")[tab]) }) },
        bottomBar = {
            NavigationBar {
                NavigationBarItem(selected = tab == 0, onClick = { tab = 0 }, icon = { Icon(Icons.Filled.Home, null) }, label = { Text("This device") })
                NavigationBarItem(selected = tab == 1, onClick = { tab = 1 }, icon = { Icon(Icons.Filled.Share, null) }, label = { Text("Network") })
                NavigationBarItem(selected = tab == 2, onClick = { tab = 2 }, icon = { Icon(Icons.Filled.Settings, null) }, label = { Text("Settings") })
            }
        },
    ) { inner ->
        Column(Modifier.padding(inner).fillMaxSize()) {
            if (note.isNotEmpty()) {
                Text(note, Modifier.padding(horizontal = 16.dp, vertical = 4.dp), style = MaterialTheme.typography.bodySmall)
            }
            when (tab) {
                0 -> Overview(status) { note = it; refresh() }
                1 -> Network(reticulum) { note = it; refresh() }
                else -> SettingsView { note = it; refresh() }
            }
        }
    }
}

@Composable
fun Overview(s: JSONObject, said: (String) -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var heard by remember { mutableStateOf(JSONArray()) }
    var heardError by remember { mutableStateOf("") }
    var newLibrary by remember { mutableStateOf(false) }
    if (newLibrary) NewLibraryDialog(said) { newLibrary = false }
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
        item {
            Card(Modifier.fillMaxWidth()) {
                Column(Modifier.padding(16.dp)) {
                    if (s.optBoolean("up")) {
                        Text("retifs is running", style = MaterialTheme.typography.titleMedium)
                        Text("Node ${s.optString("node")}", style = MaterialTheme.typography.bodySmall)
                    } else {
                        Text("retifs isn't running yet", style = MaterialTheme.typography.titleMedium)
                        Text(s.optString("error"), style = MaterialTheme.typography.bodySmall)
                    }
                    s.optJSONObject("usage")?.let { u ->
                        Spacer(Modifier.padding(4.dp))
                        Text("Own data: ${human(u.optLong("pinned"))} of ${human(u.optLong("store_limit"))}")
                        Text("Held for others: ${human(u.optLong("leased"))}")
                        Text("Cache: ${human(u.optLong("cache"))} of ${human(u.optLong("cache_max"))}")
                        Text("Free on the phone: ${human(u.optLong("disk_available"))}")
                    }
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
                IconButton(onClick = { newLibrary = true }) { Icon(Icons.Filled.Add, "New library") }
            }
        }
        if (libraries.length() == 0) {
            item { Text("None yet.", style = MaterialTheme.typography.bodySmall) }
        }
        items((0 until libraries.length()).map { libraries.getJSONObject(it) }) { l -> LibraryCard(l, said) }
        item { Text("Heard on the network", style = MaterialTheme.typography.titleSmall) }
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

@Composable
fun Network(r: JSONObject, said: (String) -> Unit) {
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    // A change is written to Reticulum's settings, then Reticulum restarted to use it.
    val changed = { error: String, done: String ->
        if (error.isEmpty()) RetifsService.restartReticulum(context)
        said(error.ifEmpty { done })
    }
    var adding by remember { mutableStateOf(false) }
    val connections = r.optJSONArray("connections") ?: JSONArray()
    Scaffold(floatingActionButton = {
        FloatingActionButton(onClick = { adding = true }) { Icon(Icons.Filled.Add, "Add a connection") }
    }) { inner ->
        LazyColumn(Modifier.padding(inner).fillMaxSize().padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            item {
                Text(
                    if (r.optBoolean("running")) "Reticulum is running (transport ${r.optString("transport").take(12)}…)"
                    else "Reticulum isn't running: ${r.optString("error")}",
                    style = MaterialTheme.typography.bodyMedium,
                )
                Text("Out of the box it finds others on the same Wi-Fi; add a connection to a node you trust to reach further.", style = MaterialTheme.typography.bodySmall)
            }
            items((0 until connections.length()).map { connections.getJSONObject(it) }) { c ->
                Card(Modifier.fillMaxWidth()) {
                    Row(Modifier.padding(12.dp), verticalAlignment = Alignment.CenterVertically) {
                        Column(Modifier.weight(1f)) {
                            Text(c.optString("name"), style = MaterialTheme.typography.titleMedium)
                            val state = if (c.optBoolean("up")) "connected · ↓ ${human(c.optLong("rx"))} ↑ ${human(c.optLong("tx"))}" else if (c.optBoolean("enabled")) "not connected" else "off"
                            Text("${c.optString("kind")} · $state", style = MaterialTheme.typography.bodySmall)
                        }
                        Switch(checked = c.optBoolean("enabled"), onCheckedChange = { on ->
                            scope.launch { changed(core { Core.setInterface(c.optString("name"), on) }, "Connection switched ${if (on) "on" else "off"}") }
                        })
                        IconButton(onClick = {
                            scope.launch { changed(core { Core.removeInterface(c.optString("name")) }, "Connection removed") }
                        }) { Icon(Icons.Filled.Delete, "Remove") }
                    }
                }
            }
        }
    }
    if (adding) {
        var name by remember { mutableStateOf("") }
        var host by remember { mutableStateOf("") }
        var port by remember { mutableStateOf("4242") }
        AlertDialog(
            onDismissRequest = { adding = false },
            title = { Text("Add a connection") },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text("A TCP connection to another Reticulum node: someone's you trust, or a public one. Everything retifs sends is encrypted end to end either way.", style = MaterialTheme.typography.bodySmall)
                    OutlinedTextField(name, { name = it }, label = { Text("Name") }, singleLine = true)
                    OutlinedTextField(host, { host = it }, label = { Text("Host") }, singleLine = true)
                    OutlinedTextField(port, { port = it }, label = { Text("Port") }, singleLine = true)
                }
            },
            confirmButton = {
                TextButton(onClick = {
                    adding = false
                    val p = port.trim().toIntOrNull() ?: 4242
                    val n = name.trim().ifEmpty { host.trim() }
                    scope.launch { changed(core { Core.addTcp(n, host.trim(), p) }, "Connection added; Reticulum restarted") }
                }, enabled = host.isNotBlank()) { Text("Add") }
            },
            dismissButton = { TextButton(onClick = { adding = false }) { Text("Cancel") } },
        )
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
