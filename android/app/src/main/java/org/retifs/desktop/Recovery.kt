package org.retifs.desktop

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.DialogProperties
import kotlinx.coroutines.launch
import org.json.JSONObject

/** What a recovery kit is, said before one is made (as the desktop says it). */
private const val ABOUT = "A recovery kit is 24 words that bring all your libraries back on a new device if you lose every device you have. You write them down on paper once; they are shown only now and kept nowhere. Whoever has them can open every library of yours, so keep them as you'd keep a key to your home."

/** The recovery kit (retifs ADR-0033): make one, or recover from one on a new device. */
@Composable
fun RecoveryCard(said: (String) -> Unit) {
    var asking by remember { mutableStateOf(false) }
    var kit by remember { mutableStateOf<JSONObject?>(null) }
    var recovering by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    if (asking) {
        AlertDialog(
            onDismissRequest = { asking = false },
            title = { Text("Make a recovery kit?") },
            text = { Text(ABOUT) },
            confirmButton = {
                Button(onClick = {
                    asking = false
                    said("Making your recovery kit…")
                    scope.launch {
                        val k = JSONObject(core { Core.recoveryCreate() })
                        if (k.has("error")) said(k.getString("error")) else { said(""); kit = k }
                    }
                }) { Text("Make kit") }
            },
            dismissButton = { TextButton(onClick = { asking = false }) { Text("Cancel") } },
        )
    }
    kit?.let { KitWordsDialog(it) { kit = null } }
    if (recovering) RecoverDialog(said) { recovering = false }
    Card(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(16.dp)) {
            Text("Recovery kit", style = MaterialTheme.typography.titleSmall)
            Text("If every device of yours is lost, 24 words bring your libraries back on a new one.", style = MaterialTheme.typography.bodySmall)
            Row(Modifier.padding(top = 12.dp)) {
                Button(onClick = { asking = true }) { Text("Make a kit") }
                Spacer(Modifier.width(8.dp))
                OutlinedButton(onClick = { recovering = true }) { Text("Recover") }
            }
        }
    }
}

/** The words, numbered, until the user says they wrote them down: they aren't shown again. */
@Composable
private fun KitWordsDialog(kit: JSONObject, close: () -> Unit) {
    val words = kit.getJSONArray("words")
    val covered = kit.optJSONArray("covered")
    val notCovered = kit.optJSONArray("not_covered")
    var written by remember { mutableStateOf(false) }
    AlertDialog(
        onDismissRequest = {},
        properties = DialogProperties(dismissOnBackPress = false, dismissOnClickOutside = false),
        title = { Text("Your recovery kit") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Write these words down on paper, in order, and keep the paper somewhere safe. They won't be shown again.", style = MaterialTheme.typography.bodySmall)
                // Two columns of twelve: 1–12, then 13–24.
                Row {
                    for (col in 0 until 2) {
                        Column(Modifier.weight(1f)) {
                            for (row in 0 until 12) {
                                val i = col * 12 + row
                                Text("${(i + 1).toString().padStart(2)}. ${words.getString(i)}", fontFamily = FontFamily.Monospace)
                            }
                        }
                    }
                }
                val covers = if (notCovered == null || notCovered.length() == 0) {
                    "It covers all ${covered?.length() ?: 0} of your libraries, and those you make or join later."
                } else {
                    "Not covered (no device of yours may invite there): " + (0 until notCovered.length()).joinToString { notCovered.getString(it) } + "."
                }
                Text(covers, style = MaterialTheme.typography.bodySmall)
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Checkbox(checked = written, onCheckedChange = { written = it })
                    Text("I have written the 24 words down, in order")
                }
            }
        },
        confirmButton = { Button(enabled = written, onClick = close) { Text("Done") } },
    )
}

/** The words typed and checked as they come; then minutes of recovering, said as it goes. */
@Composable
private fun RecoverDialog(said: (String) -> Unit, close: () -> Unit) {
    val scope = rememberCoroutineScope()
    var text by remember { mutableStateOf("") }
    var check by remember { mutableStateOf(JSONObject().put("ok", false).put("say", "0 of 24 words")) }
    var busy by remember { mutableStateOf(false) }
    LaunchedEffect(text) { check = JSONObject(core { Core.recoveryCheck(text) }) }
    AlertDialog(
        onDismissRequest = { if (!busy) close() },
        properties = DialogProperties(dismissOnClickOutside = false),
        title = { Text(if (busy) "Recovering…" else "Recover from a kit") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                if (busy) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        CircularProgressIndicator(Modifier.padding(end = 12.dp))
                        Text("Looking for a device of yours that keeps your keyring, then bringing each library back. This can take several minutes.")
                    }
                } else {
                    Text("Type your 24 words, in order. A device of yours that keeps your keyring must be running somewhere on the network.", style = MaterialTheme.typography.bodySmall)
                    OutlinedTextField(value = text, onValueChange = { text = it }, minLines = 4, modifier = Modifier.fillMaxWidth(), label = { Text("Recovery words") })
                    Text(check.optString("say"), style = MaterialTheme.typography.bodySmall)
                }
            }
        },
        confirmButton = {
            if (!busy) {
                Button(enabled = check.optBoolean("ok"), onClick = {
                    busy = true
                    scope.launch {
                        val r = JSONObject(core { Core.recoveryRestore(text) })
                        busy = false
                        close()
                        if (r.has("error")) {
                            said("Not recovered: " + r.getString("error"))
                        } else {
                            val libs = r.getJSONArray("libraries")
                            said(
                                if (libs.length() == 0) "Nothing to bring back: the keyring lists no library this device doesn't have."
                                else "Back on this device: " + (0 until libs.length()).joinToString { libs.getString(it) } + ". Content arrives in the background; remove the devices you lost in each library."
                            )
                        }
                    }
                }) { Text("Recover") }
            }
        },
        dismissButton = { if (!busy) TextButton(onClick = close) { Text("Cancel") } },
    )
}
