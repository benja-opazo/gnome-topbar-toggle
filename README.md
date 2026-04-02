# gnome-topbar-toggle

A GNOME system tray toggle button that runs a user-configured bash script and shows visual state feedback via an emoji icon with a colored status dot. Tested on GNOME.

## Prerequisites

- Rust (stable toolchain)
- GTK 3.24+
- GLib
- Cairo
- libnotify
- Noto Color Emoji font (for emoji rendering)
- Bash

On Debian/Ubuntu:

```bash
sudo apt install libgtk-3-dev libglib2.0-dev libcairo2-dev libnotify-dev fonts-noto-color-emoji
```

## Installation

```bash
git clone <repo-url>
cd gnome-topbar-toggle
cargo build --release
sudo cp target/release/gnome-topbar-toggle /usr/local/bin/
```

## Usage

```bash
gnome-topbar-toggle <unique-id>
```

The `<unique-id>` namespaces the instance, allowing multiple toggles to run simultaneously with independent configurations. Configuration is persisted to `~/.config/bash-toggle/<id>.json`.

On first launch, right-click the tray icon to:
- **Add/Change Script** — select the bash script to execute on toggle
- **Browse All Emojis...** — pick an icon for the tray button
- **State: Off** — click to run the script

The status dot on the icon reflects the current state:
- Gray: idle
- Blue: script is running
- Green: finished successfully (reverts after 1.5s)
- Red: script failed (reverts after 2s)

## systemd user service

Create `~/.config/systemd/user/topbar-toggle@.service`:

```ini
[Unit]
Description=GNOME Topbar Toggle (%i)
After=graphical-session.target
PartOf=graphical-session.target

[Service]
Type=simple
ExecStart=/usr/local/bin/gnome-topbar-toggle %i
Restart=on-failure
RestartSec=3
Environment=DISPLAY=:0
Environment=DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/%U/bus

[Install]
WantedBy=graphical-session.target
```

Enable and start an instance (replace `my-toggle` with your chosen ID):

```bash
systemctl --user daemon-reload
systemctl --user enable --now topbar-toggle@my-toggle.service
```

To run multiple instances:

```bash
systemctl --user enable --now topbar-toggle@vpn.service
systemctl --user enable --now topbar-toggle@sync.service
```

Each instance has its own configuration and tray icon.
