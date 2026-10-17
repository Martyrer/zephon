# Changelog

All notable changes to this project are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- A room that loses its connection while playing reconnects by itself: up to 3 attempts, after
  2, 4 and 8 s. The row shows "Reconnecting, attempt 1 of 3…"; after the last attempt it says
  what to check.

### Changed

- The lag line at the bottom of the window is gone: its dots looked draggable but weren't, and
  each room already shows its lag. When rooms on different profiles play out of step, the
  footer now says by how much and which profile lines them up.
- Each room picks its latency profile on a 3-stop slider under its volume, in place of the
  drop-down. Drag the thumb or click a profile's name.

### Fixed

- Play is disabled while no room is on; its tooltip says to turn one on. Switching off the
  last room while playing now stops playback instead of leaving Stop showing with nothing on.

- When no room is left playing because they all failed, playback stops, so the footer shows
  Play instead of Stop. The footer no longer says "Connecting…" when nothing is connecting.
- A slow answer from a receiver on weak Wi-Fi no longer drops the room: it now gets 10 s, not
  3 s. Stop no longer waits for an answer either.
- A room whose start took over about a second no longer plays with its clock correction stuck
  at the limit for many minutes.

## [0.1.0-alpha] - unreleased

First public release.

- Streams Windows system audio to AirPlay 2 receivers (AirPlay 2 realtime, NTP timing, PCM),
  built on the Nyx-Refrain crates.
- Several rooms at once, each with its own volume and latency profile (Low, Balanced, Stable).
- Main window with the room list, loss per room and the lag line; settings window with theme,
  window material, font, text size, accent color, playback defaults, hidden rooms, start with
  Windows and the firewall rule.
- Command line: `devices`, `receivers`, `firewall`, `stream`.
