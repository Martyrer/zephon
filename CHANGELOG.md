# Changelog

All notable changes to this project are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.0-alpha] - unreleased

First public release.

- Streams Windows system audio to AirPlay 2 receivers (AirPlay 2 realtime, NTP timing, PCM),
  built on the Nyx-Refrain crates.
- Several rooms at once, each with its own volume and latency profile (Low, Balanced, Stable).
- Main window with the room list, loss per room and the lag line; settings window with theme,
  window material, font, text size, accent color, playback defaults, hidden rooms, start with
  Windows and the firewall rule.
- Command line: `devices`, `receivers`, `firewall`, `stream`.
