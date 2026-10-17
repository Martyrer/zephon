# AGENTS.md

Instructions for coding agents working on Zephon. `CLAUDE.md` is a symlink to this file, so
edit this file only.

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

The project builds on Windows only (MSVC toolchain). CI runs these on every pull request, so
run them before you say a change is done:

```
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo doc --no-deps --locked        # with RUSTDOCFLAGS=-D warnings
cargo build --locked --release
```

- MSRV is Rust 1.95 (`rust-version` in `Cargo.toml`); CI checks it. Do not use newer std APIs.
- Always pass `--locked`. Change `Cargo.lock` only when the task is a dependency change.
- `ZEPHON_DEMO=1 cargo run` starts the GUI with made-up rooms and a temporary settings folder.
  Use it to check UI changes by hand and for screenshots.

## Layout

- `src/lib.rs`: the library. Public core modules: `advice`, `autostart`, `capture`,
  `controller`, `discovery`, `firewall`, `session`, `settings`. Private: `cli`, `logging`,
  `single_instance`, `ui`, `winreg`.
- `src/main.rs`: only sets the Windows GUI subsystem and calls `zephon::run()`. Keep it that way.
- `src/controller/`: `mod.rs` (public state and commands), `engine.rs` (the background task
  that owns the sessions), `demo.rs` (demo mode).
- `src/ui/`: gpui-kit windows. The UI only sends `Command`s and renders `AppState` snapshots;
  app logic belongs in the controller, not the UI.

When you add, remove or move a module, update the code layout table in `docs/ARCHITECTURE.md`.

## Rules

### Never touch the real machine from tests or tools

Tests must not change the Windows Firewall, the registry, the real `%APPDATA%\Zephon` folder,
or show a UAC prompt. The code has seams for this; use them:

- settings writes go through the engine's injectable writer, and settings tests use temporary
  folders;
- firewall script tests run the PowerShell scripts against stub cmdlets (see the test harness
  in `src/firewall.rs`). Do not weaken its allowlist checks.

Do not add `cfg!(test)` branches to production code to skip side effects. Add a seam instead.

### No personal data

This is a public repository. Never commit real device or room names, real IP addresses, user
names, email addresses, log excerpts or screenshots from a real setup. In tests, docs and
examples use made-up names such as "Bedroom TV" and documentation addresses from
`192.0.2.0/24`. Take screenshots in demo mode only.

### Tests must be able to fail

A test checks behavior, not that the code is written a certain way. Before you add a test,
make sure it fails when the behavior breaks: put in the bug, see the test fail, take the bug
out. Do not write tests that restate a constant or compare a value with itself. Use tokio
paused time (`start_paused`) for timing logic, not real sleeps.

### Pinned dependencies

- The Nyx-Refrain crates are pinned to one git `rev`. Do not bump it as part of another
  change. A bump needs the upstream diff read (especially `raop/src/ap2/session.rs`) and a
  listening test on real receivers, which an agent cannot do; leave that to a human.
- `gpui-kit` and `gpui-pre` are pinned to exact versions and must be bumped together, in their
  own pull request.
- GitHub Actions are pinned by commit SHA, with the version in a comment. Keep that form.

### Decisions left to the maintainer

`advice::suggest_profile` is a `TODO(user)` stub on purpose: its thresholds are a product
decision. Do not fill it in unless asked. Its specs are `#[ignore]`d until it exists.

### Code style

- Match the surrounding code. Comments explain why, not what.
- `unsafe` blocks need a `// SAFETY:` comment (clippy `undocumented_unsafe_blocks`).
- Doc comments must build cleanly with `RUSTDOCFLAGS=-D warnings` (CI checks this).
- User-facing text is plain, short English. Keep README, CHANGELOG and the docs in step with
  behavior changes.

### Git

- One change per commit or pull request.
- Do not push, tag or publish a release unless asked. Tags starting with `v` run the release
  workflow.
