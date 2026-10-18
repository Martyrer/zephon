# Contributing

Thanks for helping. Bug reports, testing on receivers other than HomePod mini, and pull requests
are all welcome.

## Reporting a bug

Open an issue with the bug report form. Include:

- Windows version, Zephon version (Settings > About) and the receiver models;
- what you did, what you expected and what happened;
- the relevant part of `%APPDATA%\Zephon\log.txt`.

The log contains your receivers' names and local IP addresses. Remove them first if you prefer.

For security problems, follow [SECURITY.md](SECURITY.md) instead.

## Development

You need Windows, Rust stable (MSVC toolchain) and the Visual Studio C++ build tools.

```
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

CI runs the same commands on every pull request.

Notes:

- `Cargo.lock` is committed. Build with `--locked`, and update it only on purpose.
- The Nyx-Refrain crates are pinned to one commit. Before bumping `rev`, read the upstream diff
  (especially `raop/src/ap2/session.rs`) and test playback and latency by ear.
- gpui-kit is pinned to an exact version because it ships breaking changes often. Upgrade it in
  its own pull request.
- The exe's firewall rule is tied to its path, so `target\debug` and `target\release` each need
  their own rule (`cargo run -- firewall --install`).
- `ZEPHON_DEMO=1` starts the GUI with made-up rooms and devices and a temporary settings folder.
  Use it for screenshots, so that your own room names and IP addresses stay private.
- Debug builds optimize the dependencies (`[profile.dev.package."*"]` in `Cargo.toml`), so the
  window in `cargo run` is close to the release build. Only Zephon's own code is unoptimized.
  The first debug build takes a few minutes longer.

## Pull requests

- Keep each pull request to one change, and describe how you tested it (which receivers, how
  long).
- Match the style of the surrounding code: comments explain why, not what.
- New behavior in the controller or settings gets a unit test where that is practical.

## License

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
the work by you, as defined in the Apache-2.0 license, shall be dual licensed as in the
[README](README.md#license), without any additional terms or conditions.
