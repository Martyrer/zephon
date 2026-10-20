# Architecture

Zephon is a thin Rust app around the [Nyx-Refrain](https://github.com/pStrikeZ/Nyx-Refrain)
library crates `raop`, `capture`, `pipeline`, `discovery` and `netutil`. They are git
dependencies pinned to `rev = "0a2ef7b"` (release 0.9.3). The reasons for this stack are in
[ADR 0001](adr/0001-airplay2-ntp-pcm-on-nyx-refrain-crates.md). Terms are defined in the
[glossary](GLOSSARY.md), and the UI design is in [design/DESIGN.md](design/DESIGN.md).

Before bumping the Nyx `rev`, read the diff of these crates (especially
`raop/src/ap2/session.rs`) and re-run the listening and latency checks.

## Protocol

AirPlay 2 realtime (stream type 96) with transient pairing, ChaCha20, NTP timing and raw PCM
44.1 kHz/16-bit/stereo. Each receiver gets its own `Ap2Session`. A session:

- sets `quiet_mode: false` and a real `volume_db` (`Ap2Config::default()` is silent);
- wraps the start in a 10 s timeout;
- sends `feedback()` every 2 s to keep the session open, and counts the connection as lost
  when a keep-alive or volume change gets no answer within 10 s (TCP resends on a lossy Wi-Fi
  link often delay an answer past 3 s while the audio still plays). Stop does not wait for a
  request in flight;
- has the capture hub feed its queue only once it streams, so a slow start cannot overflow
  the queue and leave a gap that throws off Nyx's drift estimate;
- applies live volume changes and forwards the receiver's volume reports;
- sends TEARDOWN when it stops.

Silence fill and drift compensation stay inside Nyx: the keep-alive render stream, silence on
underrun or rebuild, and a drift compensator per session.

## Code layout

| Module | Role |
|---|---|
| `settings` | JSON settings and the `%APPDATA%\Zephon` paths |
| `capture` | Capture endpoint list and choice. `CaptureHub` reads one WASAPI loopback source and fans it out into one `AudioSource` queue per session. |
| `level` | `LevelTap`: a lock-free ring of each session's chunk peaks by capture time, recorded on Nyx's feeder thread and read by the window's level meters |
| `discovery` | mDNS browse on the LAN interfaces only, lookup by name or IP, `GET /info` volume read |
| `advice` | `LossStats` and `suggest_profile` (the "Glitchy?" profile hint) |
| `session` | One `Ap2Session` per receiver, and the latency profiles |
| `firewall` | Check (NetSecurity PowerShell, per protocol) and the one-UAC-prompt install, both the app's own (paths passed to PowerShell as base64, never as script text) |
| `autostart`, `winreg` | Start with Windows (the HKCU `Run` value) and small HKCU registry helpers |
| `controller` | UI-agnostic app state, commands and session events, resume on launch. The UI only sends `Command`s and renders `AppState` snapshots. |
| `ui` | gpui-kit windows (main and settings). The controller's notifier wakes a GPUI foreground task through a `futures` unbounded channel; it refreshes one shared snapshot entity both windows observe. The shared look (`ui::appearance`) is a GPUI global. The main window's content is a cached view; the level meters (`ui::meter`) are a sibling view painted over it, so meter frames do not re-render the content (except while an accessibility client is attached: a reused cached view drops out of the accessibility tree). The title bar is a view of its own outside the cache (see "UI responsiveness"). `ui::perf` holds the frame tests. |
| `cli` | Argument parsing, the subcommands, console attach, GUI launch |
| `logging` | The log file (rotated past 5 MB) and the console log |
| `single_instance` | One GUI at a time: a named mutex, and a second launch brings the first window forward |

The code is a library (`src/lib.rs`) plus a thin executable (`src/main.rs`) that only sets
the Windows GUI subsystem and calls `zephon::run`. The headless core modules are public, so
integration tests in a `tests/` folder could use them; `cli`, `logging`, `single_instance`,
`ui` and `winreg` are private. `controller` is split into `mod.rs` (the public state and commands),
`engine.rs` (the background task that owns the sessions) and `demo.rs` (demo mode).

Each session task is independent, so one receiver failing never stops the others.

## UI responsiveness

Streaming comes first and the window second: the window must never slow the sound down, and
it must feel instant. Two kinds of frame matter.

- **Full frame.** Nothing is reused. GPUI draws one for each change to the shown state, and
  for each step of a window drag: every `WM_MOVE` refreshes the whole window, and during the
  drag GPUI draws from a 10 ms timer. A full frame that takes longer than that makes the drag
  stutter. With eight rooms, four of them playing, it takes about 2.7 ms in a release build
  (about 7 ms before the layout fixes below); with all eight playing, about 3.7 ms.
- **Meter frame.** Up to 30 a second while a room plays. It draws the meter layer, the title
  bar and gpui-kit's root, and replays the cached content: about 0.4 to 0.5 ms. While an
  accessibility client is attached nothing is replayed (see `ui::meter`), so each meter frame
  is a full frame.

What keeps them cheap:

- **The title bar is not cached.** Windows asks on each click in the title bar whether it hit
  a caption button or the drag area (`WM_NCHITTEST`), and GPUI answers from the
  window-control areas of the last frame. A cached view that is replayed does not carry them
  over (gpui-pre 0.3.9: `Window::reuse_paint` does not replay `window_control_hitboxes`), so
  a title bar inside the cached content lost its buttons on every meter frame.
- **Columns have a definite width.** A flex column whose width comes from stretching (a plain
  `v_flex()` in another column) is laid out again in each layout pass of its parent, so each
  level of nesting doubles the cost of what is inside. Give columns `w_full()`. Ten nested
  stretched columns over 100 rows took 220 ms; with `w_full()`, 2 ms.
- **Deep leaves are placed absolutely.** Flex rows also lay out their children more than
  once. The profile labels under each room's picker sit absolutely in a row of fixed height,
  so they are laid out once. Each is at most a third of the row wide, so they cannot overlap.
  With a wide font (Verdana, say) and the window just wider than the narrow layout, the chosen
  label can need more than a third, and its figure is cut short.
- **Repeated controls use gpui-base's primitives** (the switch, the sliders, the profile
  labels): gpui-kit's styled controls cost several times as much to lay out.

`ui::perf` runs the main and settings windows on GPUI's test platform with the real text
system. It checks what a meter frame draws, and checks each frame's cost against a reference
frame timed alongside it, so the limits hold on a slow or busy machine. In a release build it
also checks the times against hard limits: 10 ms for a full frame, 1 ms for a meter frame. CI
runs it in a release build. Debug builds optimize
the dependencies (`[profile.dev.package."*"]`), so `cargo run` and these tests come close to
the release build.

## Reconnect

A session that fails after it was streaming (the keep-alive or a volume change got no answer
within 10 s, or the receiver closed the connection) ends as usual. The controller then starts a
new session for that room, up to 3 times, with exponential backoff: 2, 4 and 8 s after the
failed session has ended. An attempt resets the count only once it has played for 60 s; one
that drops sooner counts as failed, so a link that keeps dropping ends in "gave up" instead of
reconnecting forever. A failed first start is not
retried, and a start timeout during a reconnect does not raise the firewall banner, since the
room played before. After the last attempt the room shows the first cause and what to check.

When every room that is on has failed and none is playing, connecting or waiting to reconnect,
the controller stops playback (and closes the capture), so the UI offers Play again.

## Capture endpoint

The capture endpoint is chosen in this order:

1. the saved endpoint, by ID, then by name;
2. the Windows default playback device.

If the saved endpoint is missing, the app reports an error. It never falls back silently,
because the Windows default can be a silent device (for example a virtual mixer input).

## Latency profiles

| Profile | Lag behind the PC's speakers |
|---|---|
| `low` | about 125 ms |
| `balanced` (default) | about 165 ms |
| `stable` | about 257 ms |

Measured on a HomePod mini. The profile is saved per room. Changing it while the room plays
restarts only that room's session.

## Loss and resend stats

When a receiver misses audio packets, it asks for them again on the RAOP control channel
(`0x80 0xD5` retransmit request). Nyx answers these in its control thread (from a
`PacketHistory` of the last 512 packets, about 4 s) and counts them in the public `Ap2Stats`
(`Ap2Session::stats()`): `retransmit_requests`, `retransmit_served` and `retransmit_missing`.
Each session reads them every 5 s and logs:

```
[Kitchen] stats: profile stable, packets 7512, timing requests 61, retransmit requests 14,
resent 22, missed 0; last 60 s: 6.0 requests/min, 0.0 missed/min; buffer 12, drift 3.1 ppm
```

- **retransmit requests**: requests received (one request can name several packets);
- **resent**: packets sent again;
- **missed**: packets asked for that were no longer in the history, or never sent. The receiver
  cannot get them in time, so they are likely audible.

The `last 60 s` rates (`LossStats`) drive the row's loss line and the profile hint. Nyx does not
tell whether a resent packet arrived in time to be played; only `missed` is known to be lost.

## Files

`%APPDATA%\Zephon\` holds:

- `settings.json`: the capture endpoint (ID and name), `default_profile` and `default_volume`
  (new rooms), per-receiver `selected`, `profile`, `volume` and `hidden`, an optional
  `interface` pin such as `"Ethernet"`, `was_playing`, `resume_on_launch`,
  `volume_follows_receiver`, and the look: `theme`, `backdrop`, `font_family`, `text_size`,
  `accent`. Every key is optional, so older files load with the defaults.
- `log.txt`: the `tracing` log. The app logs at debug level, including a stats line per session
  every 5 s. Set `RUST_LOG` to change the console log level.
- `receivers.toml`: the receiver cache, so known rooms appear at once (as Offline until mDNS
  sees them).

## Windows integration

- **Console.** The exe is a GUI-subsystem program, so the GUI opens without a console window.
  With arguments, it attaches to the parent terminal's console and prints there.
- **Firewall.** `firewall --install` runs one elevated Windows PowerShell script (the absolute
  path under the system directory, never a search). It removes inbound block rules and older
  allow rules for this exact exe path only, then adds an inbound allow rule named
  `Zephon - <full exe path>` (all protocols, profile Any). Paths reach the script only as
  base64-encoded UTF-16 that the script decodes, so no folder name can break out of a string.
  The check requires enabled inbound allow rules for the exe path that cover TCP and UDP. Port
  and profile limits on a rule are not checked. A failed query is reported as an error, never
  as "firewall off".
- **Start with Windows.** The value `Zephon` (this exe's quoted path) under
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
- **Accent color.** Read from `HKCU\...\Explorer\Accent\AccentPalette` (DWM `AccentColor` if
  missing), then adjusted until text on it and the accent on the glass reach 4.5:1 contrast. A
  thread watches the key, so changes apply live.
- **mDNS adapters.** Queries go only to adapters that are up, physical and have a
  non-link-local IPv4 address. If none qualifies, Nyx's own choice is used. With several LAN
  adapters, one browse runs on each.

## Hidden commands

`zephon status` starts the app controller headless and prints what the UI would show: capture
endpoint, receivers with their volumes, and firewall state.
