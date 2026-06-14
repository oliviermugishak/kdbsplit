# Architecture

KbdSplit uses the design in `DESIGN.md`: a privileged daemon owns Linux input/output, and the GUI is a normal desktop control plane.

## Daemon

`kbdsplitd`:

- enumerates `/dev/input/event*`
- identifies keyboard-like devices from `EV_KEY` capabilities
- fingerprints devices with bus/vendor/product/name/phys/uniq
- reads assigned keyboards on one thread per device
- grabs locked keyboards with `EVIOCGRAB`
- creates virtual Xbox-like controllers with `/dev/uinput`
- persists assignment and lock state in the user config directory
- exposes a small Unix socket API at `/tmp/kbdsplit.sock`

## GUI

`kbdsplit-gui`:

- polls daemon state at interactive frequency
- shows all keyboards and permission warnings
- lets users assign keyboards to slots 1-4
- locks/unlocks assigned keyboards
- renders controller state from the same snapshot the daemon emits

The first version uses request/response IPC instead of a streaming event bus. That keeps reconnection and daemon startup simple while still giving responsive live state.
