# Contributing

## Development Setup

**Requirements:**
- Rust 1.96+ (edition 2024)
- Linux with `libudev-dev` (or equivalent)
- `/dev/uinput` access (or `sudo` for testing)

```bash
git clone https://github.com/oliviermugishak/kbdsplit
cd kbdsplit
cargo build --workspace
```

## Running

Start the daemon and GUI in separate terminals:

```bash
# Terminal 1
cargo run -p kbdsplit-daemon --bin kbdsplitd

# Terminal 2
cargo run -p kbdsplit-gui --bin kbdsplit-gui
```

The GUI auto-starts the daemon when both are installed, but during development run them separately.

## Code Style

- Rust edition 2024
- Run `cargo fmt --all` before committing
- Keep clippy clean: `cargo clippy --workspace -- -D warnings`
- No `unwrap()` or `expect()` in production code — use `anyhow` or proper error handling
- Use structured `tracing` for logging, not `eprintln!` or `println!`
- All workspace dependencies shared through `[workspace.dependencies]` in root `Cargo.toml`

## Testing

```bash
cargo test --workspace
```

The test suite covers state reconciliation, binding resolution, profile I/O, and evdev/uinput integration. Key test:

- `opposing_stick_actions_cancel` — verifies opposite directions cancel out in stick state

## Benchmarking

```bash
cargo run -p kbdsplit-bench -- sudo
```

Creates a virtual uinput keyboard and measures write latency, throughput, and jitter. Used to catch regressions in the emit path.

## Pull Requests

1. Ensure all tests pass and clippy is clean
2. Add or update tests for new functionality
3. Update crate descriptions in `Cargo.toml` if public API changes
4. Update `DESIGN.md` or `docs/` if architecture changes
5. Keep commits focused; rebase onto `main` before submitting

## Documentation

- `DESIGN.md` — high-level design and rationale
- `docs/architecture.md` — component breakdown
- `docs/protocol.md` — IPC frame format
- `docs/mapping.md` — key-to-action mapping model

## CI

The project uses GitHub Actions:
- **CI**: build, test, clippy, fmt-check, MSRV check, dependency audit on every push/PR
- **Release**: builds static binaries and publishes to GitHub Releases on tag push
