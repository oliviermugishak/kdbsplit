
# Design doc: Linux Keyboard-to-Xbox Splitter in Rust

## 1) Product goal

Build a Linux-only desktop app that can take up to 4 physical keyboards, including the built-in laptop keyboard, and expose each one as a separate virtual Xbox-style controller that games detect as real gamepads. The app must have a friendly GUI, real-time feedback, device locking, per-keyboard assignment to controller slots 1–4, profile saving, and a live controller diagram showing which physical keys map to which gamepad buttons.

The key technical idea is to build a **userspace input broker**: read physical input from Linux `evdev`, transform events, then emit virtual controllers through `uinput`. Linux’s input subsystem explicitly treats `evdev` as the preferred userspace input interface and `uinput` as the userspace mechanism for creating virtual input devices. ([Linux Kernel Documentation][1])

---

## 2) Core design principles

1. **Games must see controllers, not remapped keyboard events.**
   The output must be virtual gamepads with the Linux gamepad event layout, not window-level keyboard automation. Linux defines standard gamepad reporting semantics for buttons and axes so user space does not need per-device mappings. ([Linux Kernel Documentation][2])

2. **The GUI is only the control plane.**
   The GUI must configure the daemon, but all event capture and output should live in a separate backend so input latency stays low and the app remains robust if the UI crashes.

3. **Per-device locking must be real exclusivity.**
   When a keyboard is assigned and locked, the backend should grab it so no other process receives the same events. Linux input handles support device grabbing via `EVIOCGRAB`, which makes the grabbing handle the sole recipient of input events from that device. ([Linux Kernel Archives][3])

4. **Virtual controller output must be standard.**
   The output device should follow Linux gamepad conventions so games, SDL, Steam Input, and anti-cheat layers have the best chance of recognizing it cleanly. ([Linux Kernel Documentation][2])

---

## 3) Recommended technology stack

### Backend

Use Rust with:

* `evdev` for reading physical keyboards
* `uinput` or `input-linux` for virtual controller emission
* `serde` + `toml`/`json` for profiles
* `tracing` for logs
* `tokio` only if you truly need async I/O orchestration; otherwise a thread-per-device or reactor-style loop is often simpler for input devices

Linux input docs and the Rust crates both support this architecture directly: `evdev` exposes `/dev/input/eventX`, while `uinput` creates virtual devices from userspace. `input-linux` is an umbrella crate that exposes both `evdev` and `uinput` modules if you prefer a single dependency surface. ([Docs.rs][4])

### GUI

My recommendation is **egui** first.

`egui` is an immediate-mode GUI library designed for highly interactive applications and is fully native. That is a strong fit for a real-time input mapper with live highlighting and frequent state updates. `dioxus_desktop` is also viable, but it is webview-based and requires a webview runtime to be installed on the system. ([Docs.rs][5])

### Suggested UI choice

* **Primary choice:** `egui`
* **Secondary choice:** `Dioxus` only if you want web-style composition, richer HTML/CSS rendering, or future web reuse

For this project, `egui` is the safer v1 choice because the app is input-heavy, highly interactive, and benefits from a fast redraw loop. ([Docs.rs][5])

---

## 4) System architecture

## 4.1 Components

### `kbdsplitd` — privileged daemon

Responsibilities:

* discover keyboards
* read evdev events
* detect hotplug / unplug
* assign devices to controller slots
* grab or release devices
* maintain slot state
* emit virtual Xbox controllers through uinput
* persist mappings and profiles
* expose a local control API for the GUI

### `kbdsplit-gui` — desktop frontend

Responsibilities:

* show all detected keyboards
* show slot assignment state
* let the user lock/unlock devices
* let the user edit mappings
* render live Xbox controller diagrams
* show event activity in real time
* launch the daemon or connect to it

### Optional `kbdsplit-agent`

A small helper that can run privileged operations if you decide to keep the GUI unprivileged.

This split is justified by Linux permissions realities: opening `/dev/input/eventX` can require elevated privileges, while `uinput` also interacts with privileged device nodes. The `evdev` crate documentation explicitly notes that opening the device node often requires appropriate privileges, and that a privileged process can pass in an already-open file descriptor. ([Docs.rs][4])

---

## 5) Event pipeline

## 5.1 Input flow

1. Enumerate `/dev/input/event*`.
2. Identify keyboards by supported keys and device metadata.
3. Start one reader loop per keyboard.
4. Convert raw evdev key events into internal actions.
5. Route actions to a controller slot.
6. Update the slot state.
7. Emit the corresponding uinput gamepad event.

Linux evdev gives you structured events with type, code, and value, and the kernel event codes are architecture-independent. That means the same mapping logic will work across Linux machines without per-platform rewrites. ([Linux Kernel Documentation][6])

## 5.2 Output flow

Each assigned slot gets one virtual controller with a stable device identity and a standard gamepad layout. The virtual device should expose:

* D-pad
* face buttons
* menu/start/select
* shoulders
* triggers
* left stick
* right stick
* optional guide button

Linux’s gamepad spec defines the canonical event model, and `uinput` creates a virtual device with chosen capabilities that are delivered to userspace and in-kernel consumers. ([Linux Kernel Documentation][2])

---

## 6) Mapping model

The app should support three layers of mapping:

### Layer A: Physical key → logical action

Example: `W` → `LeftStickUp`, `J` → `A`, `K` → `B`.

### Layer B: Logical action → virtual controller event

Example: `A` → `BTN_SOUTH`, `LeftStickUp` → `ABS_Y = -32767`.

### Layer C: Profile → slot behavior

Example: “FPS profile,” “Fighting game profile,” “eFootball profile,” or custom per-game configs.

This three-layer model keeps the system flexible. It lets you change the look of the controller output without rewriting keyboard bindings, and it lets you reuse one profile across different controller slots.

---

## 7) Slot and device management

The app should treat each controller slot as an independent state machine:

* `Empty`
* `Bound`
* `Locked`
* `Active`
* `Paused`
* `Error`

A physical keyboard should also have its own state:

* `Available`
* `Assigned`
* `Grabbing`
* `Grabbed`
* `Released`
* `Disconnected`

### Assignment rules

* One keyboard can map to one slot at a time.
* One slot can only have one primary keyboard.
* Optional advanced mode: allow multiple keyboards to merge into one slot.
* When locked, the device is grabbed so the system stops forwarding those events elsewhere. Linux input handles support grab semantics, and `EVIOCGRAB` makes the grabbing handle the sole recipient. ([Linux Kernel Archives][3])

### Built-in keyboard handling

The built-in laptop keyboard should be treated like any other evdev keyboard but labeled clearly as “internal.” That matters because the user may want to reserve it for Player 1 and external USB keyboards for other players.

---

## 8) GUI design

## 8.1 Main screen layout

### Left panel

* device list
* slot picker 1–4
* lock/unlock toggle
* profile selection
* mapping editor
* save/import/export buttons

### Center or right panel

* large Xbox controller image
* highlighted buttons and stick zones
* live state transitions
* analog values where relevant

### Bottom strip

* event log
* hotplug notices
* permission status
* active profile and active slot summary

`egui` is especially suitable for this because it is designed for highly interactive applications and redraws the whole interface at display refresh rate. `Dioxus Desktop` is webview-based instead, so it is better when you want a browser-like component model rather than a low-latency tool panel. ([Docs.rs][5])

## 8.2 Controller visualization

The controller image should not be decorative only. It should reflect state:

* press a mapped key → highlight the target controller button
* hold a key → keep highlight active
* analog-style mapping → show stick deflection
* trigger mapping → show trigger depth
* conflict → flash a warning color

This gives the user immediate confirmation that the split is working.

---

## 9) Permissions and deployment model

There are two realistic deployment options:

### Option 1: privileged daemon

Run `kbdsplitd` with the minimum privileges required to read input devices and create virtual controllers.

### Option 2: privileged helper + normal GUI

Run a small service that handles device access, while the GUI remains normal user-space.

The second option is usually more user-friendly. The `evdev` docs note that a privileged process can open a device and pass the file descriptor to another process, which is a clean way to avoid running the whole GUI as root. ([Docs.rs][4])

For packaging, the safest path is:

* systemd user service if permissions allow
* otherwise a system service plus DBus or a local socket API
* udev rules only if needed to grant access to `/dev/input` and `/dev/uinput`

---

## 10) API design

Use a local IPC API between GUI and daemon.

### Suggested transport

* Unix domain socket
* or DBus if you want better desktop integration
* or gRPC only if you are intentionally building a larger platform

### Example commands

* `list_devices`
* `list_slots`
* `assign_device_to_slot`
* `lock_device`
* `unlock_device`
* `set_binding`
* `load_profile`
* `save_profile`
* `calibrate`
* `preview_state`

### Example event stream

* `DeviceAdded`
* `DeviceRemoved`
* `DeviceAssigned`
* `DeviceLocked`
* `DeviceUnlocked`
* `SlotUpdated`
* `MappingChanged`
* `ProfileLoaded`
* `ErrorRaised`

This evented design keeps the GUI reactive and lets you show real-time feedback without polling.

---

## 11) Data model

## 11.1 Device identity

Store a fingerprint such as:

* bus type
* vendor ID
* product ID
* name
* phys path
* uniq, if present

That gives you robust persistence even when `/dev/input/eventX` numbers change.

## 11.2 Profiles

A profile should store:

* profile name
* target slot
* device fingerprint
* lock mode
* key map
* stick behavior
* trigger behavior
* deadzones
* repeat behavior
* turbo settings, if any
* passthrough policy

## 11.3 Binding format

Example conceptual structure:

```text
slot_1:
  device_fingerprint: ...
  locked: true
  bindings:
    KEY_J: BTN_SOUTH
    KEY_K: BTN_EAST
    KEY_U: BTN_WEST
    KEY_I: BTN_NORTH
    KEY_W: ABS_Y_NEG
```

---

## 12) Edge cases you must handle

1. **Hotplug while locked**
   Reconnect the same device if possible and restore assignment.

2. **Duplicate keyboards from the same manufacturer**
   Do not rely on name alone; include full fingerprint data.

3. **Key repeat**
   Distinguish press, release, and repeat behavior. Games usually need stable press/release semantics.

4. **Ghosting / rollover limitations**
   Some cheap keyboards cannot register certain key combinations, so the mapper should show missing events clearly.

5. **Wayland vs X11**
   Since the output is kernel-level virtual input, the desktop compositor should not matter for the game path itself, but your GUI may behave differently across environments.

6. **Permissions failure**
   Show a clean, actionable error when `/dev/input` or `/dev/uinput` cannot be opened.

7. **Game compatibility**
   Some games may prefer a specific controller flavor. That is why standard gamepad event layouts matter. Linux’s gamepad spec exists to normalize how gamepads report data. ([Linux Kernel Documentation][2])

---

## 13) Testing strategy

### Unit tests

* mapping translation
* profile serialization
* slot allocator
* hotplug reconciliation
* conflict detection

### Integration tests

* read real evdev keyboard events
* verify virtual controller nodes appear
* verify event emission matches expected codes
* verify lock/unlock behavior

### Manual tests

* Steam Big Picture
* SDL-based games
* native Linux games
* Proton games
* games with anti-cheat disabled in offline test mode

### Debug tools

* event log viewer
* live controller viewer
* “inject test event” mode
* “record and replay” keyboard sessions

---

## 14) Rust crate choices

### Best backend baseline

* `evdev`
* `uinput`
* or `input-linux` if you want one umbrella crate for both interfaces. ([Docs.rs][7])

### Best GUI baseline

* `egui` for the first production version because it is a native, immediate-mode GUI library designed for interactive apps. ([Docs.rs][5])

### Secondary GUI option

* `Dioxus Desktop` if you want a webview-based renderer and a more web-like UI architecture, understanding that a webview must be installed on the target Linux system. ([Docs.rs][8])

### Supporting crates

* `serde`
* `tracing`
* `anyhow` or a custom error enum
* `thiserror`
* `directories`
* `notify` or a DBus/udev watcher
* `parking_lot` for lightweight locking
* `crossbeam-channel` or `tokio::sync` channels

---

## 15) Suggested repository structure

```text
keyboard-splitter/
  Cargo.toml
  crates/
    core/
      src/
        device/
        mapping/
        profile/
        slot/
        state/
        error.rs
    daemon/
      src/
        main.rs
        ipc.rs
        runtime.rs
    gui/
      src/
        main.rs
        app.rs
        widgets/
        screens/
        controller_view.rs
    shared/
      src/
        protocol.rs
        types.rs
        config.rs
  assets/
    xbox_controller.svg
    icons/
  profiles/
  packaging/
    systemd/
    udev/
    desktop/
  docs/
    architecture.md
    protocol.md
    mapping.md
```

---

## 16) MVP roadmap

### MVP 1

* detect keyboards
* display them in GUI
* assign one keyboard to one slot
* create one virtual controller
* basic key-to-button mapping
* save/load profile

### MVP 2

* 4-slot support
* locking/grabbing
* live controller diagram
* hotplug support
* per-slot configuration

### MVP 3

* advanced mapping editor
* analog emulation from keyboard clusters
* profile presets
* import/export
* startup autoload

### MVP 4

* polished UX
* better error recovery
* accessibility improvements
* controller test mode
* game-specific presets

---

## 17) Biggest risks

1. **Permissions and installation complexity**
   This is the most common real-world failure point.

2. **Bad key scanning assumptions**
   Some keyboards do not behave well under certain combinations.

3. **Game compatibility variance**
   Not every game treats virtual pads the same way, even if the kernel device is correct.

4. **Overly ambitious scope**
   The GUI can become too fancy too early. The backend must work first.

5. **Device identity confusion**
   If fingerprints are weak, the wrong keyboard may be reattached after reboot.

---

## 18) Final recommendation

Build this as a **Rust daemon plus Rust GUI**, using `evdev` for capture, `uinput` for virtual controllers, and **egui** for the first UI. Make the Linux input backend rock solid before polishing the controller image and mapping editor. That gives you the highest chance of ending with a tool that feels like a real desktop utility rather than a fragile key remapper. ([Linux Kernel Documentation][1])


[1]: https://docs.kernel.org/input/input.html?utm_source=chatgpt.com "1. Introduction - The Linux Kernel documentation"
[2]: https://docs.kernel.org/input/gamepad.html?utm_source=chatgpt.com "4. Linux Gamepad Specification"
[3]: https://www.kernel.org/doc/html/v5.6/driver-api/input.html?utm_source=chatgpt.com "Input Subsystem — The Linux Kernel documentation"
[4]: https://docs.rs/evdev/latest/i686-pc-windows-msvc/evdev/?utm_source=chatgpt.com "evdev - Rust"
[5]: https://docs.rs/egui/latest/egui/?utm_source=chatgpt.com "egui - Rust"
[6]: https://docs.kernel.org/input/event-codes.html?utm_source=chatgpt.com "2. Input event codes - The Linux Kernel documentation"
[7]: https://docs.rs/input-linux?utm_source=chatgpt.com "input_linux - Rust - Docs.rs"
[8]: https://docs.rs/crate/dioxus-desktop/latest?utm_source=chatgpt.com "dioxus-desktop 0.7.9"
