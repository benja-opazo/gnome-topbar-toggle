# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
cargo build            # Debug build
cargo build --release  # Release build
cargo run -- <id>      # Run with a unique instance ID (e.g. cargo run -- my-toggle)
cargo fmt              # Format code
cargo clippy           # Lint
cargo test             # Run tests (none currently exist)
```

## Architecture

Everything lives in a single file: `src/main.rs` (~700 lines). The app is a GNOME system tray toggle button that executes a user-configured bash script and shows visual state feedback via an emoji icon with a colored status dot.

**State machine** (`AppState` enum): `Off` → `TurningOn` (script running) → `On` (success, 1.5s then reverts) or `Error` (failure, 2s then reverts). State is shared across threads via `Arc<Mutex<AppContext>>`.

**Persistence** (`PersistentConfig`): Serialized to `~/.config/bash-toggle/<id>.json`. Stores the chosen emoji, script path, and recently used emojis. The unique ID passed at launch (required CLI arg) namespaces multiple instances.

**Tray icon**: Rendered using Cairo — draws the emoji and a small colored status circle (gray/blue/green/red), encodes to PNG, passes to `tray-icon` crate.

**Emoji picker**: A GTK window with categorized emoji browsing, smooth animated scrolling (cubic easing), positioned at the mouse cursor, auto-hides on focus loss.

**Script execution**: Runs in a background thread via `Command::new("bash")`, captures stdout/stderr, sends state transitions back to the GTK main loop via a `glib` channel.

**Desktop notifications**: Sent via `notify-rust` on config changes and script events.

## Runtime requirements

GTK 3.24, GLib, Cairo, libnotify, and Bash must be installed on the system.
