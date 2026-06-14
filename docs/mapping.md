# Mapping

The mapping model has one physical key bound to one controller action.

Default bindings live in `crates/core/src/mapping.rs`.

The daemon ignores key repeat events and only processes press and release transitions. Held actions are reduced into a full controller state, which is then emitted to the virtual gamepad.

Profiles are TOML files stored in the platform user config directory through `directories::ProjectDirs`.
