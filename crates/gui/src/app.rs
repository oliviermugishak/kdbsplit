use crate::ipc_client::IpcClient;
use egui::{
    Align2, Color32, FontId, Painter, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2,
};
use kbdsplit_shared::{
    AppSnapshot, CaptureStatus, ClientCommand, ControllerAction, ControllerSlot, ControllerState,
    DEFAULT_SOCKET_PATH, DeviceId, GamepadButton, Stick, Direction, Trigger, KeyboardDevice,
    LogLevel, ServerMessage,
    SlotLifecycle, SlotStatus,
};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

pub struct KbdSplitApp {
    ipc: IpcClient,
    snapshot: Option<AppSnapshot>,
    selected_slot: ControllerSlot,
    last_poll: Instant,
    connection_error: Option<String>,
    daemon_start_attempted: bool,
    new_profile_name: String,
}

impl KbdSplitApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            ipc: IpcClient::new(DEFAULT_SOCKET_PATH),
            snapshot: None,
            selected_slot: ControllerSlot::One,
            last_poll: Instant::now() - Duration::from_secs(1),
            connection_error: None,
            daemon_start_attempted: false,
            new_profile_name: String::new(),
        }
    }

    fn poll_snapshot(&mut self) {
        if self.last_poll.elapsed() < Duration::from_millis(16) {
            return;
        }
        self.last_poll = Instant::now();
        match self.ipc.request(&ClientCommand::GetSnapshot) {
            Ok(ServerMessage::Snapshot(snapshot)) => {
                self.snapshot = Some(snapshot);
                self.connection_error = None;
            }
            Ok(ServerMessage::Error(error)) => self.connection_error = Some(error),
            Ok(ServerMessage::Ack) => {}
            Err(err) => {
                if !self.daemon_start_attempted {
                    self.daemon_start_attempted = true;
                    if let Err(start_err) = self.ipc.ensure_daemon_started() {
                        self.connection_error = Some(format!("{err:#}; {start_err:#}"));
                    }
                    return;
                }
                self.connection_error = Some(format!("{err:#}"));
            }
        }
    }

    fn send(&mut self, command: ClientCommand) {
        match self.ipc.request(&command) {
            Ok(ServerMessage::Snapshot(snapshot)) => {
                self.snapshot = Some(snapshot);
                self.connection_error = None;
            }
            Ok(ServerMessage::Ack) => self.connection_error = None,
            Ok(ServerMessage::Error(error)) => self.connection_error = Some(error),
            Err(err) => self.connection_error = Some(format!("{err:#}")),
        }
        self.last_poll = Instant::now() - Duration::from_secs(1);
    }
}

impl eframe::App for KbdSplitApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_snapshot();
        ctx.request_repaint_after(Duration::from_millis(33));

        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("KbdSplit");
                ui.separator();
                ui.label("Keyboard to Xbox controller splitter");
                if let Some(snapshot) = &self.snapshot {
                    ui.separator();
                    ui.label(format!("Profile: {}", snapshot.active_profile));
                }
            });
        });

        egui::SidePanel::left("devices")
            .resizable(true)
            .default_width(480.0)
            .width_range(320.0..=700.0)
            .show(ctx, |ui| self.draw_left_panel(ui));

        egui::TopBottomPanel::bottom("event_log")
            .resizable(true)
            .default_height(120.0)
            .show(ctx, |ui| self.draw_bottom_panel(ui));

        egui::CentralPanel::default().show(ctx, |ui| self.draw_main_panel(ui));
    }
}

impl KbdSplitApp {
    fn draw_left_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Controller Slot").strong());
            for slot in ControllerSlot::ALL {
                let selected = self.selected_slot == slot;
                if ui
                    .selectable_label(selected, slot.number().to_string())
                    .on_hover_text(format!("Configure {slot}"))
                    .clicked()
                {
                    self.selected_slot = slot;
                }
            }
        });
        ui.add_space(8.0);

        if let Some(error) = &self.connection_error {
            warning_band(ui, Color32::from_rgb(130, 55, 38), "Daemon", error);
            if ui.button("Start daemon").clicked() {
                match self.ipc.ensure_daemon_started() {
                    Ok(()) => self.connection_error = None,
                    Err(err) => self.connection_error = Some(format!("{err:#}")),
                }
            }
            ui.add_space(8.0);
        }

        let Some(snapshot) = self.snapshot.clone() else {
            ui.label("Waiting for daemon...");
            return;
        };

        for warning in &snapshot.permission_warnings {
            warning_band(ui, Color32::from_rgb(128, 80, 24), "Permission", warning);
        }

        ui.separator();
        ui.label(RichText::new("Profile").strong());
        ui.horizontal(|ui| {
            let active = &snapshot.active_profile;
            let all_profiles = &snapshot.profile_names;
            egui::ComboBox::from_id_salt("profile_selector")
                .width(180.0)
                .selected_text(active)
                .show_ui(ui, |ui| {
                    for name in all_profiles {
                        let is_active = name == active;
                        if ui
                            .selectable_label(is_active, name)
                            .clicked()
                            && !is_active
                        {
                            self.send(ClientCommand::LoadProfile { name: name.clone() });
                        }
                    }
                });
            if ui.button("Save").on_hover_text("Save current profile").clicked() {
                self.send(ClientCommand::SaveProfile);
            }
        });
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.new_profile_name);
            if ui.button("New").on_hover_text("Create new profile").clicked()
                && !self.new_profile_name.is_empty()
            {
                self.send(ClientCommand::CreateProfile {
                    name: self.new_profile_name.clone(),
                });
                self.new_profile_name.clear();
            }
            let can_delete = snapshot.profile_names.len() > 1;
            if ui
                .add_enabled(can_delete, egui::Button::new("Delete"))
                .on_disabled_hover_text("Cannot delete the last profile")
                .clicked()
            {
                self.send(ClientCommand::DeleteProfile {
                    name: snapshot.active_profile.clone(),
                });
            }
        });
        ui.add_space(4.0);

        ui.separator();
        ui.label(RichText::new("Keyboards").strong());
        ui.add_space(4.0);

        if snapshot.devices.is_empty() {
            ui.label("No keyboards are visible to the daemon.");
        }

        for device in &snapshot.devices {
            self.draw_device_row(ui, device);
        }

        ui.add_space(12.0);
        ui.separator();

        // Get current slot status and show bindings
        let slot_status = snapshot
            .slots
            .iter()
            .find(|slot| slot.slot == self.selected_slot)
            .cloned();

        if let Some(slot) = &slot_status {
            self.draw_binding_editor(ui, slot);
        }

        ui.add_space(12.0);
        ui.separator();
        if let Some(slot) = slot_status {
            self.draw_slot_controls(ui, &slot);
        } else {
            ui.label("No slot selected.");
        }
    }

    fn draw_device_row(&mut self, ui: &mut egui::Ui, device: &KeyboardDevice) {
        let assigned_here = device.assigned_slot == Some(self.selected_slot);
        let text = if device.is_internal {
            format!("{}  (internal)", device.name)
        } else {
            device.name.clone()
        };
        ui.group(|ui| {
            ui.horizontal(|ui| {
                let status_color = if device.connected {
                    Color32::from_rgb(56, 153, 96)
                } else {
                    Color32::from_rgb(150, 68, 68)
                };
                ui.colored_label(status_color, "●");
                ui.vertical(|ui| {
                    ui.label(RichText::new(text).strong());
                    ui.label(RichText::new(&device.path).small().color(Color32::GRAY));
                });
            });
            ui.horizontal(|ui| {
                if let Some(slot) = device.assigned_slot {
                    ui.label(format!("Assigned to {slot}"));
                } else {
                    ui.label("Available");
                }
                if device.locked {
                    ui.colored_label(Color32::from_rgb(200, 118, 42), "Locked");
                }
            });
            ui.horizontal(|ui| {
                if assigned_here {
                    if ui
                        .add_enabled(device.connected, egui::Button::new("Unassign"))
                        .clicked()
                    {
                        self.send(ClientCommand::UnassignSlot { slot: device.assigned_slot.unwrap() });
                    }
                } else if ui
                    .add_enabled(device.connected, egui::Button::new("Assign"))
                    .clicked()
                {
                    self.send(ClientCommand::AssignDevice {
                        device_id: device.id.clone(),
                        slot: self.selected_slot,
                    });
                }
                if device.assigned_slot.is_some() {
                    let command = if device.locked {
                        ClientCommand::UnlockDevice {
                            device_id: device.id.clone(),
                        }
                    } else {
                        ClientCommand::LockDevice {
                            device_id: device.id.clone(),
                        }
                    };
                    let lock_enabled = device.locked || device.can_grab;
                    if ui
                        .add_enabled(
                            lock_enabled,
                            egui::Button::new(if device.locked { "Unlock" } else { "Lock" }),
                        )
                        .on_hover_text(
                            "When locked, Linux stops forwarding this keyboard to other apps.",
                        )
                        .clicked()
                    {
                        self.send(command);
                    }
                    if !device.can_grab && !device.locked {
                        ui.label(
                            egui::RichText::new("(no write permission)")
                                .small()
                                .color(egui::Color32::GRAY),
                        );
                    }
                }
            });
            if let Some(error) = &device.last_error {
                ui.colored_label(Color32::from_rgb(210, 86, 78), error);
            }
        });
        ui.add_space(6.0);
    }

    fn draw_binding_editor(&mut self, ui: &mut egui::Ui, slot: &SlotStatus) {
        // Build a map of action -> key_label from bindings
        let binding_map: BTreeMap<ControllerAction, String> = slot
            .bindings
            .iter()
            .map(|b| (b.action, b.label.clone()))
            .collect();

        // Check capture status
        let capture_action = self.snapshot.as_ref()
            .map(|snap| match &snap.capture_status {
                CaptureStatus::Waiting { slot: s, action } if *s == slot.slot => Some(*action),
                _ => None,
            })
            .unwrap_or(None);

        // Face buttons section
        ui.label(RichText::new("Face Buttons").strong());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for button in [GamepadButton::North, GamepadButton::West, GamepadButton::South, GamepadButton::East] {
                let key_label = binding_map.get(&ControllerAction::Button(button)).cloned().unwrap_or_default();
                let is_capturing = capture_action.as_ref().map(|a| {
                    if let ControllerAction::Button(b) = a { *b == button } else { false }
                }).unwrap_or(false);
                let display = if is_capturing {
                    format!("{} (press key...)", button)
                } else {
                    format!("{}: {}", button, key_label)
                };
                if ui.button(display).clicked() {
                    self.send(ClientCommand::StartBindingCapture {
                        slot: self.selected_slot,
                        action: ControllerAction::Button(button),
                    });
                }
                ui.add_space(4.0);
            }
        });
        ui.add_space(8.0);

        // Shoulder buttons section
        ui.label(RichText::new("Shoulder Buttons").strong());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let lb_key = binding_map.get(&ControllerAction::Button(GamepadButton::LeftShoulder)).cloned().unwrap_or_default();
            let rb_key = binding_map.get(&ControllerAction::Button(GamepadButton::RightShoulder)).cloned().unwrap_or_default();
            let lb_capturing = capture_action.as_ref().map(|a| {
                if let ControllerAction::Button(b) = a { *b == GamepadButton::LeftShoulder } else { false }
            }).unwrap_or(false);
            let rb_capturing = capture_action.as_ref().map(|a| {
                if let ControllerAction::Button(b) = a { *b == GamepadButton::RightShoulder } else { false }
            }).unwrap_or(false);

            if ui.button(format!("LB: {}", if lb_capturing { "press key..." } else { &lb_key })).clicked() {
                self.send(ClientCommand::StartBindingCapture {
                    slot: self.selected_slot,
                    action: ControllerAction::Button(GamepadButton::LeftShoulder),
                });
            }
            if ui.button(format!("RB: {}", if rb_capturing { "press key..." } else { &rb_key })).clicked() {
                self.send(ClientCommand::StartBindingCapture {
                    slot: self.selected_slot,
                    action: ControllerAction::Button(GamepadButton::RightShoulder),
                });
            }
        });
        ui.add_space(8.0);

        // Menu buttons section
        ui.label(RichText::new("Menu Buttons").strong());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let back_key = binding_map.get(&ControllerAction::Button(GamepadButton::Select)).cloned().unwrap_or_default();
            let start_key = binding_map.get(&ControllerAction::Button(GamepadButton::Start)).cloned().unwrap_or_default();
            let guide_key = binding_map.get(&ControllerAction::Button(GamepadButton::Guide)).cloned().unwrap_or_default();
            let back_capturing = capture_action.as_ref().map(|a| {
                if let ControllerAction::Button(b) = a { *b == GamepadButton::Select } else { false }
            }).unwrap_or(false);
            let start_capturing = capture_action.as_ref().map(|a| {
                if let ControllerAction::Button(b) = a { *b == GamepadButton::Start } else { false }
            }).unwrap_or(false);
            let guide_capturing = capture_action.as_ref().map(|a| {
                if let ControllerAction::Button(b) = a { *b == GamepadButton::Guide } else { false }
            }).unwrap_or(false);

            if ui.button(format!("Back: {}", if back_capturing { "press key..." } else { &back_key })).clicked() {
                self.send(ClientCommand::StartBindingCapture {
                    slot: self.selected_slot,
                    action: ControllerAction::Button(GamepadButton::Select),
                });
            }
            if ui.button(format!("Start: {}", if start_capturing { "press key..." } else { &start_key })).clicked() {
                self.send(ClientCommand::StartBindingCapture {
                    slot: self.selected_slot,
                    action: ControllerAction::Button(GamepadButton::Start),
                });
            }
            if ui.button(format!("Guide: {}", if guide_capturing { "press key..." } else { &guide_key })).clicked() {
                self.send(ClientCommand::StartBindingCapture {
                    slot: self.selected_slot,
                    action: ControllerAction::Button(GamepadButton::Guide),
                });
            }
        });
        ui.add_space(8.0);

        // Triggers section
        ui.label(RichText::new("Triggers").strong());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let lt_key = binding_map.get(&ControllerAction::Trigger(Trigger::Left)).cloned().unwrap_or_default();
            let rt_key = binding_map.get(&ControllerAction::Trigger(Trigger::Right)).cloned().unwrap_or_default();
            let lt_capturing = capture_action.as_ref().map(|a| {
                if let ControllerAction::Trigger(t) = a { *t == Trigger::Left } else { false }
            }).unwrap_or(false);
            let rt_capturing = capture_action.as_ref().map(|a| {
                if let ControllerAction::Trigger(t) = a { *t == Trigger::Right } else { false }
            }).unwrap_or(false);

            if ui.button(format!("LT: {}", if lt_capturing { "press key..." } else { &lt_key })).clicked() {
                self.send(ClientCommand::StartBindingCapture {
                    slot: self.selected_slot,
                    action: ControllerAction::Trigger(Trigger::Left),
                });
            }
            if ui.button(format!("RT: {}", if rt_capturing { "press key..." } else { &rt_key })).clicked() {
                self.send(ClientCommand::StartBindingCapture {
                    slot: self.selected_slot,
                    action: ControllerAction::Trigger(Trigger::Right),
                });
            }
        });
        ui.add_space(8.0);

        // Thumb stick buttons section
        ui.label(RichText::new("Thumb Stick Buttons").strong());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let ls_key = binding_map.get(&ControllerAction::Button(GamepadButton::LeftThumb)).cloned().unwrap_or_default();
            let rs_key = binding_map.get(&ControllerAction::Button(GamepadButton::RightThumb)).cloned().unwrap_or_default();
            let ls_capturing = capture_action.as_ref().map(|a| {
                if let ControllerAction::Button(b) = a { *b == GamepadButton::LeftThumb } else { false }
            }).unwrap_or(false);
            let rs_capturing = capture_action.as_ref().map(|a| {
                if let ControllerAction::Button(b) = a { *b == GamepadButton::RightThumb } else { false }
            }).unwrap_or(false);

            if ui.button(format!("LS: {}", if ls_capturing { "press key..." } else { &ls_key })).clicked() {
                self.send(ClientCommand::StartBindingCapture {
                    slot: self.selected_slot,
                    action: ControllerAction::Button(GamepadButton::LeftThumb),
                });
            }
            if ui.button(format!("RS: {}", if rs_capturing { "press key..." } else { &rs_key })).clicked() {
                self.send(ClientCommand::StartBindingCapture {
                    slot: self.selected_slot,
                    action: ControllerAction::Button(GamepadButton::RightThumb),
                });
            }
        });
        ui.add_space(8.0);

        // D-Pad section
        ui.label(RichText::new("D-Pad").strong());
        ui.add_space(4.0);
        for (button, name) in [
            (GamepadButton::DpadUp, "Up"),
            (GamepadButton::DpadDown, "Down"),
            (GamepadButton::DpadLeft, "Left"),
            (GamepadButton::DpadRight, "Right"),
        ] {
            let key = binding_map.get(&ControllerAction::Button(button)).cloned().unwrap_or_default();
            let is_capturing = capture_action.as_ref().map(|a| {
                if let ControllerAction::Button(b) = a { *b == button } else { false }
            }).unwrap_or(false);

            if ui.button(format!("D-{}: {}", name, if is_capturing { "press key..." } else { &key })).clicked() {
                self.send(ClientCommand::StartBindingCapture {
                    slot: self.selected_slot,
                    action: ControllerAction::Button(button),
                });
            }
        }
        ui.add_space(8.0);

        // Left Stick section
        ui.label(RichText::new("Left Stick").strong());
        ui.add_space(4.0);
        for (dir, name) in [
            (Direction::Up, "Up"),
            (Direction::Down, "Down"),
            (Direction::Left, "Left"),
            (Direction::Right, "Right"),
        ] {
            let key = binding_map.get(&ControllerAction::Stick { stick: Stick::Left, direction: dir }).cloned().unwrap_or_default();
            let is_capturing = capture_action.as_ref().map(|a| {
                if let ControllerAction::Stick { stick, direction } = a {
                    *stick == Stick::Left && *direction == dir
                } else { false }
            }).unwrap_or(false);

            if ui.button(format!("L-{}: {}", name, if is_capturing { "press key..." } else { &key })).clicked() {
                self.send(ClientCommand::StartBindingCapture {
                    slot: self.selected_slot,
                    action: ControllerAction::Stick { stick: Stick::Left, direction: dir },
                });
            }
        }
        ui.add_space(8.0);

        // Right Stick section
        ui.label(RichText::new("Right Stick").strong());
        ui.add_space(4.0);
        for (dir, name) in [
            (Direction::Up, "Up"),
            (Direction::Down, "Down"),
            (Direction::Left, "Left"),
            (Direction::Right, "Right"),
        ] {
            let key = binding_map.get(&ControllerAction::Stick { stick: Stick::Right, direction: dir }).cloned().unwrap_or_default();
            let is_capturing = capture_action.as_ref().map(|a| {
                if let ControllerAction::Stick { stick, direction } = a {
                    *stick == Stick::Right && *direction == dir
                } else { false }
            }).unwrap_or(false);

            if ui.button(format!("R-{}: {}", name, if is_capturing { "press key..." } else { &key })).clicked() {
                self.send(ClientCommand::StartBindingCapture {
                    slot: self.selected_slot,
                    action: ControllerAction::Stick { stick: Stick::Right, direction: dir },
                });
            }
        }

        // Cancel capture if in progress
        if capture_action.is_some() {
            ui.add_space(8.0);
            if ui.button("Cancel Capture").clicked() {
                self.send(ClientCommand::CancelBindingCapture);
            }
        }
    }

    fn draw_slot_controls(&mut self, ui: &mut egui::Ui, slot: &SlotStatus) {
        let device_text = slot
            .device_id
            .as_ref()
            .map(DeviceId::to_string)
            .unwrap_or_else(|| "No keyboard assigned".to_owned());
        ui.label(format!("{}: {}", slot.slot, lifecycle_text(slot.lifecycle)));
        ui.label(RichText::new(device_text).small().color(Color32::GRAY));

        ui.horizontal(|ui| {
            if ui
                .add_enabled(slot.device_id.is_some(), egui::Button::new("Clear slot"))
                .clicked()
            {
                self.send(ClientCommand::UnassignSlot { slot: slot.slot });
            }
            if ui.button("Save profile").clicked() {
                self.send(ClientCommand::SaveProfile);
            }
        });
        if let Some(error) = &slot.last_error {
            ui.colored_label(Color32::from_rgb(210, 86, 78), error);
        }

        ui.add_space(10.0);
        ui.label(RichText::new("Test Output").strong());
        ui.horizontal_wrapped(|ui| {
            for button in [
                GamepadButton::South,
                GamepadButton::East,
                GamepadButton::West,
                GamepadButton::North,
                GamepadButton::LeftShoulder,
                GamepadButton::RightShoulder,
                GamepadButton::Start,
                GamepadButton::Select,
            ] {
                if ui.button(button.to_string()).clicked() {
                    let pressed = !slot.state.buttons.get(&button).copied().unwrap_or(false);
                    self.send(ClientCommand::InjectTestAction {
                        slot: slot.slot,
                        action: ControllerAction::Button(button),
                        pressed,
                    });
                }
            }
        });
    }

    fn draw_main_panel(&mut self, ui: &mut egui::Ui) {
        let snapshot = self.snapshot.as_ref();
        let slot = snapshot.and_then(|snapshot| {
            snapshot
                .slots
                .iter()
                .find(|slot| slot.slot == self.selected_slot)
        });

        ui.horizontal(|ui| {
            ui.heading(format!("{}", self.selected_slot));
            if let Some(slot) = slot {
                let color = match slot.lifecycle {
                    SlotLifecycle::Empty => Color32::GRAY,
                    SlotLifecycle::Bound => Color32::from_rgb(72, 132, 200),
                    SlotLifecycle::Locked => Color32::from_rgb(210, 134, 54),
                    SlotLifecycle::Active => Color32::from_rgb(54, 172, 100),
                    SlotLifecycle::Paused => Color32::from_rgb(160, 130, 48),
                    SlotLifecycle::Error => Color32::from_rgb(210, 76, 68),
                };
                ui.colored_label(color, lifecycle_text(slot.lifecycle));
                if slot.controller_ready {
                    ui.label("Virtual controller ready");
                } else {
                    ui.label("Virtual controller not ready");
                }
            }
        });
        ui.add_space(8.0);

        let available = ui.available_size();
        let (rect, _) = ui.allocate_exact_size(available, Sense::hover());
        let state = slot.map(|slot| &slot.state).cloned().unwrap_or_default();
        draw_controller(ui.painter(), rect, &state);
    }

    fn draw_bottom_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Event Log").strong());
            if let Some(snapshot) = &self.snapshot {
                let connected = snapshot
                    .devices
                    .iter()
                    .filter(|device| device.connected)
                    .count();
                ui.separator();
                ui.label(format!("{connected} keyboard(s) connected"));
            }
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .show(ui, |ui| {
                if let Some(snapshot) = &self.snapshot {
                    for entry in snapshot.event_log.iter().rev() {
                        let color = match entry.level {
                            LogLevel::Info => Color32::GRAY,
                            LogLevel::Warning => Color32::from_rgb(210, 145, 66),
                            LogLevel::Error => Color32::from_rgb(220, 80, 72),
                        };
                        ui.colored_label(
                            color,
                            format!("{:>6} ms  {}", entry.millis, entry.message),
                        );
                    }
                }
            });
    }
}

fn warning_band(ui: &mut egui::Ui, color: Color32, label: &str, text: &str) {
    egui::Frame::NONE
        .fill(color.linear_multiply(0.18))
        .stroke(Stroke::new(1.0, color.linear_multiply(0.6)))
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.colored_label(color, RichText::new(label).strong());
            ui.label(text);
        });
}

fn lifecycle_text(lifecycle: SlotLifecycle) -> &'static str {
    match lifecycle {
        SlotLifecycle::Empty => "Empty",
        SlotLifecycle::Bound => "Bound",
        SlotLifecycle::Locked => "Locked",
        SlotLifecycle::Active => "Active",
        SlotLifecycle::Paused => "Paused",
        SlotLifecycle::Error => "Error",
    }
}

fn draw_controller(painter: &Painter, rect: Rect, state: &ControllerState) {
    let width = rect.width().min(rect.height() * 1.62);
    let height = width / 1.62;
    let center = rect.center();
    let body = Rect::from_center_size(center, Vec2::new(width, height));

    let base = Color32::from_rgb(38, 42, 48);
    let panel = Color32::from_rgb(54, 60, 68);
    let active = Color32::from_rgb(54, 190, 112);
    let stroke = Stroke::new(2.0, Color32::from_rgb(95, 104, 116));
    let guide_active = Color32::from_rgb(16, 124, 16);

    // Main body - Xbox controller outline
    painter.rect(
        body,
        24.0,
        base,
        Stroke::new(2.0, Color32::from_rgb(80, 88, 96)),
        StrokeKind::Outside,
    );

    // Left and right grip areas
    let left_grip = Rect::from_center_size(
        Pos2::new(body.left() + width * 0.16, body.center().y + height * 0.16),
        Vec2::new(width * 0.25, height * 0.58),
    );
    let right_grip = Rect::from_center_size(
        Pos2::new(body.right() - width * 0.16, body.center().y + height * 0.16),
        Vec2::new(width * 0.25, height * 0.58),
    );
    painter.rect(left_grip, 32.0, base, stroke, StrokeKind::Outside);
    painter.rect(right_grip, 32.0, base, stroke, StrokeKind::Outside);

    // Left stick
    draw_stick(
        painter,
        Pos2::new(body.left() + width * 0.31, body.top() + height * 0.48),
        width * 0.075,
        state.axes.left_x,
        state.axes.left_y,
        active,
    );

    // Right stick
    draw_stick(
        painter,
        Pos2::new(body.left() + width * 0.64, body.top() + height * 0.63),
        width * 0.075,
        state.axes.right_x,
        state.axes.right_y,
        active,
    );

    // D-pad
    draw_dpad(
        painter,
        Pos2::new(body.left() + width * 0.43, body.top() + height * 0.66),
        width * 0.052,
        state,
        panel,
        active,
    );

    // Face buttons - Y, B, A, X
    let face_center = Pos2::new(body.left() + width * 0.72, body.top() + height * 0.42);
    draw_button(painter, face_center + Vec2::new(0.0, -height * 0.09), "Y", state, GamepadButton::North, active);
    draw_button(painter, face_center + Vec2::new(width * 0.06, 0.0), "B", state, GamepadButton::East, active);
    draw_button(painter, face_center + Vec2::new(0.0, height * 0.09), "A", state, GamepadButton::South, active);
    draw_button(painter, face_center + Vec2::new(-width * 0.06, 0.0), "X", state, GamepadButton::West, active);

    // Menu buttons
    draw_small_button(
        painter,
        Pos2::new(body.center().x - width * 0.07, body.top() + height * 0.45),
        "Back",
        state,
        GamepadButton::Select,
        active,
    );
    draw_small_button(
        painter,
        Pos2::new(body.center().x + width * 0.07, body.top() + height * 0.45),
        "Start",
        state,
        GamepadButton::Start,
        active,
    );
    draw_small_button(
        painter,
        Pos2::new(body.center().x, body.top() + height * 0.35),
        "Guide",
        state,
        GamepadButton::Guide,
        guide_active,
    );

    // Shoulder buttons
    draw_shoulder(
        painter,
        Rect::from_center_size(
            Pos2::new(body.left() + width * 0.28, body.top() - height * 0.02),
            Vec2::new(width * 0.26, height * 0.12),
        ),
        "LB",
        state.buttons.get(&GamepadButton::LeftShoulder).copied().unwrap_or(false),
        active,
    );
    draw_shoulder(
        painter,
        Rect::from_center_size(
            Pos2::new(body.right() - width * 0.28, body.top() - height * 0.02),
            Vec2::new(width * 0.26, height * 0.12),
        ),
        "RB",
        state.buttons.get(&GamepadButton::RightShoulder).copied().unwrap_or(false),
        active,
    );

    // Triggers
    draw_trigger_bar(
        painter,
        Rect::from_center_size(
            Pos2::new(body.left() + width * 0.28, body.top() - height * 0.16),
            Vec2::new(width * 0.23, height * 0.065),
        ),
        "LT",
        state.axes.left_trigger,
        active,
    );
    draw_trigger_bar(
        painter,
        Rect::from_center_size(
            Pos2::new(body.right() - width * 0.28, body.top() - height * 0.16),
            Vec2::new(width * 0.23, height * 0.065),
        ),
        "RT",
        state.axes.right_trigger,
        active,
    );
}

fn draw_button(
    painter: &Painter,
    center: Pos2,
    label: &str,
    state: &ControllerState,
    button: GamepadButton,
    active: Color32,
) {
    let pressed = state.buttons.get(&button).copied().unwrap_or(false);
    let fill = if pressed {
        active
    } else {
        Color32::from_rgb(74, 81, 90)
    };
    painter.circle(
        center,
        18.0,
        fill,
        Stroke::new(2.0, Color32::from_rgb(115, 124, 134)),
    );
    painter.text(
        center,
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(16.0),
        Color32::WHITE,
    );
}

fn draw_small_button(
    painter: &Painter,
    center: Pos2,
    label: &str,
    state: &ControllerState,
    button: GamepadButton,
    active: Color32,
) {
    let pressed = state.buttons.get(&button).copied().unwrap_or(false);
    let fill = if pressed {
        active
    } else {
        Color32::from_rgb(70, 76, 84)
    };
    let rect = Rect::from_center_size(center, Vec2::new(52.0, 24.0));
    painter.rect(
        rect,
        8.0,
        fill,
        Stroke::new(1.0, Color32::from_rgb(115, 124, 134)),
        StrokeKind::Outside,
    );
    painter.text(
        center,
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(12.0),
        Color32::WHITE,
    );
}

fn draw_stick(painter: &Painter, center: Pos2, radius: f32, x: i16, y: i16, active: Color32) {
    painter.circle(
        center,
        radius * 1.45,
        Color32::from_rgb(26, 29, 34),
        Stroke::new(2.0, Color32::from_rgb(92, 102, 112)),
    );
    let knob = center
        + Vec2::new(
            x as f32 / 32767.0 * radius * 0.55,
            y as f32 / 32767.0 * radius * 0.55,
        );
    let fill = if x != 0 || y != 0 {
        active
    } else {
        Color32::from_rgb(86, 94, 104)
    };
    painter.circle(
        knob,
        radius,
        fill,
        Stroke::new(2.0, Color32::from_rgb(120, 130, 140)),
    );
}

fn draw_dpad(
    painter: &Painter,
    center: Pos2,
    size: f32,
    state: &ControllerState,
    panel: Color32,
    active: Color32,
) {
    let parts = [
        (Vec2::new(0.0, -size), GamepadButton::DpadUp),
        (Vec2::new(0.0, size), GamepadButton::DpadDown),
        (Vec2::new(-size, 0.0), GamepadButton::DpadLeft),
        (Vec2::new(size, 0.0), GamepadButton::DpadRight),
    ];
    for (offset, button) in parts {
        let rect = Rect::from_center_size(center + offset, Vec2::new(size * 0.9, size * 0.9));
        let pressed = state.buttons.get(&button).copied().unwrap_or(false);
        painter.rect(
            rect,
            5.0,
            if pressed { active } else { panel },
            Stroke::new(1.5, Color32::from_rgb(112, 122, 132)),
            StrokeKind::Outside,
        );
    }
}

fn draw_shoulder(painter: &Painter, rect: Rect, label: &str, pressed: bool, active: Color32) {
    painter.rect(
        rect,
        12.0,
        if pressed {
            active
        } else {
            Color32::from_rgb(62, 68, 76)
        },
        Stroke::new(1.5, Color32::from_rgb(112, 122, 132)),
        StrokeKind::Outside,
    );
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(13.0),
        Color32::WHITE,
    );
}

fn draw_trigger_bar(painter: &Painter, rect: Rect, label: &str, value: u8, active: Color32) {
    painter.rect(
        rect,
        8.0,
        Color32::from_rgb(44, 49, 56),
        Stroke::new(1.5, Color32::from_rgb(112, 122, 132)),
        StrokeKind::Outside,
    );
    if value > 0 {
        let filled = Rect::from_min_size(
            rect.min,
            Vec2::new(rect.width() * value as f32 / 255.0, rect.height()),
        );
        painter.rect_filled(filled, 8.0, active);
    }
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(12.0),
        Color32::WHITE,
    );
}