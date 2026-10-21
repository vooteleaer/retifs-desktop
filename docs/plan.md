# retifs-desktop: plan

A desktop management utility for retifs, in the spirit of the Nextcloud desktop client: a tray icon that says whether everything is fine, and a window to see and manage what retifs holds, where, and who shares it. Linux first, Windows too. Android and Haiku get their own native apps later.

## Principles
- **A client of the daemon, nothing more.** All state lives in `retifsd`; the utility talks to it only through `libretifs` (the owner's socket, or `--via` an own node). It holds no data and no keys of its own, so it can be closed, crash or be replaced without consequence.
- **Manages retifs, doesn't catalogue content** (the user's rule, 2026-10-21). It shows this node, storage, namespaces as units (members, sharing, what's kept where, redundancy) and devices. Browsing, searching and viewing records is what apps are for (a photo album, a music player…), and the file tree is the mount's job; the utility only switches the mount on and off.
- **Say what's true.** "Only here", "2 copies", "waiting for the grant", "the content isn't reachable": the daemon's real states, not a green tick.
- **No new protocol.** Where the local API lacks something the utility needs, it's added to retifs as a request (listed below), not worked around here.

## Architecture
Native widgets on each platform (the user's choice, 2026-10-21: no webview), over one shared core, the same pattern as retifs' own frontends (FUSE on Linux, userlandfs on Haiku):
```
 gtk/      GTK 4 + libadwaita (Linux)        ─┐
 windows/  native Windows controls (later)    ├── core/ (Rust) ── libretifs ── retifs.sock ── retifsd
 haiku/    BeAPI (later)                     ─┘
```
- **core/** wraps `libretifs` with a small tokio runtime and offers blocking calls (status, namespaces, search, history) and shared logic (search terms, what changed between versions, sizes). A front end runs them on a worker thread and shows the results on its UI thread.
- **gtk/** is the Linux app: an `AdwNavigationSplitView` with a sidebar (overview, namespaces), preference groups for the overview, a banner while the daemon isn't running, the node in the window subtitle.
- **Windows:** native controls; Win32 from Rust (`windows-rs`) or a WinUI app over a C interface to the core, decided when it starts.
- **Haiku:** a BeAPI app; Tracker already shows attributes.
- The tray: StatusNotifierItem on Linux (KDE natively, GNOME with the AppIndicator extension), the notification area on Windows.
- `libretifs` comes from the retifs repo (a path or git dependency).

## Views
| View | Shows | Local API today |
|---|---|---|
| **Tray** | daemon up or down; peers reachable; data still "only here"; errors; a menu to open the window, pause, quit | `Ping`; health summary missing (below) |
| **Overview** | this node (address, device name), storage use (own, held for others, cache, reserve, volume), redundancy at a glance | `Ping` (usage) |
| **Namespaces** | each namespace: records, writers and rights, peers, checkpoint, keep mode, lazy or full | `Namespaces`, `NsInfo` |
| **Namespace** | its address to share, members and their rights (grant, revoke), what this device keeps, peers, record count, checkpoint | `NsInfo`, `NsGrant`, `NsRevoke` |
| **Sharing** | share read-only as a QR code or link; invite to write (one-use invitation, ADR-0020); followed channels | `NsAddress`, `Follow`, `Channels`; invitations missing |
| **This device keeps** | keep mode and rules per device ("keep album Prangli here"): the counterpart of selective sync | `NsKeep`, keep-rule records |
| **Redundancy** | per namespace: copies and holders, what's only here, repairs in progress | `Status` per file; namespace summary missing |
| **Devices and apps** | own nodes admitted for `--via`, app tokens, adding and removing them | `AppList`, `AppAdd`, `AppRemove` |
| **Activity** | sync activity: which namespaces changed, from which member and device, how many records (not what's in them) | `Watch` wakes it; a change list is missing |
| **Settings** | mount on/off and where; gateway on/off; disk budget; start at login | systemd units; budget flags only on the command line today |

## Needed in retifs (local API)
1. **Health summary:** one request for the tray: peers reachable, files only here, files being placed or repaired, last errors.
2. **Activity:** recent changes of a namespace (commit time, writer, which records, which keys), paged. `Watch` says *that* something changed; this says *what*.
3. **Redundancy per namespace:** copies and holders summarised over its files, not one `Status` call per file.
4. **Invitations** (ADR-0020): make one, list pending, cancel; redeeming happens in the daemon.
5. **Settings at run time:** disk budget and keep defaults changeable without restarting the daemon.
6. **A daemon event stream** (optional, later): one long poll for everything instead of one `Watch` per namespace.

## Milestones
Each one is usable on its own.
1. **Skeleton:** connects to `retifsd`; overview with the node, storage use and namespaces; a banner while it isn't running (GTK done; the tray icon comes with milestone 6).
2. **Namespaces:** each namespace's management page: address to share, members and rights, what this device keeps, peers, checkpoint (GTK done).
3. **Sharing, read-only:** QR code and link for a namespace (`retifs:ns/…?q=…`), follow from a pasted link.
4. **Keeping:** keep mode and rules per device; mount and gateway switches.
5. **Devices and apps:** own nodes and app tokens.
6. **Health and redundancy:** after retifs gains the summary requests (1, 3).
7. **Activity:** after the change list (2).
8. **Invitations:** after ADR-0020 is implemented (4).
9. **Packaging:** AppImage/.deb (Linux), MSI (Windows), start at login.

## Open
- Whether the utility also manages a node over `--via` (a headless home server from the laptop).
