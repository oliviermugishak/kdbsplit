# KbdSplit

KbdSplit is a Linux desktop app that turns up to four physical keyboards into four virtual Xbox-style controllers.

The project has two binaries:

- `kbdsplitd`: privileged input daemon. It reads `/dev/input/event*`, locks keyboards with `EVIOCGRAB`, and emits virtual gamepads through `/dev/uinput`.
- `kbdsplit-gui`: egui desktop frontend. It shows keyboards, slots, lock state, permission status, live activity, and the controller visualization.

## Build

```bash
cargo build --workspace
```

## Install and Uninstall

Install:

```bash
./install.sh
```

Uninstall:

```bash
./uninstall.sh
```

The installer places `kbdsplitd` and `kbdsplit-gui` in `/usr/local/bin`, installs the desktop launcher, installs udev rules for `/dev/input` and `/dev/uinput`, and adds the current user to the `input` group unless `--no-input-group` is passed.

After the first install, log out and back in so Linux applies the new group membership.

## Run

For development, run the daemon and GUI from the build output:

```bash
cargo run -p kbdsplit-daemon --bin kbdsplitd
cargo run -p kbdsplit-gui --bin kbdsplit-gui
```

The GUI also tries to start `kbdsplitd` automatically when both binaries are installed in the same directory.

## Permissions

The daemon needs access to `/dev/input/event*` and `/dev/uinput`. `./install.sh` installs the udev rule in `packaging/udev/70-kbdsplit.rules`, adds your user to the `input` group, reloads udev rules, and asks you to log out and back in if needed.

## GUI Capabilities

- Assign physical keyboards to controller slots 1-4
- Lock/unlock devices (grabs the keyboard via `EVIOCGRAB`)
- Full binding editor: map any key to any Xbox button, D-pad direction, stick axis, trigger, shoulder, thumbstick click, or menu button
- Live controller diagram showing real-time button presses and stick deflection
- Profile management: create, switch between, and delete profiles
- Two built-in profiles: **Default** (general-purpose) and **eFootball** (curated for football games)
- Hotplug detection, permission warnings, event log
- Test output buttons to verify controller output without a game

## Default Mapping

| Keys | Xbox Control |
|---|---|
| `WASD` | Left stick |
| Arrow keys | Right stick |
| `J` / `K` / `U` / `I` | A / B / X / Y |
| `Q` / `E` | LB / RB |
| `Left Shift` / `Space` | LT / RT |
| `H` / `L` | Back / Start |
| `Esc` / `Enter` | Guide / Start (duplicate) |

All bindings are fully customizable through the GUI binding editor.
