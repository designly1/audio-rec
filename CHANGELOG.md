# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/).

## [0.2.0] - 2026-11-18

### Added
- **Connection quality panel**, shown while connected:
  - a Good / Fair / Poor rating based on audio dropouts in the last minute
  - an output level meter
  - connection uptime
  - dropout counts for the last minute and the whole session, plus how long ago the last dropout happened
  - link drop and reconnect counts
- Dropout detection using a WASAPI loopback capture of the default output:
  - A silence of 20 ms to 1 s in the middle of playing audio counts as a dropout.
  - A longer silence counts as a pause.
  - Windows does not expose A2DP packet loss or signal strength, so this is an estimate.
  - Nothing is recorded or stored.
- The app version now appears next to the window heading.
- Unit tests for dropout detection and the rating thresholds, plus an ignored smoke test that opens a real loopback stream.

### Changed
- The default window is taller, at 540×660 with a minimum of 480×600, to fit the new panel.
- The README now has a "Connection quality" section.

## [0.1.0] - 2026-10-19

### Added
- First release:
  - a Bluetooth A2DP receiver for Windows built on `AudioPlaybackConnection`, with an egui interface
  - discovery and connection of paired devices
  - a `--diagnose` mode

[0.2.0]: https://github.com/designly1/audio-rec/compare/9ced078...v0.2.0
[0.1.0]: https://github.com/designly1/audio-rec/commit/9ced078
