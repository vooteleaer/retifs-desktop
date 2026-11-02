package org.retifs.desktop

import android.content.Intent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.Checkbox
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import org.json.JSONArray
import org.json.JSONObject

/*
 * A library as in the Haiku, GTK and Windows utilities: whether it's
 * public, what this user may do, what's here of it (a bar while it comes
 * in), and a menu: favourite, keep on this device, copy link, members,
 * invite, announce, leave. Members: who reads, who writes, who may invite,
 * invitations not used yet; removing. Invite: a role and how long, then the
 * link to pass on (Android's share sheet).
 */

private fun grouped(n: Long): String = String.format("%,d", n)

/** What a library is and has, for its card. */
@Composable
fun LibraryCard(l: JSONObject, said: (String) -> Unit) {
    val scope = rememberCoroutineScope()
    val clipboard = LocalClipboardManager.current
    val name = l.optString("name")
    val me = name == "me"
    val own = l.optBoolean("own")
    val mayInvite = l.optString("access").contains("invite")
    val kept = !l.optBoolean("lazy") && l.optString("keep") == "all"
    var menu by remember { mutableStateOf(false) }
    var members by remember { mutableStateOf(false) }
    var inviting by remember { mutableStateOf(false) }
    var leaving by remember { mutableStateOf(false) }
    Card(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(start = 16.dp, top = 8.dp, bottom = 12.dp, end = 4.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text((if (l.optBoolean("favorite")) "★ " else "") + name, style = MaterialTheme.typography.titleMedium)
                    val who = listOf(
                        if (l.optBoolean("public")) "public" else "private",
                        l.optString("access"),
                    ) + (if (own) listOf("yours") else emptyList())
                    Text(who.joinToString(" · "), style = MaterialTheme.typography.bodySmall)
                    val here = when {
                        l.optBoolean("lazy") -> "fetched as needed"
                        kept -> "kept on this device"
                        else -> "content fetched as needed"
                    }
                    Text("${grouped(l.optLong("records"))} record(s) · ${l.optInt("peers")} peer(s) · $here", style = MaterialTheme.typography.bodySmall)
                }
                IconButton(onClick = { menu = true }) { Icon(Icons.Filled.MoreVert, "What can be done with it") }
                DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                    val run = { what: String, call: () -> String ->
                        menu = false
                        scope.launch { said(core(call).ifEmpty { what }) }
                        Unit
                    }
                    if (!me) DropdownMenuItem(
                        text = { Text(if (l.optBoolean("favorite")) "Remove from favorites" else "Add to favorites") },
                        onClick = { run("Done") { Core.favorite(name, !l.optBoolean("favorite")) } },
                    )
                    DropdownMenuItem(
                        text = { Text(if (kept) "Fetch as needed only" else "Keep on this device") },
                        onClick = { run(if (kept) "Only what's used is kept" else "Keeping it on this device") { Core.keep(name, !kept) } },
                    )
                    if (!me) {
                        DropdownMenuItem(text = { Text("Members…") }, onClick = { menu = false; members = true })
                        DropdownMenuItem(text = { Text("Invite…") }, enabled = mayInvite, onClick = { menu = false; inviting = true })
                        DropdownMenuItem(
                            text = { Text(if (l.optBoolean("public")) "Copy link" else "Copy link (whoever has it can read)") },
                            onClick = {
                                menu = false
                                scope.launch {
                                    val link = core { Core.link(name) }
                                    if (link.startsWith("error")) said(link) else {
                                        clipboard.setText(AnnotatedString(link))
                                        said("The link is copied: whoever has it can read $name")
                                    }
                                }
                            },
                        )
                    }
                    if (own && !me) DropdownMenuItem(
                        text = { Text(if (l.optBoolean("announced")) "Stop announcing" else "Announce on the network") },
                        onClick = {
                            run(if (l.optBoolean("announced")) "Not announced any more" else "Announced: anyone may join and read it") {
                                Core.announce(name, !l.optBoolean("announced"))
                            }
                        },
                    )
                    if (!me && !own) DropdownMenuItem(text = { Text("Leave library…") }, onClick = { menu = false; leaving = true })
                }
            }
            if (l.optBoolean("coming")) {
                val all = l.optLong("kept")
                val got = l.optLong("kept_here")
                if (all > 0) {
                    Text("Fetching ${grouped(got)} of ${grouped(all)} files…", style = MaterialTheme.typography.bodySmall)
                    LinearProgressIndicator(progress = { got.toFloat() / all }, modifier = Modifier.fillMaxWidth().padding(end = 12.dp, top = 4.dp))
                } else {
                    Text("Fetching its records…", style = MaterialTheme.typography.bodySmall)
                    LinearProgressIndicator(Modifier.fillMaxWidth().padding(end = 12.dp, top = 4.dp))
                }
            }
        }
    }
    if (members) MembersDialog(name, mayInvite, onInvite = { members = false; inviting = true }, said = said) { members = false }
    if (inviting) InviteDialog(name, said) { inviting = false }
    if (leaving) AlertDialog(
        onDismissRequest = { leaving = false },
        title = { Text("Leave $name?") },
        text = { Text("Its records go from this phone. Others who have it keep theirs; you can join it again.") },
        confirmButton = {
            TextButton(onClick = {
                leaving = false
                scope.launch { said(core { Core.leave(name) }.ifEmpty { "Left $name" }) }
            }) { Text("Leave") }
        },
        dismissButton = { TextButton(onClick = { leaving = false }) { Text("Cancel") } },
    )
}

/** A library's members and invitations not used yet. */
@Composable
fun MembersDialog(ns: String, manage: Boolean, onInvite: () -> Unit, said: (String) -> Unit, close: () -> Unit) {
    val scope = rememberCoroutineScope()
    var tick by remember { mutableIntStateOf(0) }
    var members by remember { mutableStateOf(JSONArray()) }
    var invites by remember { mutableStateOf(JSONArray()) }
    var error by remember { mutableStateOf("") }
    var removing by remember { mutableStateOf<JSONObject?>(null) }
    LaunchedEffect(tick) {
        val m = core { Core.members(ns) }
        if (m.startsWith("[")) { members = JSONArray(m); error = "" } else error = JSONObject(m).optString("error", m)
        val i = core { Core.invites(ns) }
        if (i.startsWith("[")) invites = JSONArray(i)
    }
    AlertDialog(
        onDismissRequest = close,
        title = { Text("Members of $ns") },
        text = {
            LazyColumn(Modifier.heightIn(max = 420.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                if (error.isNotEmpty()) item { Text(error) }
                items((0 until members.length()).map { members.getJSONObject(it) }) { m ->
                    val words = m.optString("words")
                    val who = m.optString("name").ifEmpty { words }
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Column(Modifier.weight(1f)) {
                            Text(who, style = MaterialTheme.typography.titleSmall)
                            val details = mutableListOf(m.optString("role"))
                            if (m.optBoolean("creator")) details += "made it"
                            m.optJSONArray("limits")?.let { a -> for (i in 0 until a.length()) details += a.optString(i) }
                            if (who != words) details += words
                            Text(details.joinToString(" · "), style = MaterialTheme.typography.bodySmall)
                        }
                        if (manage && !m.optBoolean("you") && !m.optBoolean("creator")) {
                            TextButton(onClick = { removing = m }) { Text("Remove") }
                        }
                    }
                }
                val pending = (0 until invites.length()).map { invites.getJSONObject(it) }.filter { !it.optBoolean("spent") }
                if (pending.isNotEmpty()) {
                    item { HorizontalDivider(); Text("Invitations not used yet", style = MaterialTheme.typography.titleSmall, modifier = Modifier.padding(top = 8.dp)) }
                }
                items(pending) { inv ->
                    val left = (inv.optLong("expires") - System.currentTimeMillis()) / 3_600_000
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Column(Modifier.weight(1f)) {
                            Text("Invitation ${inv.optString("id").take(8)}", style = MaterialTheme.typography.titleSmall)
                            Text("${inv.optString("role")} · ${if (left < 0) "expired" else "$left h left"}", style = MaterialTheme.typography.bodySmall)
                        }
                        if (manage) TextButton(onClick = {
                            scope.launch { said(core { Core.cancelInvite(ns, inv.optString("id")) }.ifEmpty { "Withdrawn" }); tick++ }
                        }) { Text("Withdraw") }
                    }
                }
                item { Text("Removing someone: they keep what they've seen, not what's written from then on.", style = MaterialTheme.typography.bodySmall) }
            }
        },
        confirmButton = { if (manage) Button(onClick = onInvite) { Text("Invite…") } },
        dismissButton = { TextButton(onClick = close) { Text("Close") } },
    )
    removing?.let { m ->
        val who = m.optString("name").ifEmpty { m.optString("words") }
        AlertDialog(
            onDismissRequest = { removing = null },
            title = { Text("Remove $who from $ns?") },
            text = { Text("What they write stops counting, and the library's key changes: they keep what they've read, but can't read what's written from now on.") },
            confirmButton = {
                TextButton(onClick = {
                    removing = null
                    scope.launch { said(core { Core.removeMember(ns, m.optString("writer")) }.ifEmpty { "Removed $who" }); tick++ }
                }) { Text("Remove") }
            },
            dismissButton = { TextButton(onClick = { removing = null }) { Text("Cancel") } },
        )
    }
}

/** An invitation: who it makes the person, for how long; then its link. */
@Composable
fun InviteDialog(ns: String, said: (String) -> Unit, close: () -> Unit) {
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    val clipboard = LocalClipboardManager.current
    var role by remember { mutableStateOf("read") }
    var days by remember { mutableIntStateOf(7) }
    var link by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = close,
        title = { Text("Invite someone to $ns") },
        text = {
            Column {
                if (link.isEmpty()) {
                    Text("They may:")
                    for ((value, label) in listOf("read" to "Read", "write" to "Read and write", "invite" to "Read, write and invite others")) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            RadioButton(selected = role == value, onClick = { role = value })
                            Text(label)
                        }
                    }
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        for ((d, label) in listOf(1 to "1 day", 7 to "1 week", 30 to "30 days")) {
                            FilterChip(selected = days == d, onClick = { days = d }, label = { Text(label) })
                        }
                    }
                } else {
                    SelectionContainer { Text(link, style = MaterialTheme.typography.bodySmall) }
                    Text("Pass it in person or end to end: whoever sees it can read; the first to open it becomes the member.", style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(top = 8.dp))
                }
            }
        },
        confirmButton = {
            if (link.isEmpty()) {
                Button(onClick = {
                    scope.launch {
                        val made = core { Core.invite(ns, role, days) }
                        if (made.startsWith("error")) said(made) else link = made
                    }
                }) { Text("Make invitation") }
            } else {
                Button(onClick = {
                    val send = Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, link)
                    context.startActivity(Intent.createChooser(send, "Send the invitation"))
                }) { Text("Share…") }
            }
        },
        dismissButton = {
            Row {
                if (link.isNotEmpty()) TextButton(onClick = { clipboard.setText(AnnotatedString(link)); said("The invitation is copied") }) { Text("Copy") }
                TextButton(onClick = close) { Text("Close") }
            }
        },
    )
}

/** A new library: its name, and whether to announce it as public. */
@Composable
fun NewLibraryDialog(said: (String) -> Unit, close: () -> Unit) {
    val scope = rememberCoroutineScope()
    var name by remember { mutableStateOf("") }
    var public by remember { mutableStateOf(false) }
    AlertDialog(
        onDismissRequest = close,
        title = { Text("New library") },
        text = {
            Column {
                OutlinedTextField(value = name, onValueChange = { name = it }, label = { Text("Name") }, singleLine = true)
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Checkbox(checked = public, onCheckedChange = { public = it })
                    Text("Announce it on the network (public: anyone may read it)")
                }
                Text("A private one is shared from its menu: members and invitations.", style = MaterialTheme.typography.bodySmall)
            }
        },
        confirmButton = {
            Button(enabled = name.isNotBlank(), onClick = {
                val n = name.trim()
                close()
                scope.launch { said(core { Core.create(n, public) }.ifEmpty { "Made $n" }) }
            }) { Text("Create") }
        },
        dismissButton = { TextButton(onClick = close) { Text("Cancel") } },
    )
}
