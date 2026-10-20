# About

Zephon is a Windows 11 app (GUI and command line) that captures system audio with WASAPI
loopback and streams it to AirPlay 2 receivers. It is a thin Rust app around the
[Nyx-Refrain](https://github.com/pStrikeZ/Nyx-Refrain) crates, with a gpui-kit GUI.

Read first:

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): protocol, module roles, latency profiles, loss
  stats.
- [docs/GLOSSARY.md](docs/GLOSSARY.md): the terms used in code and docs (sender, receiver, capture
  endpoint, lag, latency profile).
- [docs/adr/](docs/adr/): why this stack.
- [docs/design/DESIGN.md](docs/design/DESIGN.md): the UI design, before any UI change.
- [CONTRIBUTING.md](CONTRIBUTING.md): the human contributor guide. This file adds to it.

## Commands

The project builds on Windows only, with the MSVC toolchain. Before you report a change as
done, run each step of `.github/workflows/ci.yml` locally. Keep its env vars, such as
`RUSTDOCFLAGS=-D warnings` for `cargo doc`.

- MSRV is Rust 1.95 (`rust-version` in `Cargo.toml`). Do not use std APIs newer than 1.95.
- Always pass `--locked`. Change `Cargo.lock` only when the task is a dependency change.
- `ZEPHON_DEMO=1 cargo run` starts the GUI with made-up rooms and a temporary settings folder.
  Use it to check UI changes by hand and for screenshots.

## Rules

### Performance and responsiveness

Priority order: streaming performance, then UI responsiveness, then everything else. The window
must never make the sound stutter. The window must react at once: a click, a window drag or a
slider drag must never wait for a frame. A slow frame is a bug. Read "UI responsiveness" in
`docs/ARCHITECTURE.md` before any UI change.

- Do nothing slow on the UI thread: no blocking I/O, no lock held during work, no wait for the
  controller. The controller and the audio path never wait for the UI either.
- In a release build, the budget for a full frame is 5 ms, with a hard limit of 10 ms. A full
  frame is each step of a window drag and each state change.
- The hard limit for a meter frame is 1 ms.
- When an accessibility client is attached, the content is not cached. Then each meter frame is
  a full frame (see `ui::meter`). This is one more reason to keep full frames cheap.
- Keep the title bar out of the cached content. Otherwise the replayed view loses the caption
  buttons and the drag area, and clicks on them fail.
- Give flex columns a definite width (`v_flex().w_full()`). A column that only stretches is laid
  out again on each pass of its parent. Each level of nesting multiplies the cost.
- Keep repeated rows shallow. In rows, use gpui-base unstyled primitives, not gpui-kit styled
  controls.
- Draw a frame only when something on screen changes. A timer or animation that does not stop
  uses CPU while the app is open. Example: the meter stops when no room plays or the window is
  minimized. It also skips frames in which no bar moves.
- Measure, do not guess. Each timing test in `ui::perf` compares its frame with another frame
  drawn in the same run: a full frame with a reference frame, a meter frame with a full frame.
  Thus the check is valid on any machine. In a release build,
  the tests also fail a frame over its hard limit. CI runs them in a release build.
- Before and after a UI change that adds to a room row or the title bar, run
  `cargo test --locked --release ui::perf -- --nocapture`. Compare the printed times with the
  5 ms target.
- If a change adds a new kind of frame or window, add a test in `ui::perf`.
- Do not loosen a budget in `ui::perf` to make a change pass. Make the change cheaper.

### Never touch the real machine from tests or tools

Tests must not change the Windows Firewall, the registry or the real `%APPDATA%\Zephon` folder.
Tests must not show a UAC prompt. The code has seams for this. Use them:

- Settings writes go through the engine's injectable writer. Settings tests use temporary
  folders.
- Firewall script tests run the PowerShell scripts against stub cmdlets (see the test harness in
  `src/firewall.rs`). Do not weaken its allowlist checks.

Do not add `cfg!(test)` branches to production code to skip side effects. Add a seam instead.

### No personal data

This is a public repository. Never commit these from a real setup:

- device or room names
- IP addresses
- user names or email addresses
- log excerpts or screenshots

In tests, docs and examples, use made-up names such as "Bedroom TV". Use documentation addresses
from `192.0.2.0/24`. Take screenshots in demo mode only.

### Tests must be able to fail

Test behavior, not code structure. Do not write tests that restate a constant or compare a value
with itself. Before you add a test, prove that it can fail:

1. Put in the bug.
2. Run the test and see it fail.
3. Take the bug out.

For timing logic, use tokio paused time (`start_paused`), not real sleeps.

### Pinned dependencies

- The Nyx-Refrain crates are pinned to one git `rev`. Do not bump it as part of another change.
  Before a bump, a human must read the upstream diff, especially `raop/src/ap2/session.rs`. The
  human must also do a listening test on real receivers. An agent cannot do these steps, so
  leave bumps to a human.
- `gpui-kit` and `gpui-pre` are pinned to exact versions. Bump them together, in their own pull
  request.
- GitHub Actions are pinned by commit SHA, with the version in a comment. Keep that form.

### Code style

- Match the surrounding code. Comments explain why, not what.
- When you add, remove or move a module, update the code layout table in
  `docs/ARCHITECTURE.md`.
- `unsafe` blocks need a `// SAFETY:` comment (clippy `undocumented_unsafe_blocks`).
- Doc comments must build cleanly with `RUSTDOCFLAGS=-D warnings` (CI checks this).
- User-facing text is plain, short English. When behavior changes, update README, CHANGELOG and
  the docs.

### Git

- One change per commit or pull request.
- Do not push, tag or publish a release unless asked. Tags starting with `v` run the release
  workflow.
