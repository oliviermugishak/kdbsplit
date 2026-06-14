use crate::evdev::{EV_KEY, EV_SYN, SYN_DROPPED, InputReader, discover_keyboards};
use crate::ipc::{read_command, write_message};
use crate::uinput::VirtualGamepad;
use crate::SHUTDOWN;
use anyhow::{Context, Result};
use kbdsplit_core::{AppConfig, ProfileStore, RuntimeSlot};
use kbdsplit_shared::{
    AppSnapshot, CaptureStatus, ClientCommand, ControllerAction, ControllerSlot, DeviceId, EventLogEntry, KeyboardDevice,
    KeyBinding, KeyCode, LogLevel, ServerMessage, SlotLifecycle,
};
use parking_lot::Mutex;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};
use tracing::{error, info, warn};

pub struct Daemon {
    socket_path: PathBuf,
    state: Arc<Mutex<DaemonState>>,
    active_readers: Arc<Mutex<BTreeSet<DeviceId>>>,
}

struct DaemonState {
    devices: BTreeMap<DeviceId, KeyboardDevice>,
    slots: BTreeMap<ControllerSlot, RuntimeSlot>,
    outputs: BTreeMap<ControllerSlot, VirtualGamepad>,
    config: AppConfig,
    store: ProfileStore,
    permission_warnings: Vec<String>,
    event_log: Vec<EventLogEntry>,
    started_at: Instant,
    capture_in_progress: Option<(ControllerSlot, kbdsplit_shared::ControllerAction, Instant)>,
    capture_timeout: Duration,
}

impl Daemon {
    pub fn start(socket_path: PathBuf) -> Result<Self> {
        let store = ProfileStore::new()?;
        let config = store.load().unwrap_or_else(|err| {
            warn!("failed to load profile: {err:#}");
            AppConfig::default()
        });
        let mut state = DaemonState {
            devices: BTreeMap::new(),
            slots: ControllerSlot::ALL
                .into_iter()
                .map(|slot| (slot, RuntimeSlot::new(slot)))
                .collect(),
            outputs: BTreeMap::new(),
            config,
            store,
            permission_warnings: Vec::new(),
            event_log: Vec::new(),
            started_at: Instant::now(),
            capture_in_progress: None,
            capture_timeout: Duration::from_secs(30),
        };
        state.rescan_devices();
        state.restore_profile_assignments();

        let state = Arc::new(Mutex::new(state));
        let active_readers = Arc::new(Mutex::new(BTreeSet::new()));
        spawn_hotplug_watcher(state.clone(), active_readers.clone());
        for device_id in assigned_devices(&state) {
            active_readers.lock().insert(device_id.clone());
            let state_for_reader = state.clone();
            let readers_for_reader = active_readers.clone();
            thread::spawn(move || {
                let id = device_id.clone();
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    reader_loop(state_for_reader, id.clone());
                }));
                readers_for_reader.lock().remove(&device_id);
                if result.is_err() {
                    tracing::error!("reader_loop panicked for {id}");
                }
            });
        }

        Ok(Self {
            socket_path,
            state,
            active_readers,
        })
    }

    pub fn run(self) -> Result<()> {
        if self.socket_path.exists() {
            fs::remove_file(&self.socket_path)
                .with_context(|| format!("failed to remove {}", self.socket_path.display()))?;
        }
        let listener = UnixListener::bind(&self.socket_path)
            .with_context(|| format!("failed to bind {}", self.socket_path.display()))?;
        listener.set_nonblocking(true)?;
        info!("listening on {}", self.socket_path.display());

        loop {
            if SHUTDOWN.load(Ordering::Acquire) {
                info!("shutdown signal received, exiting");
                break;
            }
            let mut stream = match listener.accept() {
                Ok((stream, _)) => {
                    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
                    stream
                }
                Err(err) if err.kind() == ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(100));
                    continue;
                }
                Err(err) => {
                    error!("IPC accept failed: {err}");
                    thread::sleep(Duration::from_millis(100));
                    continue;
                }
            };
            let command = match read_command(&mut stream) {
                Ok(command) => command,
                Err(err) => {
                    let _ = write_message(&mut stream, &ServerMessage::Error(format!("{err:#}")));
                    continue;
                }
            };
            let should_shutdown = matches!(command, ClientCommand::Shutdown);
            let response = match self.handle_command(command) {
                Ok(message) => message,
                Err(err) => ServerMessage::Error(format!("{err:#}")),
            };
            let _ = write_message(&mut stream, &response);
            if should_shutdown {
                break;
            }
        }

        // Graceful cleanup: destroy virtual gamepads, remove socket, save config
        {
            let mut state = self.state.lock();
            state.outputs.clear();
            let _ = state.save_config();
        }
        let _ = fs::remove_file(&self.socket_path);
        info!("daemon shutdown complete");
        Ok(())
    }

    fn handle_command(&self, command: ClientCommand) -> Result<ServerMessage> {
        match command {
            ClientCommand::GetSnapshot => Ok(ServerMessage::Snapshot(self.state.lock().snapshot())),
            ClientCommand::AssignDevice { device_id, slot } => {
                self.assign_device(device_id, slot)?;
                Ok(ServerMessage::Snapshot(self.state.lock().snapshot()))
            }
            ClientCommand::UnassignSlot { slot } => {
                self.state.lock().unassign_slot(slot)?;
                Ok(ServerMessage::Snapshot(self.state.lock().snapshot()))
            }
            ClientCommand::LockDevice { device_id } => {
                {
                    let mut state = self.state.lock();
                    state.set_device_locked(&device_id, true)?;
                    if let Some(device) = state.devices.get(&device_id) {
                        if device.assigned_slot.is_some() {
                            // ensure reader is running for this locked device
                            drop(state);
                            self.ensure_reader(device_id.clone());
                            Ok(ServerMessage::Snapshot(self.state.lock().snapshot()))
                        } else {
                            Ok(ServerMessage::Snapshot(state.snapshot()))
                        }
                    } else {
                        Ok(ServerMessage::Snapshot(state.snapshot()))
                    }
                }
            }
            ClientCommand::UnlockDevice { device_id } => {
                self.state.lock().set_device_locked(&device_id, false)?;
                Ok(ServerMessage::Snapshot(self.state.lock().snapshot()))
            }
            ClientCommand::SaveProfile => {
                let mut state = self.state.lock();
                state.sync_profile_bindings();
                state.save_config()?;
                Ok(ServerMessage::Snapshot(state.snapshot()))
            }
            ClientCommand::LoadProfile { name } => {
                self.state.lock().load_profile(&name)?;
                Ok(ServerMessage::Snapshot(self.state.lock().snapshot()))
            }
            ClientCommand::CreateProfile { name } => {
                self.state.lock().create_profile(&name)?;
                Ok(ServerMessage::Snapshot(self.state.lock().snapshot()))
            }
            ClientCommand::DeleteProfile { name } => {
                self.state.lock().delete_profile(&name)?;
                Ok(ServerMessage::Snapshot(self.state.lock().snapshot()))
            }
            ClientCommand::SetBinding { slot, binding } => {
                self.state.lock().set_binding(slot, binding)?;
                Ok(ServerMessage::Snapshot(self.state.lock().snapshot()))
            }
            ClientCommand::StartBindingCapture { slot, action } => {
                self.state.lock().start_binding_capture(slot, action);
                Ok(ServerMessage::Snapshot(self.state.lock().snapshot()))
            }
            ClientCommand::CancelBindingCapture => {
                self.state.lock().cancel_binding_capture();
                Ok(ServerMessage::Snapshot(self.state.lock().snapshot()))
            }
            ClientCommand::InjectTestAction {
                slot,
                action,
                pressed,
            } => {
                let (current, output) = {
                    let mut state = self.state.lock();
                    let runtime_slot = state.slots.get_mut(&slot).context("slot does not exist")?;
                    runtime_slot.apply_action(action, pressed);
                    let current = runtime_slot.status.state;
                    match state.outputs.remove(&slot) {
                        Some(o) => (current, Some(o)),
                        None => (current, None),
                    }
                };
                if let Some(mut output) = output {
                    output.emit_state(&current)?;
                    let mut state = self.state.lock();
                    state.outputs.insert(slot, output);
                }
                Ok(ServerMessage::Snapshot(self.state.lock().snapshot()))
            }
            ClientCommand::Shutdown => Ok(ServerMessage::Ack),
        }
    }

    fn assign_device(&self, device_id: DeviceId, slot: ControllerSlot) -> Result<()> {
        {
            let mut state = self.state.lock();
            state.assign_device(device_id.clone(), slot)?;
        }
        self.ensure_reader(device_id);
        Ok(())
    }

    fn ensure_reader(&self, device_id: DeviceId) {
        if !self.active_readers.lock().insert(device_id.clone()) {
            return;
        }
        let state = self.state.clone();
        let active_readers = self.active_readers.clone();
        thread::spawn(move || {
            let id = device_id.clone();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                reader_loop(state, id.clone());
            }));
            active_readers.lock().remove(&device_id);
            if result.is_err() {
                tracing::error!("reader_loop panicked for {id}");
            }
        });
    }
}

fn assigned_devices(state: &Arc<Mutex<DaemonState>>) -> Vec<DeviceId> {
    state
        .lock()
        .devices
        .values()
        .filter(|device| device.assigned_slot.is_some())
        .map(|device| device.id.clone())
        .collect()
}

impl DaemonState {
    fn snapshot(&self) -> AppSnapshot {
        AppSnapshot {
            devices: self.devices.values().cloned().collect(),
            slots: ControllerSlot::ALL
                .into_iter()
                .filter_map(|slot| self.slots.get(&slot).map(|runtime| runtime.status.clone()))
                .collect(),
            active_profile: self.config.active_profile.clone(),
            profile_names: self.config.profiles.keys().cloned().collect(),
            permission_warnings: self.permission_warnings.clone(),
            event_log: self.event_log.iter().skip(self.event_log.len().saturating_sub(80)).cloned().collect(),
            capture_status: self.capture_status(),
        }
    }

    fn capture_status(&self) -> CaptureStatus {
        match &self.capture_in_progress {
            Some((slot, action, started)) => {
                if started.elapsed() > self.capture_timeout {
                    return CaptureStatus::None;
                }
                CaptureStatus::Waiting {
                    slot: *slot,
                    action: *action,
                }
            }
            None => CaptureStatus::None,
        }
    }

    fn rescan_devices(&mut self) {
        let (devices, warnings) = discover_keyboards();
        self.permission_warnings = warnings;

        match fs::OpenOptions::new().read(true).write(true).open("/dev/uinput") {
            Ok(file) => { drop(file); }
            Err(err) => {
                self.permission_warnings.push(format!(
                    "Cannot open /dev/uinput: {}. Install udev rules and ensure you're in the input group.",
                    err
                ));
            }
        }

        for device in self.devices.values_mut() {
            device.connected = false;
        }

        for mut device in devices {
            if let Some(existing) = self.devices.get(&device.id) {
                device.assigned_slot = existing.assigned_slot;
                device.locked = existing.locked;
                device.last_error = existing.last_error.clone();
            }
            self.devices.insert(device.id.clone(), device);
        }
    }

    fn restore_profile_assignments(&mut self) {
        let Some(profile) = self
            .config
            .profiles
            .get(&self.config.active_profile)
            .cloned()
        else {
            return;
        };
        for (slot, slot_profile) in profile.slots {
            if let Some(device_id) = slot_profile.device_id
                && self.devices.contains_key(&device_id)
            {
                let _ = self.assign_device(device_id.clone(), slot);
                if slot_profile.locked
                    && let Err(err) = self.set_device_locked(&device_id, true)
                {
                    self.log(LogLevel::Warning, format!("Could not lock {device_id}: {err}"));
                }
            }
        }
    }

    fn assign_device(&mut self, device_id: DeviceId, slot: ControllerSlot) -> Result<()> {
        anyhow::ensure!(
            self.devices
                .get(&device_id)
                .is_some_and(|device| device.connected),
            "keyboard is not connected"
        );

        for runtime_slot in self.slots.values_mut() {
            if runtime_slot.status.device_id.as_ref() == Some(&device_id) {
                runtime_slot.status.device_id = None;
                runtime_slot.status.locked = false;
                runtime_slot.status.controller_ready = false;
                runtime_slot.status.lifecycle = SlotLifecycle::Empty;
                runtime_slot.clear_inputs();
            }
        }
        for device in self.devices.values_mut() {
            if device.assigned_slot == Some(slot) || device.id == device_id {
                device.assigned_slot = None;
            }
        }

        let output_ready = match self.outputs.entry(slot) {
            std::collections::btree_map::Entry::Occupied(_) => true,
            std::collections::btree_map::Entry::Vacant(entry) => match VirtualGamepad::create(slot)
            {
                Ok(output) => {
                    entry.insert(output);
                    true
                }
                Err(err) => {
                    let msg = format!("Could not create virtual controller for {slot}: {err:#}");
                    self.log(LogLevel::Error, msg.clone());
                    false
                }
            },
        };

        let device = self.devices.get_mut(&device_id)
            .context("device disappeared during assignment")?;
        device.assigned_slot = Some(slot);
        let locked = device.locked;

        let runtime_slot = self.slots.get_mut(&slot).context("slot does not exist")?;
        runtime_slot.status.device_id = Some(device_id.clone());
        runtime_slot.status.locked = locked;
        runtime_slot.status.controller_ready = output_ready;
        runtime_slot.status.lifecycle = if locked {
            SlotLifecycle::Locked
        } else {
            SlotLifecycle::Bound
        };
        runtime_slot.status.last_error = None;
        if let Some(profile) = self.config.profiles.get(&self.config.active_profile)
            && let Some(slot_profile) = profile.slots.get(&slot)
        {
            runtime_slot.set_bindings(slot_profile.bindings.clone());
        }

        if let Some(profile) = self.config.profiles.get_mut(&self.config.active_profile)
            && let Some(slot_profile) = profile.slots.get_mut(&slot)
        {
            slot_profile.device_id = Some(device_id.clone());
            slot_profile.locked = locked;
        }
        self.save_config()?;
        self.log(LogLevel::Info, format!("Assigned keyboard to {slot}"));
        Ok(())
    }

    fn unassign_slot(&mut self, slot: ControllerSlot) -> Result<()> {
        let Some(runtime_slot) = self.slots.get_mut(&slot) else {
            anyhow::bail!("slot does not exist");
        };
        if let Some(device_id) = runtime_slot.status.device_id.take()
            && let Some(device) = self.devices.get_mut(&device_id)
        {
            device.assigned_slot = None;
            device.locked = false;
        }
        runtime_slot.status.locked = false;
        runtime_slot.status.controller_ready = false;
        runtime_slot.status.lifecycle = SlotLifecycle::Empty;
        runtime_slot.clear_inputs();
        self.outputs.remove(&slot);
        if let Some(profile) = self.config.profiles.get_mut(&self.config.active_profile)
            && let Some(slot_profile) = profile.slots.get_mut(&slot)
        {
            slot_profile.device_id = None;
            slot_profile.locked = false;
        }
        self.save_config()?;
        self.log(LogLevel::Info, format!("Cleared {slot}"));
        Ok(())
    }

    fn set_device_locked(&mut self, device_id: &DeviceId, locked: bool) -> Result<()> {
        let device = self
            .devices
            .get_mut(device_id)
            .context("keyboard not found")?;
        if locked && !device.can_grab {
            anyhow::bail!(
                "Cannot lock keyboard: no write permission on '{}'. \
                 Add yourself to the 'input' group and install udev rules.",
                device.path
            );
        }
        device.locked = locked;
        if let Some(slot) = device.assigned_slot {
            if let Some(runtime_slot) = self.slots.get_mut(&slot) {
                runtime_slot.status.locked = locked;
                runtime_slot.status.lifecycle = if locked {
                    SlotLifecycle::Locked
                } else {
                    SlotLifecycle::Bound
                };
            }
            if let Some(profile) = self.config.profiles.get_mut(&self.config.active_profile)
                && let Some(slot_profile) = profile.slots.get_mut(&slot)
            {
                slot_profile.locked = locked;
            }
        }
        self.save_config()?;
        self.log(
            LogLevel::Info,
            format!("{} keyboard", if locked { "Locked" } else { "Unlocked" }),
        );
        Ok(())
    }

    fn set_binding(
        &mut self,
        slot: ControllerSlot,
        binding: kbdsplit_shared::KeyBinding,
    ) -> Result<()> {
        let Some(profile) = self.config.profiles.get_mut(&self.config.active_profile) else {
            anyhow::bail!("active profile does not exist");
        };
        let Some(slot_profile) = profile.slots.get_mut(&slot) else {
            anyhow::bail!("slot profile does not exist");
        };
        if let Some(existing) = slot_profile
            .bindings
            .iter_mut()
            .find(|existing| existing.key == binding.key)
        {
            *existing = binding;
        } else {
            slot_profile.bindings.push(binding);
        }
        self.sync_profile_bindings();
        self.save_config()
    }

    fn load_profile(&mut self, name: &str) -> Result<()> {
        anyhow::ensure!(self.config.profiles.contains_key(name), "profile not found");
        self.config.active_profile = name.to_owned();
        self.restore_profile_assignments();
        self.sync_profile_bindings();
        self.save_config()
    }

    fn create_profile(&mut self, name: &str) -> Result<()> {
        anyhow::ensure!(!self.config.profiles.contains_key(name), "profile already exists");
        let profile = kbdsplit_core::Profile {
            name: name.to_owned(),
            slots: self.config.profiles.get(&self.config.active_profile)
                .map(|p| p.slots.clone())
                .unwrap_or_else(|| kbdsplit_core::Profile::default().slots),
        };
        self.config.active_profile = name.to_owned();
        self.config.profiles.insert(name.to_owned(), profile);
        self.restore_profile_assignments();
        self.sync_profile_bindings();
        self.save_config()
    }

    fn delete_profile(&mut self, name: &str) -> Result<()> {
        anyhow::ensure!(self.config.profiles.len() > 1, "cannot delete the last profile");
        anyhow::ensure!(self.config.profiles.contains_key(name), "profile not found");
        self.config.profiles.remove(name);
        if self.config.active_profile == name {
            self.config.active_profile = self.config.profiles.keys().next().unwrap().clone();
            self.restore_profile_assignments();
        }
        self.sync_profile_bindings();
        self.save_config()
    }

    fn sync_profile_bindings(&mut self) {
        let Some(profile) = self.config.profiles.get(&self.config.active_profile).cloned() else {
            return;
        };
        for (slot, slot_profile) in &profile.slots {
            if let Some(runtime_slot) = self.slots.get_mut(slot) {
                runtime_slot.set_bindings(slot_profile.bindings.clone());
            }
        }
    }

    fn save_config(&self) -> Result<()> {
        self.store.save(&self.config)
    }

    fn log(&mut self, level: LogLevel, message: String) {
        self.event_log.push(EventLogEntry {
            millis: self.started_at.elapsed().as_millis(),
            level,
            message,
        });
        if self.event_log.len() > 200 {
            let excess = self.event_log.len() - 200;
            self.event_log.drain(..excess);
        }
    }

    fn release_all_locks(&mut self) {
        let locked_slots: Vec<ControllerSlot> = self
            .devices
            .values()
            .filter(|d| d.locked)
            .filter_map(|d| d.assigned_slot)
            .collect();
        for device in self.devices.values_mut() {
            if device.locked {
                device.locked = false;
                device.assigned_slot = None;
            }
        }
        for slot in &locked_slots {
            self.outputs.remove(slot);
            if let Some(runtime_slot) = self.slots.get_mut(slot) {
                runtime_slot.status.locked = false;
                runtime_slot.status.lifecycle = SlotLifecycle::Empty;
                runtime_slot.status.controller_ready = false;
                runtime_slot.clear_inputs();
            }
        }
        if let Some(profile) = self.config.profiles.get_mut(&self.config.active_profile) {
            for slot_profile in profile.slots.values_mut() {
                slot_profile.device_id = None;
                slot_profile.locked = false;
            }
        }
        self.log(LogLevel::Warning, "KILL SWITCH: all locks released".to_owned());
        let _ = self.save_config();
    }

    fn start_binding_capture(&mut self, slot: ControllerSlot, action: ControllerAction) {
        if self.capture_in_progress.is_some() {
            self.log(LogLevel::Warning, "Overwriting existing capture".to_owned());
        }
        self.capture_in_progress = Some((slot, action, Instant::now()));
        self.log(LogLevel::Info, format!("Capture started for {slot}: {action}"));
    }

    fn cancel_binding_capture(&mut self) {
        self.capture_in_progress = None;
    }
}

fn spawn_hotplug_watcher(
    state: Arc<Mutex<DaemonState>>,
    active_readers: Arc<Mutex<BTreeSet<DeviceId>>>,
) {
    thread::spawn(move || {
        loop {
            if SHUTDOWN.load(Ordering::Acquire) {
                return;
            }
            thread::sleep(Duration::from_secs(1));
            if SHUTDOWN.load(Ordering::Acquire) {
                return;
            }
            let mut state_lock = state.lock();
            let before = state_lock
                .devices
                .values()
                .filter(|device| device.connected)
                .count();
            state_lock.rescan_devices();
            let after = state_lock
                .devices
                .values()
                .filter(|device| device.connected)
                .count();
            if before != after {
                state_lock.log(
                    LogLevel::Info,
                    format!("Keyboard list updated: {after} connected"),
                );
            }
            let reconnected: Vec<DeviceId> = state_lock
                .devices
                .values()
                .filter(|device| device.connected && device.assigned_slot.is_some())
                .map(|device| device.id.clone())
                .filter(|id| !active_readers.lock().contains(id))
                .collect();
            for device_id in &reconnected {
                if let Some(device) = state_lock.devices.get(device_id)
                    && let Some(slot) = device.assigned_slot
                    && let Some(runtime_slot) = state_lock.slots.get_mut(&slot)
                {
                    runtime_slot.status.lifecycle = if runtime_slot.status.locked {
                        SlotLifecycle::Locked
                    } else {
                        SlotLifecycle::Bound
                    };
                    runtime_slot.status.last_error = None;
                }
            }
            drop(state_lock);
            for device_id in reconnected {
                if active_readers.lock().insert(device_id.clone()) {
                    let state = state.clone();
                    let readers = active_readers.clone();
                    let id = device_id.clone();
                    thread::spawn(move || {
                        let dev_id = id.clone();
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            reader_loop(state, dev_id.clone());
                        }));
                        readers.lock().remove(&id);
                        if result.is_err() {
                            tracing::error!("reader_loop panicked for {dev_id}");
                        }
                    });
                }
            }
        }
    });
}

fn reader_loop(state: Arc<Mutex<DaemonState>>, device_id: DeviceId) {
    let path = {
        let s = state.lock();
        let Some(device) = s.devices.get(&device_id) else { return };
        PathBuf::from(&device.path)
    };

    let mut reader = match InputReader::open(&path) {
        Ok(reader) => reader,
        Err(err) => {
            let mut s = state.lock();
            if let Some(device) = s.devices.get_mut(&device_id) {
                device.connected = false;
                device.last_error = Some(format!("{err:#}"));
            }
            s.log(
                LogLevel::Error,
                format!("Could not read {}: {err:#}", path.display()),
            );
            return;
        }
    };

    // Read initial slot/locked state — then cache and refresh periodically
    let current_slot = {
        let s = state.lock();
        let Some(device) = s.devices.get(&device_id) else { return };
        let Some(slot) = device.assigned_slot else {
            let _ = reader.set_grabbed(false);
            return;
        };
        slot
    };

    // Initial grab
    if let Err(err) = reader.set_grabbed(
        state.lock().devices.get(&device_id)
            .is_some_and(|d| d.locked)
    ) {
        let mut s = state.lock();
        if let Some(device) = s.devices.get_mut(&device_id) {
            device.last_error = Some(format!("{err:#}"));
        }
        if let Some(runtime_slot) = s.slots.get_mut(&current_slot) {
            runtime_slot.status.lifecycle = SlotLifecycle::Bound;
            runtime_slot.status.locked = false;
            runtime_slot.status.last_error = Some(format!("{err:#}"));
        }
        s.log(LogLevel::Error, format!("Could not lock keyboard: {err:#}"));
    }

    // Kill-switch: both Shift keys + Esc releases all locks
    const KEY_LEFTSHIFT: u16 = 42;
    const KEY_RIGHTSHIFT: u16 = 54;
    const KEY_ESC: u16 = 1;
    let mut kill_left_shift = false;
    let mut kill_right_shift = false;

    loop {
        if SHUTDOWN.load(Ordering::Acquire) {
            return;
        }

        // Wait for events with 100ms timeout (for periodic state refresh).
        // Instant wakeup on key press — no polling latency.
        let available = match reader.wait_for_event(100) {
            Ok(a) => a,
            Err(_) => {
                // Check if device still exists on epoll error
                let s = state.lock();
                if !s.devices.contains_key(&device_id) { return; }
                continue;
            }
        };

        if available {
            // Read ALL buffered events without sleeping between them.
            // This eliminates the 4ms inter-event latency that was causing
            // bursty gamepad output and delayed responses.
            while let Ok(Some(event)) = reader.read_event() {
                if SHUTDOWN.load(Ordering::Acquire) { return; }

                if event.type_ == EV_KEY && event.value != 2 {
                    let pressed = event.value != 0;
                    match event.code {
                        KEY_LEFTSHIFT => kill_left_shift = pressed,
                        KEY_RIGHTSHIFT => kill_right_shift = pressed,
                        _ => {}
                    }
                    if pressed && kill_left_shift && kill_right_shift && event.code == KEY_ESC {
                        state.lock().release_all_locks();
                        return;
                    }
                    handle_key_event(&state, current_slot, event.code, pressed);
                } else if event.type_ == EV_SYN && event.code == SYN_DROPPED {
                    handle_syn_dropped(&state, &mut reader, current_slot);
                }
            }
        }

        // Periodic refresh (every 100ms via epoll timeout):
        // check slot assignment, lock state, and grab status.
        // Eliminates the global mutex acquisition on idle iterations.
        let mut s = state.lock();
        let Some(device) = s.devices.get(&device_id) else { return };
        let Some(slot) = device.assigned_slot else {
            let _ = reader.set_grabbed(false);
            return;
        };
        if slot != current_slot {
            return;
        }
        if device.locked != reader.grabbed()
            && let Err(err) = reader.set_grabbed(device.locked)
        {
            s.log(LogLevel::Error, format!("Could not lock keyboard: {err:#}"));
        }
    }
}

fn handle_key_event(
    state: &Arc<Mutex<DaemonState>>,
    slot: ControllerSlot,
    key_code: u16,
    pressed: bool,
) {
    // Phase 1: Lock — update state, extract output
    let (current, mut output) = {
        let mut s = state.lock();

        // Auto-cancel expired captures
        if let Some((_, _, started)) = &s.capture_in_progress
            && started.elapsed() > s.capture_timeout
        {
            s.log(LogLevel::Info, "Capture timed out".to_owned());
            s.capture_in_progress = None;
        }

        // Check capture mode
        if let Some((capture_slot, capture_action, _started)) = &s.capture_in_progress.clone() {
            if *capture_slot == slot && pressed {
                let label = kbdsplit_core::key_label(key_code);
                let action_str = *capture_action;
                let binding = KeyBinding {
                    key: KeyCode(key_code),
                    label,
                    action: *capture_action,
                };

                let profile_name = s.config.active_profile.clone();
                if let Some(profile) = s.config.profiles.get_mut(&profile_name)
                    && let Some(slot_profile) = profile.slots.get_mut(&slot)
                {
                    slot_profile.bindings.retain(|b| b.action != binding.action);
                    slot_profile.bindings.retain(|b| b.key.0 != key_code);
                    slot_profile.bindings.push(binding);
                }
                s.sync_profile_bindings();
                let _ = s.save_config();
                s.capture_in_progress = None;
                s.log(LogLevel::Info, format!("Bound key {} to {action_str} in {slot}", key_code));
                return;
            }
            if pressed {
                s.log(LogLevel::Info, format!("Key press on {slot}, but capture was for different slot"));
            }
        }

        // Normal key handling
        if !s.outputs.contains_key(&slot) { return; }

        let current = if pressed {
            let binding = s
                .config
                .profiles
                .get(&s.config.active_profile)
                .and_then(|profile| profile.slots.get(&slot))
                .and_then(|slot_profile| {
                    slot_profile.bindings.iter().find(|b| b.key.0 == key_code).cloned()
                });
            let Some(binding) = binding else { return };
            let Some(runtime_slot) = s.slots.get_mut(&slot) else { return };
            runtime_slot.key_down(KeyCode(key_code), binding.action);
            runtime_slot.status.state
        } else {
            let Some(runtime_slot) = s.slots.get_mut(&slot) else { return };
            runtime_slot.key_up(KeyCode(key_code));
            runtime_slot.status.state
        };

        let output = s.outputs.remove(&slot).unwrap();
        (current, output)
    };

    // Phase 2: Emit WITHOUT the lock — this is the longest syscall
    let result = output.emit_state(&current);

    // Phase 3: Re-lock for error recovery and to put output back
    let mut s = state.lock();
    s.outputs.insert(slot, output);

    match result {
        Ok(()) => {
            if let Some(runtime_slot) = s.slots.get_mut(&slot)
                && runtime_slot.status.lifecycle == SlotLifecycle::Error
            {
                runtime_slot.status.lifecycle = if runtime_slot.status.locked {
                    SlotLifecycle::Locked
                } else {
                    SlotLifecycle::Bound
                };
                runtime_slot.status.last_error = None;
            }
        }
        Err(err) => {
            if let Some(runtime_slot) = s.slots.get_mut(&slot) {
                runtime_slot.status.lifecycle = SlotLifecycle::Error;
                runtime_slot.status.last_error = Some(format!("{err:#}"));
            }
            s.log(
                LogLevel::Error,
                format!("Controller output failed for {slot}: {err:#}"),
            );
        }
    }
}

fn handle_syn_dropped(
    state: &Arc<Mutex<DaemonState>>,
    reader: &mut InputReader,
    slot: ControllerSlot,
) {
    let bitmap = match reader.read_key_bitmap() {
        Ok(b) => b,
        Err(err) => {
            tracing::warn!("EVIOCGKEY failed for {slot}: {err}");
            return;
        }
    };

    // Clone bindings out of the lock to avoid borrow conflicts
    let bindings = {
        let s = state.lock();
        s.config
            .profiles
            .get(&s.config.active_profile)
            .and_then(|p| p.slots.get(&slot))
            .map(|sp| sp.bindings.clone())
    };
    let Some(bindings) = bindings else { return };

    // Phase 1: Lock — reconcile state, extract output
    let (current, mut output) = {
        let mut s = state.lock();
        let Some(runtime_slot) = s.slots.get_mut(&slot) else { return };
        if !runtime_slot.reconcile_from_bitmap(&bitmap, &bindings) {
            return;
        }
        let current = runtime_slot.status.state;
        let Some(output) = s.outputs.remove(&slot) else { return };
        (current, output)
    };

    // Phase 2: Emit WITHOUT the lock
    let result = output.emit_state(&current);

    // Phase 3: Re-lock for error recovery and to put output back
    let mut s = state.lock();
    s.outputs.insert(slot, output);

    match result {
        Ok(()) => {
            if let Some(runtime_slot) = s.slots.get_mut(&slot)
                && runtime_slot.status.lifecycle == SlotLifecycle::Error
            {
                runtime_slot.status.lifecycle = if runtime_slot.status.locked {
                    SlotLifecycle::Locked
                } else {
                    SlotLifecycle::Bound
                };
                runtime_slot.status.last_error = None;
            }
        }
        Err(err) => {
            if let Some(runtime_slot) = s.slots.get_mut(&slot) {
                runtime_slot.status.lifecycle = SlotLifecycle::Error;
                runtime_slot.status.last_error = Some(format!("{err:#}"));
            }
            s.log(
                LogLevel::Error,
                format!("SYN_DROPPED: controller output failed for {slot}: {err:#}"),
            );
            s.outputs.remove(&slot);
        }
    }
}
