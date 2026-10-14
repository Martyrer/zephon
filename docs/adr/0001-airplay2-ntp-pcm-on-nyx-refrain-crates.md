---
status: accepted
date: 2026-10-12
---

# AirPlay 2 realtime with NTP and raw PCM, built on pinned Nyx-Refrain crates

## Context

HomePod Software 27.0 (September 2026) stopped accepting RAOP (AirPlay 1), which is why
TuneBlade can still discover HomePod minis but cannot play to them. In testing, a HomePod mini
reset a plain RAOP ANNOUNCE but played two senders that used AirPlay 2 with transient pairing,
ChaCha20, NTP timing, realtime stream type 96 and raw PCM 44.1/16/2: pyatv and Nyx-Refrain.
Nyx-Refrain is MIT-licensed Rust that also handles Windows loopback capture, silence fill, device
changes and drift. Measured on a HomePod mini, it lags the local speakers by about 165 ms at its
`balanced` profile, repeatable to ±5 ms. Rust was the preferred language.

## Decision

Build the real app as a thin Rust binary that speaks AirPlay 2 realtime with NTP timing and
raw PCM. It depends on Nyx-Refrain's library crates (`raop`, `capture`, `pipeline`,
`discovery`, `netutil`) as git dependencies pinned to commit `0a2ef7b`. Our own code is only
glue: latency profile, session loop with the 2 s keep-alive, receiver lookup, capture device
selection, firewall check, and settings.

## Considered options

- **RAOP (AirPlay 1)**: rejected. It is dead on 27.0, which is what broke TuneBlade.
- **PTP timing with ALAC** (the path OwnTone and Music Assistant use): not chosen now. NTP and
  PCM are proven on HomePod mini and already implemented in Nyx; PTP/ALAC is unbuilt on Windows. It
  is kept as the fallback if Apple stops accepting NTP or PCM.
- **Python + pyatv**: rejected for the real app. It requires Python 3.12 (0.18.0 breaks on
  3.14), relies on monkeypatched internals for live input and low latency, and its lag varies by
  about ±40 ms per session (215-290 ms). It remains the working fallback.
- **Fork Nyx-Refrain**: rejected as the default. We do not want to own 25k lines of young,
  single-maintainer code. A pinned dependency gets upstream fixes with a `rev`
  bump, and forking later is cheap (change the git URL or `[patch]` one crate).
- **airplay2-rs as base**: rejected. It is GPL-3, ALAC only, has no Windows capture, and does
  not build on Windows. It stays a protocol reference.
- **StreamToSpeaker as base**: rejected. It uses PTP grandmaster and buffered AAC, the
  combination 27.0 is reported to break, and captures through a kernel driver. Its PTP/ALAC
  pieces stay a reference.

## Consequences

- The app must override the `Ap2Config::default()` silence trap (`quiet_mode: true`,
  `volume_db: -144`) and call `feedback()` every 2 s itself.
- The inbound firewall rule is a hard prerequisite: without it, the receiver's NTP requests are
  dropped and SETUP fails after about 32 s. The app checks at startup and offers a one-time UAC
  install.
- Upstream changes are reviewed by hand before each `rev` bump, and the listening and latency
  checks are re-run.
- A firmware update that rejects NTP or PCM means implementing the PTP/ALAC fallback, either
  upstream or in a fork.
