// retifs-desktop UI (milestone 1): overview and namespaces, from the Rust
// side's `status` and `namespaces` commands and its `status` events.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);

function human(n) {
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let v = n, i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return `${v.toFixed(1)} ${units[i]}`;
}

function bar(label, used, of, note) {
  const pct = of > 0 ? Math.min(100, (used / of) * 100) : 0;
  const row = document.createElement("div");
  row.className = "bar";
  row.innerHTML = `<div class="bar-label"></div><div class="bar-track"><div class="bar-fill"></div></div><div class="bar-value"></div>`;
  row.querySelector(".bar-label").textContent = label;
  row.querySelector(".bar-fill").style.width = `${pct}%`;
  row.querySelector(".bar-value").textContent = note ?? `${human(used)} of ${human(of)}`;
  return row;
}

function showStatus(s) {
  const state = $("state");
  state.textContent = s.up ? "running" : "daemon not running";
  state.className = `pill ${s.up ? "ok" : "bad"}`;
  $("node").textContent = s.up ? `This node: ${s.node} · ${s.chunks} chunks · store ${s.store}` : `Store: ${s.store}`;
  $("error").textContent = s.up ? "" : (s.error ?? "");
  const usage = $("usage");
  usage.replaceChildren();
  const u = s.usage;
  if (!u) return;
  usage.append(
    bar("Own data", u.pinned, u.store_limit),
    bar("Held for others", u.leased, u.store_limit),
    bar("Cache", u.cache, u.cache_max),
    bar("Volume", u.disk_total - u.disk_available, u.disk_total, `${human(u.disk_available)} free of ${human(u.disk_total)} (reserve ${human(u.reserve)})`),
  );
}

async function loadNamespaces() {
  const rows = $("ns-rows");
  rows.replaceChildren();
  let list = [];
  try { list = await invoke("namespaces"); } catch (e) { $("ns-empty").textContent = String(e); }
  $("ns-empty").hidden = list.length > 0;
  for (const ns of list) {
    const tr = document.createElement("tr");
    const cells = [
      ns.name,
      String(ns.records),
      ns.writers.map(([w, rights]) => `${w.slice(0, 8)}… ${rights.join(",")}`).join("\n"),
      String(ns.peers),
      ns.lazy ? `${ns.keep} (lazy)` : ns.keep,
      ns.checkpoint ? "yes" : "—",
    ];
    for (const c of cells) {
      const td = document.createElement("td");
      td.textContent = c;
      tr.append(td);
    }
    rows.append(tr);
  }
}

for (const b of document.querySelectorAll("nav button")) {
  b.addEventListener("click", () => {
    for (const x of document.querySelectorAll("nav button")) x.classList.toggle("active", x === b);
    for (const s of document.querySelectorAll("main section")) s.hidden = s.id !== b.dataset.view;
    if (b.dataset.view === "namespaces") loadNamespaces();
  });
}

listen("status", (e) => showStatus(e.payload));
invoke("status").then(showStatus);
