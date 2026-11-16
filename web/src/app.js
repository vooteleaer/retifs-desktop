// retifs-web's page: one node's libraries (left, Announces above them) and
// what's selected (right). Data goes in as text only (textContent), never as
// markup.
"use strict";

let status = null;
let heard = [];
// What's shown, kept in the address (#library=photos, #reports): a view can be bookmarked.
function fromHash() {
  const h = decodeURIComponent(location.hash.slice(1));
  if (h.startsWith("library=")) return { kind: "library", ns: h.slice(8) };
  if (["announces", "join", "new", "reports", "node", "settings"].includes(h)) return { kind: h };
  return { kind: "node" };
}
let selected = fromHash();
window.addEventListener("hashchange", () => { selected = fromHash(); render(); });

const $ = (id) => document.getElementById(id);

function el(tag, attrs, ...kids) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (k === "class") e.className = v;
    else if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
    else if (v !== undefined && v !== null && v !== false) e.setAttribute(k, v === true ? "" : v);
  }
  for (const kid of kids.flat()) {
    if (kid === null || kid === undefined || kid === false) continue;
    e.append(kid instanceof Node ? kid : document.createTextNode(String(kid)));
  }
  return e;
}

function toast(text) {
  const t = $("toast");
  t.textContent = text;
  t.style.display = "block";
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => (t.style.display = "none"), 4000);
}

async function get(path) {
  const r = await fetch(path, { credentials: "same-origin" });
  if (r.status === 401) { location.reload(); throw new Error("not paired"); }
  return r.json();
}

async function post(path, body) {
  const r = await fetch(path, {
    method: "POST",
    credentials: "same-origin",
    headers: { "Content-Type": "application/json", "X-Retifs": "1" },
    body: JSON.stringify(body || {}),
  });
  const v = await r.json();
  if (v && v.error) throw new Error(v.error);
  return v;
}

function human(n) {
  if (n === undefined || n === null) return "";
  const u = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (n >= 1024 && i < u.length - 1) { n /= 1024; i++; }
  return (i ? n.toFixed(1) : n) + " " + u[i];
}

function when(ms) {
  if (!ms) return "";
  return new Date(ms).toLocaleString();
}

function copyButton(text) {
  return el("button", { onclick: async () => {
    try { await navigator.clipboard.writeText(text); toast("Copied"); } catch { toast("Select it and copy"); }
  } }, "Copy");
}

// ---- the left side

function libraries() {
  return (status && status.libraries) || [];
}

function renderNav() {
  const nav = $("nav");
  nav.replaceChildren();
  const item = (sel, name, meta, star) => {
    const same = selected.kind === sel.kind && selected.ns === sel.ns;
    return el("div", { class: "item" + (same ? " sel" : ""), onclick: () => { selected = sel; render(); } },
      star ? el("span", { class: "star" }, "★") : null,
      el("span", { class: "name" }, name),
      meta ? el("span", { class: "meta" }, meta) : null);
  };
  const usable = heard.filter((h) => !h.here);
  nav.append(el("h2", {}, "Announces"));
  nav.append(item({ kind: "announces" }, "Heard on the network", usable.length ? String(usable.length) : ""));
  nav.append(el("h2", {}, el("span", {}, "Libraries"),
    el("span", {}, el("button", { class: "flat", onclick: () => { selected = { kind: "join" }; render(); } }, "Join with link"),
      el("button", { class: "flat", title: "New library", onclick: () => { selected = { kind: "new" }; render(); } }, "+"))));
  for (const l of libraries()) {
    nav.append(item({ kind: "library", ns: l.name }, l.name, l.records !== undefined ? String(l.records) : "", l.favorite));
  }
  if (!libraries().length) nav.append(el("p", { class: "dim", style: "margin:4px 10px" }, status && status.up ? "None yet." : "retifs isn't answering."));
  nav.append(el("h2", {}, "This node"));
  nav.append(item({ kind: "reports" }, "Crash and trouble reports"));
  nav.append(item({ kind: "node" }, "Status and Reticulum"));
  nav.append(item({ kind: "settings" }, "Settings"));
}

function renderHeader() {
  const dot = $("dot");
  const up = status && status.up;
  dot.className = "dot " + (up ? "up" : "down");
  let text;
  if (!status) text = "…";
  else if (up) text = `node <${status.node}> · ${libraries().length} libraries`;
  else if (status.starting) text = status.starting.text || ("starting: " + status.starting.step);
  else text = "not running: " + (status.error || "");
  $("state").textContent = text;
}

// ---- the right side

const views = {
  async library(v, ns) {
    const l = libraries().find((x) => x.name === ns);
    if (!l) { v.append(el("p", {}, "No library ", ns, " here any more.")); return; }
    const writable = l.access !== "read only";
    const grant = l.access === "read, write and invite";
    v.append(el("h2", {}, l.favorite ? el("span", { class: "star" }, "★ ") : null, l.name));
    v.append(el("p", { class: "sub" }, [l.public ? "public" : "private", l.access, l.own ? "yours" : null].filter(Boolean).join(" · ")));

    const kept = l.keep === "all";
    const info = el("div", { class: "card" },
      el("div", { class: "kv" },
        el("span", { class: "k" }, "Records"), el("span", {}, String(l.records ?? "")),
        el("span", { class: "k" }, "Peers"), el("span", {}, `${l.peers ?? 0}${l.seen ? " · " + l.seen : ""}`),
        el("span", { class: "k" }, "On this device"), el("span", {}, kept ? "kept whole" : (l.lazy ? "fetched as needed" : "records here, content fetched as needed")),
        el("span", { class: "k" }, "For"), el("span", {}, l.for || "generic")),
      l.coming ? el("div", { style: "margin-top:8px" },
        el("div", { class: "dim" }, l.kept ? `Fetching ${l.kept_here ?? 0} of ${l.kept} files…` : "Syncing…"),
        l.kept ? el("div", { class: "bar" }, el("div", { style: `width:${Math.round(100 * (l.kept_here || 0) / l.kept)}%` })) : null) : null,
      el("div", { class: "row", style: "margin-top:10px" },
        el("button", { onclick: () => act("/api/favorite", { ns, on: !l.favorite }, l.favorite ? "Not a favourite" : "A favourite") }, l.favorite ? "☆ Unfavourite" : "★ Favourite"),
        el("button", { onclick: () => act("/api/keep", { ns, all: !kept }, kept ? "Content now fetched as needed" : "Keeping all of it here") }, kept ? "Keep only what's used" : "Keep all of it here"),
        l.own ? el("button", { onclick: () => act("/api/announce", { ns, on: !l.announced }, l.announced ? "No longer announced" : "Announced as public") }, l.announced ? "Stop announcing" : "Announce as public") : null));
    v.append(info);

    // A read link.
    const linkCard = el("div", { class: "card" }, el("h3", {}, "Read link"),
      el("p", { class: "dim" }, "Whoever has it can read this library."));
    const linkBtn = el("button", { onclick: async () => {
      try { const r = await get("/api/link?ns=" + encodeURIComponent(ns)); if (r.error) throw new Error(r.error);
        linkCard.append(el("div", { class: "row" }, el("span", { class: "mono grow" }, r.text), copyButton(r.text))); linkBtn.remove();
      } catch (e) { toast(e.message); }
    } }, "Show the link");
    linkCard.append(linkBtn);
    v.append(linkCard);

    if (grant) {
      // Invitations.
      const inv = el("div", { class: "card" }, el("h3", {}, "Invite a device or a person"));
      const role = el("select", {}, el("option", { value: "read" }, "read"), el("option", { value: "write", selected: true }, "read and write"), el("option", { value: "invite" }, "read, write and invite"));
      const days = el("input", { type: "number", min: "1", value: "7", style: "width:5em" });
      const out = el("div");
      inv.append(el("div", { class: "row" }, role, el("span", {}, "for"), days, el("span", {}, "days"),
        el("button", { class: "primary", onclick: async () => {
          try { const r = await post("/api/invite", { ns, role: role.value, days: Number(days.value) || 7 });
            out.replaceChildren(el("div", { class: "row" }, el("span", { class: "mono grow" }, r.text), copyButton(r.text)));
            loadInvites(ns, list);
          } catch (e) { toast(e.message); }
        } }, "Make the invitation")), out);
      const list = el("div", { class: "list" });
      inv.append(el("h3", { style: "margin-top:12px" }, "Open invitations"), list);
      v.append(inv);
      loadInvites(ns, list);
    }

    // Members.
    const mem = el("div", { class: "card" }, el("h3", {}, "Members"));
    const mlist = el("div", { class: "list" }, el("div", { class: "dim" }, "…"));
    mem.append(mlist);
    v.append(mem);
    get("/api/members?ns=" + encodeURIComponent(ns)).then((m) => {
      mlist.replaceChildren();
      if (m.error) { mlist.append(el("div", { class: "bad" }, m.error)); return; }
      for (const x of m) {
        mlist.append(el("div", { class: "row" },
          el("span", { class: "grow" }, x.name || "~" + x.words, " ", el("span", { class: "dim" }, `· ${x.role}${x.creator ? " · made it" : ""}${x.you ? " · this device" : ""}`)),
          grant && !x.you && !x.creator ? el("button", { class: "danger", onclick: () => {
            if (confirm(`Remove ${x.name || x.words} from ${ns}? Their device keeps what it has, but gets nothing new.`)) act("/api/remove-member", { ns, writer: x.writer }, "Removed");
          } }, "Remove") : null));
      }
    });

    // The Reticulum paths to its nodes, as this node's Reticulum client sees them.
    const pathCard = el("div", { class: "card" }, el("h3", {}, "Reticulum paths"),
      el("p", { class: "dim" }, "To the nodes this library syncs with, as this node sees them now."));
    const plist = el("div", { class: "list" }, el("div", { class: "dim" }, "…"));
    pathCard.append(plist);
    v.append(pathCard);
    get("/api/paths?ns=" + encodeURIComponent(ns)).then((ps) => {
      plist.replaceChildren();
      if (ps.error) { plist.append(el("div", { class: "bad" }, ps.error)); return; }
      if (!ps.length) plist.append(el("div", { class: "dim" }, "No other nodes."));
      const ago = (ms) => {
        if (!ms) return "never heard";
        const s = Math.max(0, (Date.now() - ms) / 1000);
        return "heard " + (s < 90 ? Math.round(s) + " s" : s < 5400 ? Math.round(s / 60) + " min" : s < 172800 ? Math.round(s / 3600) + " h" : Math.round(s / 86400) + " d") + " ago";
      };
      for (const p of ps) {
        const route = p.hops === null || p.hops === undefined ? "no path" : `${p.hops} hop${p.hops === 1 ? "" : "s"}`;
        const link = p.rtt_ms ? ` · rtt ${p.rtt_ms} ms${p.rate_bps ? " · " + human(p.rate_bps) + "/s" : ""}` : "";
        plist.append(el("div", { class: "row" },
          el("span", { class: "dot " + (p.hops === null || p.hops === undefined ? "down" : (p.linked ? "up" : "")) }),
          el("span", { class: "grow" }, p.name || (p.writer ? "member" : "peer"), " ", el("span", { class: "mono dim" }, p.node)),
          el("span", { class: p.hops === null || p.hops === undefined ? "bad" : "" }, route),
          el("span", { class: "dim" }, `${link}${p.slow ? " · slow" : ""} · ${ago(p.last_heard_ms)}`)));
      }
    });

    // Leaving, deleting.
    v.append(el("div", { class: "card" }, el("h3", {}, "Leave or delete"),
      el("div", { class: "row" },
        el("button", { onclick: () => {
          if (confirm(`Leave ${ns} on this device? Other members keep theirs.${l.own ? " It was made here: if no other device has it, it's gone." : ""}`)) act("/api/leave", { ns }, "Left " + ns, true);
        } }, "Leave on this device"),
        grant ? el("button", { class: "danger", onclick: () => {
          const typed = prompt(`Delete ${ns} on every member's device? This can't be undone. Type its name to delete it:`);
          if (typed === ns) act("/api/delete", { ns }, "Deleted " + ns, true);
        } }, "Delete everywhere") : null)));
  },

  async announces(v) {
    v.append(el("h2", {}, "Announces"), el("p", { class: "sub" }, "Public libraries heard on the network. Joining one keeps its records here and fetches content as it's read."));
    const list = el("div", { class: "card list" });
    v.append(list);
    if (!heard.length) list.append(el("div", { class: "dim" }, "Nothing heard yet."));
    for (const h of heard) {
      list.append(el("div", { class: "row" },
        el("div", { class: "grow" }, el("div", {}, h.name || "(unnamed)"),
          el("div", { class: "dim" }, [h.about, `${h.hops} hop(s)`, h.here ? "here as " + h.here : null].filter(Boolean).join(" · "))),
        h.here ? el("button", { onclick: () => { selected = { kind: "library", ns: h.here }; render(); } }, "Open")
               : el("button", { class: "primary", onclick: async () => {
                   try { const r = await post("/api/join", { address: h.address }); toast("Joined as " + r.text); selected = { kind: "library", ns: r.text }; await refresh(); } catch (e) { toast(e.message); }
                 } }, "Join")));
    }
  },

  async join(v) {
    v.append(el("h2", {}, "Join with a link"), el("p", { class: "sub" }, "A library's link or an invitation (retifs:…)."));
    const link = el("input", { class: "wide", placeholder: "retifs:…" });
    v.append(el("div", { class: "card" }, link, el("div", { class: "row" },
      el("button", { class: "primary", onclick: async () => {
        try { const r = await post("/api/open", { link: link.value.trim() }); toast("Joined as " + r.text); selected = { kind: "library", ns: r.text }; await refresh(); } catch (e) { toast(e.message); }
      } }, "Join"))));
  },

  async new(v) {
    v.append(el("h2", {}, "New library"), el("p", { class: "sub" }, "Made on this node: its keys are made here."));
    const name = el("input", { placeholder: "name" });
    const purpose = el("select", {}, ...["", "photos", "music", "contacts", "calendar", "maps"].map((p) => el("option", { value: p }, p || "generic")));
    v.append(el("div", { class: "card" }, el("div", { class: "row" }, name, el("span", {}, "for"), purpose,
      el("button", { class: "primary", onclick: async () => {
        const n = name.value.trim(); if (!n) return;
        try { await post("/api/create", { name: n, for: purpose.value }); toast("Made " + n); selected = { kind: "library", ns: n }; await refresh(); } catch (e) { toast(e.message); }
      } }, "Make it"))));
  },

  async reports(v) {
    v.append(el("h2", {}, "Crash and trouble reports"),
      el("p", { class: "sub" }, "From every device in the library retifs-crashlogs (retifs ADR-0036): panics, unclean stops, Reticulum gone, app crashes."));
    const note = el("textarea", { placeholder: "What went wrong? retifsd's last output is added." });
    v.append(el("div", { class: "card" }, el("h3", {}, "Write a report"), note, el("div", { class: "row" },
      el("button", { onclick: async () => {
        try { await post("/api/report", { text: note.value.trim() || "Reported from the web UI." }); note.value = ""; toast("Kept; it goes into the library shortly"); } catch (e) { toast(e.message); }
      } }, "Send"))));
    const list = el("div", { class: "card list" }, el("div", { class: "dim" }, "…"));
    const text = el("div");
    v.append(list, text);
    const r = await get("/api/reports");
    list.replaceChildren();
    if (r.error) { list.append(el("div", { class: "dim" }, "This node isn't in the crash library (", r.error, "). Join retifs-crashlogs with an invitation from one of your devices.")); return; }
    if (!r.length) list.append(el("div", { class: "dim" }, "No reports."));
    for (const x of r) {
      list.append(el("div", { class: "row", style: "cursor:pointer", onclick: async () => {
        text.replaceChildren(el("p", { class: "dim" }, "…"));
        const t = await get("/api/report?path=" + encodeURIComponent(x.path));
        text.replaceChildren(el("div", { class: "card" }, el("h3", {}, x.path), el("pre", {}, t.text || t.error || "")));
        text.scrollIntoView({ behavior: "smooth" });
      } },
        el("span", { class: x.kind === "panic" ? "bad" : "" }, x.kind),
        el("span", { class: "grow" }, x.device, el("span", { class: "dim" }, ` · ${x.source} · ${x.version || ""}`)),
        el("span", { class: "dim" }, when(x.mtime))));
    }
  },

  async node(v) {
    v.append(el("h2", {}, "This node"));
    if (status) {
      const u = status.usage || {};
      v.append(el("div", { class: "card" }, el("h3", {}, "retifs"), el("div", { class: "kv" },
        el("span", { class: "k" }, "State"), el("span", {}, status.up ? "running" : (status.starting ? status.starting.text : "not running")),
        el("span", { class: "k" }, "Node"), el("span", { class: "mono" }, status.node || ""),
        el("span", { class: "k" }, "Store"), el("span", { class: "mono" }, status.store || ""),
        el("span", { class: "k" }, "Own content"), el("span", {}, human(u.pinned)),
        el("span", { class: "k" }, "Held for others"), el("span", {}, human(u.leased)),
        el("span", { class: "k" }, "Cache"), el("span", {}, `${human(u.cache)} of ${human(u.cache_max)}`),
        el("span", { class: "k" }, "Store limit"), el("span", {}, human(u.store_limit)),
        el("span", { class: "k" }, "Disk"), el("span", {}, `${human(u.disk_available)} free of ${human(u.disk_total)}`))));
    }
    const ret = el("div", { class: "card" }, el("h3", {}, "Reticulum"), el("div", { class: "dim" }, "…"));
    v.append(ret);
    const r = await get("/api/reticulum");
    ret.replaceChildren(el("h3", {}, "Reticulum"),
      el("p", {}, r.running ? `Running${r.uptime ? ", up " + Math.round(r.uptime / 3600) + " h" : ""}` : "Not running on this machine" + (r.error ? ": " + r.error : "")));
    const list = el("div", { class: "list" });
    for (const c of r.connections || []) {
      list.append(el("div", { class: "row" }, el("span", { class: "dot " + (c.up ? "up" : (c.enabled ? "down" : "")) }),
        el("span", { class: "grow" }, c.name, el("span", { class: "dim" }, " · " + c.kind + (c.enabled ? "" : " · off"))),
        el("span", { class: "dim" }, `↓ ${human(c.rx)} ↑ ${human(c.tx)}`)));
    }
    ret.append(list);
  },

  async settings(v) {
    v.append(el("h2", {}, "Settings"), el("p", { class: "sub" }, "This node's retifs.toml. They take effect when retifs restarts."));
    const list = await get("/api/settings");
    const fields = {};
    const card = el("div", { class: "card" });
    for (const s of list) {
      const key = `${s.section}.${s.key}`;
      const input = s.switch ? el("input", { type: "checkbox", checked: s.value === "true" }) : el("input", { value: s.value || "", placeholder: s.default || "" });
      fields[key] = input;
      card.append(el("div", { class: "row" }, el("span", { class: "grow" }, s.what, el("div", { class: "dim mono" }, key + (s.unit ? " (" + s.unit + ")" : ""))), input));
    }
    card.append(el("div", { class: "row" }, el("button", { class: "primary", onclick: async () => {
      const values = {};
      for (const [k, i] of Object.entries(fields)) values[k] = i.type === "checkbox" ? String(i.checked) : i.value;
      try { await post("/api/settings", { values }); toast("Saved; restart retifs for them to take effect"); } catch (e) { toast(e.message); }
    } }, "Save")));
    v.append(card);
  },
};

async function loadInvites(ns, list) {
  const r = await get("/api/invites?ns=" + encodeURIComponent(ns));
  list.replaceChildren();
  if (r.error) { list.append(el("div", { class: "bad" }, r.error)); return; }
  const open = r.filter((i) => !i.spent);
  if (!open.length) list.append(el("div", { class: "dim" }, "None."));
  for (const i of open) {
    list.append(el("div", { class: "row" }, el("span", { class: "grow" }, i.role, el("span", { class: "dim" }, " · until " + when(i.expires))),
      el("button", { onclick: () => act("/api/cancel-invite", { ns, id: i.id }, "Cancelled") }, "Cancel")));
  }
}

async function act(path, body, said, gone) {
  try {
    await post(path, body);
    toast(said);
    if (gone) selected = { kind: "node" };
    await refresh();
  } catch (e) { toast(e.message); }
}

let rendering = false;
async function render() {
  const hash = selected.kind === "library" ? "library=" + encodeURIComponent(selected.ns) : selected.kind;
  if (decodeURIComponent(location.hash.slice(1)) !== decodeURIComponent(hash)) history.replaceState(null, "", "#" + hash);
  renderHeader();
  renderNav();
  if (rendering) return;
  rendering = true;
  const v = $("view");
  const fresh = el("div");
  try { await views[selected.kind](fresh, selected.ns); } catch (e) { fresh.append(el("p", { class: "bad" }, e.message)); }
  v.replaceChildren(fresh);
  rendering = false;
}

async function refresh() {
  try { status = await get("/api/status"); } catch { status = null; }
  try { const h = await get("/api/heard"); heard = Array.isArray(h) ? h : []; } catch { heard = []; }
  await render();
}

// The left side and header stay current; the right side is redrawn only when
// the user does something (it may be half filled in).
setInterval(async () => {
  try { status = await get("/api/status"); const h = await get("/api/heard"); heard = Array.isArray(h) ? h : heard; } catch { return; }
  renderHeader();
  renderNav();
}, 10000);

refresh();
