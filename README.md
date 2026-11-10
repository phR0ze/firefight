# firefight

Simple Flutter/Rust app for firing off arbitrary commands on a timer across a fleet of Linux machines.

An administrator configures reusable timers in a control deck, drags them onto client machines and starts
them. Every assigned client shows the live countdown, and when it finishes the client runs the timer's
actions (play a sound, run a command such as `loginctl lock-session` or `systemctl poweroff`). The server
never runs actions itself.

### Quick links
- [Overview](#overview)
  - [Components](#components)
  - [Getting started](#getting-started)
  - [Android control deck](#android-control-deck)
  - [Pre-configured deployment](#pre-configured-deployment)
  - [Running as services](#running-as-services)
  - [Security model](#security-model)
- [Development](#development)

## Overview

### Components

| Component                        | Where it runs        | What it does                                                                      |
|----------------------------------|----------------------|--------------------------------------------------------------
| `firefight-server` (Rust)        | one machine          | Stores timers and clients, drives timer state, pairs agents                       |
| `firefight-agent` (Rust)         | every client machine | Mirrors its assigned timers, runs actions when they finish                        |
| `app` (Flutter, `--mode=server`) | admin machine        | Control deck: create/edit/start timers, approve clients, drag-and-drop assignment |
| `app` (Flutter, Android)         | admin's phone        | The same control deck, laid out for a phone                                       |
| `app` (Flutter, `--mode=client`) | client machines      | Display of the timers assigned to this machine, with Dismiss                      |

```
 Flutter control deck ──REST + WebSocket──▶ firefight-server ◀──WebSocket── firefight-agent ◀── Flutter client display
                                                                         (runs the actions)      (127.0.0.1 only)
```

Agents schedule the finish locally from the server's deadline, so a short network outage right before a
timer ends does not stop the actions from running. Each run fires at most once per agent.

v1 supports **Countdown** and **At a time** (once, daily, weekdays or specific days) timers, and **Sound**
and **Command** actions. Actions can start in parallel or wait for the previous one to finish.
Bundled sounds are `gong`, `chime` and `bell`. Any absolute path to a wav/ogg/flac/mp3 file on the client
also works.

### Getting started

Everything builds inside the dev shell:

```bash
nix develop
cargo build --release
(cd app && flutter build linux --release)
```

1. **Start the server** on the machine that coordinates the fleet:
   ```bash
   ./target/release/firefight-server            # listens on 0.0.0.0:7878
   ```
   On first start it creates an admin token in `~/.local/share/firefight/admin_token`.

2. **Open the control deck** on the same machine. It picks up the admin token automatically:
   ```bash
   app/build/linux/x64/release/bundle/firefight --mode=server --server=localhost:7878
   ```
   From another machine, pass `--token=<admin token>` or paste the token into the settings screen.

3. **Start an agent** on each client machine:
   ```bash
   ./target/release/firefight-agent --server <server-host>:7878 --name "Front desk"
   ```
   The client appears under **Awaiting approval** in the control deck. Approve it, and the agent receives
   and stores its token in `~/.config/firefight/agent.toml`.

4. **Optionally open the client display** on the client machine:
   ```bash
   app/build/linux/x64/release/bundle/firefight --mode=client
   ```

5. **Create a timer** with **+ New timer**, drag its card onto a client, and press **Start** (or **Arm**
   for scheduled timers).

Settings passed on the command line are remembered in `~/.config/firefight/app.json`.

The client display lives in the system tray: closing its window hides it (the tray menu's **Show** brings
it back, **Quit** exits), and it comes to the front by itself when one of its timers starts firing. Start it
with `--start-hidden` to go straight to the tray, or `--no-tray` to make closing the window quit. The tray
needs a panel that hosts StatusNotifierItems (most do; GNOME needs the AppIndicator extension).

### Android control deck

The Android app is the same control deck as the Linux one. It talks to the same server, so timers, clients
and assignments stay in sync across every deck and client display. The phone only stores the server
address and admin token.

```bash
nix develop
cd app && flutter build apk --release      # signed with the debug key, for sideloading
adb install build/app/outputs/flutter-apk/app-release.apk
```

On first launch enter the server's LAN address (`<server-host>:7878`) and the admin token from the
server's data directory. On a phone, timers and clients are on separate tabs: assign timers with
**Assign timers…** in a client's ⋮ menu (or long-press and drag a timer card where both are visible).
Like the Linux deck, it uses plain HTTP, so use it only on a trusted LAN.

### Pre-configured deployment

For fleets set up by configuration management (a NixOS module lives in
[nixos-config](https://github.com/phR0ze/nixos-config) under `modules/apps/system/firefight`), every
piece can start already connected and keyed:

| Flag                                   | Effect                                                                         |
|----------------------------------------|--------------------------------------------------------------------------------|
| `firefight-server --admin-token-file`  | Use this admin token instead of generating one in the data directory           |
| `firefight-server --enroll-key-file`   | Agents presenting this shared key are approved automatically                   |
| `firefight-server --config`            | Declarative timers (below)                                                     |
| `firefight-agent --enroll-key-file`    | Present the shared key, so no manual approval is needed                        |
| `firefight --token-file=<path>`        | Control deck reads the admin token from a file instead of the command line     |

The `--config` file declares timers by a stable `key` (the timer id is derived from it):

```json
{
  "timers": [{
    "key": "lab-closes",
    "name": "Lab closes",
    "kind": { "type": "countdown", "duration_secs": 900 },
    "actions": [{ "kind": { "type": "command", "cmd": "loginctl lock-session" }, "mode": "after_previous" }],
    "allow_client_dismiss": false
  }]
}
```

Declared timers are managed: every server start rewrites them (edits made in the deck don't survive a
restart), and a timer removed from the file is deleted. Which clients get them is up to the control deck:
the config never assigns or unassigns anything, and timers created in the deck are left alone.
`kind` and `actions` use the same JSON as the admin API (`at_time` timers take `"time": "HH:MM"` and a
`repeat`).

### Running as services

Example systemd user units are in [`contrib/systemd`](contrib/systemd). The agent unit runs inside the
graphical session so sounds and session commands work. `loginctl lock-session` and, on most
distributions, `systemctl poweroff` are allowed for the active local user by polkit. Anything needing more
privileges must be granted explicitly (polkit rule or sudoers entry). The agent never escalates by itself.

### Security model

* The server can make approved clients run **arbitrary shell commands as the agent's user**. Only approve
  machines you recognise, and protect the admin token like a password.
* New agents start as *pending* and receive nothing until approved. On approval the server issues a
  per-client token; only its SHA-256 hash is stored on the server. A client removed from the deck loses
  its token immediately.
* With `--enroll-key-file`, anyone holding the enrollment key can add an approved client, so treat it
  like the admin token. Removing a client in the deck still revokes it; it can only come back by
  enrolling again with the key.
* The admin API requires the admin token (`Authorization: Bearer`).
* v1 traffic is plain HTTP/WebSocket (no TLS) and is intended for a trusted LAN only.
* The agent's display socket only listens on `127.0.0.1`.

## Development

```bash
nix develop
cargo test                          # core logic, server hub, agent, and a server+agent end-to-end test
cargo clippy --all-targets
(cd app && flutter analyze && flutter test)
```

Layout:

```
crates/core     shared models, wire protocol, schedule math, run state machine
crates/server   firefight-server (axum)
crates/agent    firefight-agent (tokio, rodio)
app/            Flutter app (control deck + client display; Linux and Android)
contrib/        systemd user units
docs/design     UI mockups
```

### Run locally

1. Server (throwaway data dir)
   ```bash
   cargo run -p firefight-server -- --data-dir /tmp/ff-test
   ```

2. Control deck (debug build with hot reload; press `r` in this terminal to reload after edits)
   ```bash
   cd app && flutter run -d linux -a --mode=server -a --server=localhost:7878 -a --token=$(cat /tmp/ff-test/admin_token)
   ```

3. Agent (throwaway config)
   ```bash
   cargo run -p firefight-agent -- --server localhost:7878 --name "Test box" --config /tmp/ff-test/agent.toml
   ```

4. Optional: the client display
   ```bash
   cd app && flutter run -d linux -a --mode=client
   ```

5. Optional: the Android control deck in an emulator (KVM accelerated; the device persists in
   `~/.local/share/firefight-emulator`)
   ```bash
   nix run .#emulator                # boots the emulator, returns once it is ready
   cd app && flutter run             # installs and starts the app on it, with hot reload
   ```
   Inside the emulator the host is `10.0.2.2`, so connect the app to `10.0.2.2:7878`.
