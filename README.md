# GBFR Logs for Linux

[![GitHub Release](https://img.shields.io/github/v/release/onelittlechildawa/gbfr-logs)](https://github.com/onelittlechildawa/gbfr-logs/releases)

This fork adds Linux/Wine (Proton, Bazzite, etc.) support to [onelittlechildawa/gbfr-logs](https://github.com/onelittlechildawa/gbfr-logs) ("GBFR Logs Awa Edition"), a community-maintained damage meter fork of [false-spring/gbfr-logs](https://github.com/false-spring/gbfr-logs) for Granblue Fantasy: Relink. MIT license and upstream credits are retained.

## What's here

The upstream Windows GUI is a Tauri app that renders its overlay with the Microsoft Edge WebView2 Runtime. That doesn't work under Wine — WebView2 needs DirectComposition, which Wine's implementation doesn't support, and it segfaults on startup. Rather than patch around that, this fork adds a separate path built for Linux:

- **`protocol/`** — the hook<->parser wire protocol now runs over a loopback TCP socket instead of a Windows named pipe, so a process outside Wine can talk to the in-game hook directly.
- **`engine/`** — the combat-log parsing/aggregation logic, pulled out of the Tauri app into its own crate with no GUI framework dependency, so it can be reused by more than one frontend.
- **`injector/`** — a minimal Windows binary (no Tauri/WebView2 anywhere in it) that just finds the game process and injects `hook.dll`. Runs under Wine, either directly or via `protontricks-launch`.
- **`tui/`** — a native Linux terminal UI: a live DPS overlay that connects to `hook.dll` directly over TCP and renders a damage/DPS table with `ratatui`. No Wine GUI involved on this side at all. It launches `injector.exe` for you on startup.

The original Windows GUI (`src-tauri/` + `src/`) is unchanged and still works normally on native Windows.

## Building

A devcontainer (`.devcontainer/`) cross-compiles the Windows pieces (`hook.dll`, `gbfr-logs.exe`, `injector.exe`) from Linux using mingw-w64, so you don't need to install a Windows Rust toolchain:

```
podman build -f .devcontainer/Dockerfile -t gbfr-logs-devcontainer .
podman run --rm --userns=keep-id -v "$(pwd)":/workspace:Z -w /workspace \
  gbfr-logs-devcontainer bash .devcontainer/build-windows.sh
```

The Linux-native `tui` binary builds normally with `cargo build --release -p tui` (inside or outside the devcontainer).

## Running

1. Copy `gbfr-logs.exe`/`hook.dll`/`WebView2Loader.dll` or `injector.exe`/`hook.dll` (whichever frontend you're using) into one folder.
2. Start the game and get into a session (not just the title screen).
3. Run the TUI, giving it the game's own wine binary and prefix so `injector.exe` attaches to the game's actual live wineserver session:

   ```
   pgrep -f granblue_fantasy_relink.exe
   cat /proc/<pid>/environ | tr '\0' '\n' | grep STEAM_COMPAT

   tui --injector /path/to/injector.exe \
     --wine "<STEAM_COMPAT_TOOL_PATHS entry>/files/bin/wine" \
     --wineprefix "<STEAM_COMPAT_DATA_PATH>/pfx"
   ```

   `--appid <steam appid>` is also accepted as a fallback, using
   `protontricks-launch` instead — simpler, but on at least one Bazzite/CachyOS
   setup this was observed to attach to a separate, unrelated wineserver
   session rather than the game's live one (confirmed via `wine tasklist`
   showing no game process, and a second `wineserver` process appearing).
   If injection silently never finds the game process, switch to
   `--wine`/`--wineprefix`.

## Credits

- [onelittlechildawa](https://github.com/onelittlechildawa) for the Awa Edition this fork is based on.
- [nyaoouo/GBFR-ACT](https://github.com/nyaoouo/GBFR-ACT) for the original reverse engineering work.
- [Harkain](https://github.com/Harkains) for formatting/translating skills to friendly English names.
- [false-spring/gbfr-logs](https://github.com/false-spring/gbfr-logs), the upstream project this is all based on.

## Disclaimer

This tool is meant to improve the experience Cygames has provided, not to cause them or other players harm. It modifies your running game client and is not guaranteed to work after game patches, in which case you may experience instability or crashes.
