# KbdSplit

[![CI](https://github.com/oliviermugishak/kbdsplit/actions/workflows/ci.yml/badge.svg)](https://github.com/oliviermugishak/kbdsplit/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/kbdsplit-daemon)](https://crates.io/crates/kbdsplit-daemon)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**KbdSplit** turns up to four physical keyboards into four virtual Xbox-style controllers on Linux. Each keyboard maps to its own virtual gamepad with customizable per-key bindings — perfect for local multiplayer on a single PC.

## Architecture

```
┌─────────────────┐    ┌──────────────────────────────────────┐
│   kbdsplit-gui  │◄───│  Unix socket IPC  (/tmp/kbdsplit.sock)│
│  (egui desktop)  │    └──────────────────┬───────────────────┘
└─────────────────┘                       │
                                          ▼
┌──────────────────────────────────────────────────────────────┐
│                        kbdsplitd                             │
│  ┌────────┐  ┌────────┐  ┌────────┐  ┌────────┐             │
│  │Reader 1│  │Reader 2│  │Reader 3│  │Reader 4│  ...        │
│  │(epoll) │  │(epoll) │  │(epoll) │  │(epoll) │             │
│  └────┬───┘  └────┬───┘  └────┬───┘  └────┬───┘             │
│       │           │           │           │                  │
│  ┌────▼───────────▼───────────▼───────────▼────┐             │
│  │        RuntimeSlot × 4                      │             │
│  │  held_keys  →  action_refcount  →  state    │             │
│  └────────────────┬────────────────────────────┘             │
│                   ▼                                          │
│  ┌──────────────────────────────────────────┐                │
│  │       VirtualGamepad × 4 (uinput)        │                │
│  └──────────────────────────────────────────┘                │
└──────────────────────────────────────────────────────────────┘
```

- **kbdsplitd** — privileged daemon that reads `/dev/input/event*`, locks keyboards with `EVIOCGRAB`, and emits virtual gamepads through `/dev/uinput`
- **kbdsplit-gui** — egui desktop frontend for assigning keyboards, editing bindings, and managing profiles

## Quick Start

### Install

```bash
git clone https://github.com/oliviermugishak/kbdsplit
cd kbdsplit
./install.sh
```

The installer places binaries in `/usr/local/bin`, installs udev rules, and adds your user to the `input` group. **Log out and back in** after first install.

### Run

```bash
kbdsplit-gui
```

The GUI starts the daemon automatically when both binaries are installed together. Use the system tray or launcher, or run from a terminal to see logs.

### Development Build

```bash
cargo build --workspace
cargo run -p kbdsplit-daemon --bin kbdsplitd
cargo run -p kbdsplit-gui --bin kbdsplit-gui
```

## Default Mapping

| Keys                | Xbox Control       |
|---------------------|--------------------|
| `W A S D`          | Left stick         |
| Arrow keys          | Right stick        |
| `J` / `K` / `U` / `I` | A / B / X / Y   |
| `Q` / `E`          | LB / RB            |
| `Left Shift` / `Space` | LT / RT        |
| `H` / `L`          | Back / Start       |
| `Esc` / `Enter`    | Guide / Start      |

All bindings are fully customizable through the GUI binding editor.

## GUI Capabilities

- Assign physical keyboards to controller slots 1–4
- Lock/unlock devices (`EVIOCGRAB` captures input exclusively)
- Full binding editor: map any key to any Xbox button, D-pad direction, stick axis, trigger, shoulder, thumbstick click, or menu button
- Live controller diagram with real-time button presses and stick deflection
- Profile management: create, switch between, and delete profiles
- Built-in profiles: **Default** (general purpose) and **eFootball** (curated for football games)
- Hotplug detection with auto-reconnect
- Permission warnings for unprivileged devices
- Event log for debugging
- Test output buttons to verify controller output without a game

## Kill Switch

Press **Left Shift + Right Shift + Esc** simultaneously to release all locks and stop the daemon. Works even without the GUI running.

## Permissions

The daemon requires access to `/dev/input/event*` and `/dev/uinput`. The install script sets up:

- `/etc/udev/rules.d/70-kbdsplit.rules` — grants `input` group `rw` access
- User added to `input` group

Without these, the daemon runs but cannot grab keyboards, and the GUI shows permission warnings.

## Project Structure

```
crates/
├── shared/       — Types, IPC protocol, serialization
├── core/         — Binding mapping, profiles, state reconciliation
├── daemon/       — evdev reader, uinput emitter, IPC server
├── gui/          — egui desktop frontend
└── bench/        — Latency/throughput benchmark harness
```

## Benchmarks

```bash
cargo run -p kbdsplit-bench     # requires sudo for uinput
```

Measures uinput write latency, throughput (events/sec), and jitter. Typical results on modern hardware: P50 ~12 µs, P99 ~25 µs per press+release cycle.

## License

MIT — see [LICENSE](LICENSE).
