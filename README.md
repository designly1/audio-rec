# Bluetooth Receiver

A small Rust desktop app for Windows that lets a paired phone play audio through
your PC's default speakers or headphones. Uses the Windows Bluetooth A2DP sink
through `Windows.Media.Audio.AudioPlaybackConnection`, with an egui interface.

## Use

1. Run `dist\audio-rec.exe` (or `target\release\audio-rec.exe`).
2. Pair your phone with the PC in Windows Bluetooth settings.
3. Click **Refresh**, select the phone, and click **Connect**.
4. Play audio on the phone. Keep the receiver app open while listening.
5. Click **Disconnect** or close the window to release the receiver.

Requires Windows 10 version 2004 or newer, a working Bluetooth adapter and driver,
and a remote device supporting Bluetooth audio streaming. Bluetooth LE peripherals
such as mice are not audio sources. Pairing is handled by Windows. If a phone does
not appear, check that Bluetooth is enabled and pair/reconnect it before refreshing.
Choose your output and adjust volume in Windows or on the phone. This app does not
record audio or send it over the internet.

## Development

The local environment uses stable Rust (`rustup`, Cargo, rustfmt, Clippy), targeting
`x86_64-pc-windows-gnu`, and the UCRT LLVM-MinGW compiler. Rust is installed under
`%USERPROFILE%\.cargo`; LLVM-MinGW is under
`%LOCALAPPDATA%\audio-rec\toolchains`. Both are on the user PATH; restart an existing
terminal to pick up PATH changes.

```powershell
.\scripts\build.ps1
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo run
```

`scripts\build.ps1` refreshes its own PATH, so it also works in existing terminals.
The repository config selects LLVM-MinGW for the GNU target. On another machine,
install Rust with `rustup-init --default-host x86_64-pc-windows-gnu`, install
[LLVM-MinGW](https://github.com/mstorsjo/llvm-mingw/releases), and add its `bin`
directory to PATH. Alternatively install the Microsoft C++ build tools and use
`cargo build --release --target x86_64-pc-windows-msvc` with that Rust target installed.

To verify API availability and enumerate eligible audio sources without opening a
window or connecting to a device:

```powershell
.\target\release\audio-rec.exe --diagnose diagnostics.txt
Get-Content .\diagnostics.txt
```

Discovery and connection work run on a dedicated WinRT worker so the window stays
responsive. The worker retains the receiver connection, checks its actual Windows
state, and closes it on disconnect or shutdown. It never reads the connection's
`DeviceId` property; discovery provides the device ID instead.

Windows API reference:
[Enable audio playback from remote Bluetooth devices](https://learn.microsoft.com/en-us/windows/uwp/audio-video-camera/enable-remote-audio-playback).
