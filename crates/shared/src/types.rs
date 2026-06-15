use serde::{Deserialize, Serialize};
use std::fmt;

pub const MAX_SLOTS: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DeviceId(pub String);

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceFingerprint {
    pub bustype: Option<u16>,
    pub vendor: Option<u16>,
    pub product: Option<u16>,
    pub version: Option<u16>,
    pub name: String,
    pub phys: Option<String>,
    pub uniq: Option<String>,
}

impl DeviceFingerprint {
    pub fn stable_id(&self) -> DeviceId {
        let vendor = self.vendor.map(hex4).unwrap_or_else(|| "none".to_owned());
        let product = self.product.map(hex4).unwrap_or_else(|| "none".to_owned());
        let bus = self.bustype.map(hex4).unwrap_or_else(|| "none".to_owned());
        let phys = self.phys.as_deref().unwrap_or("no-phys");
        let uniq = self.uniq.as_deref().unwrap_or("no-uniq");
        DeviceId(format!(
            "{bus}:{vendor}:{product}:{}:{}:{}",
            slug(&self.name),
            slug(phys),
            slug(uniq)
        ))
    }
}

fn hex4(value: u16) -> String {
    format!("{value:04x}")
}

fn slug(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyboardDevice {
    pub id: DeviceId,
    pub path: String,
    pub name: String,
    pub fingerprint: DeviceFingerprint,
    pub is_internal: bool,
    pub connected: bool,
    pub assigned_slot: Option<ControllerSlot>,
    pub locked: bool,
    pub can_grab: bool,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ControllerSlot {
    One,
    Two,
    Three,
    Four,
}

impl ControllerSlot {
    pub const ALL: [ControllerSlot; MAX_SLOTS] = [
        ControllerSlot::One,
        ControllerSlot::Two,
        ControllerSlot::Three,
        ControllerSlot::Four,
    ];

    pub fn index(self) -> usize {
        match self {
            ControllerSlot::One => 0,
            ControllerSlot::Two => 1,
            ControllerSlot::Three => 2,
            ControllerSlot::Four => 3,
        }
    }

    pub fn number(self) -> u8 {
        self.index() as u8 + 1
    }

    pub fn from_number(number: u8) -> Option<Self> {
        match number {
            1 => Some(Self::One),
            2 => Some(Self::Two),
            3 => Some(Self::Three),
            4 => Some(Self::Four),
            _ => None,
        }
    }
}

impl fmt::Display for ControllerSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Slot {}", self.number())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SlotLifecycle {
    Empty,
    Bound,
    Locked,
    Active,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum GamepadButton {
    South,
    East,
    West,
    North,
    LeftShoulder,
    RightShoulder,
    Select,
    Start,
    Guide,
    LeftThumb,
    RightThumb,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
}

impl GamepadButton {
    pub const ALL: [GamepadButton; 15] = [
        GamepadButton::South,
        GamepadButton::East,
        GamepadButton::West,
        GamepadButton::North,
        GamepadButton::LeftShoulder,
        GamepadButton::RightShoulder,
        GamepadButton::Select,
        GamepadButton::Start,
        GamepadButton::Guide,
        GamepadButton::LeftThumb,
        GamepadButton::RightThumb,
        GamepadButton::DpadUp,
        GamepadButton::DpadDown,
        GamepadButton::DpadLeft,
        GamepadButton::DpadRight,
    ];

    pub fn bit(self) -> u16 {
        1 << (self as u8)
    }
}

impl fmt::Display for GamepadButton {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            GamepadButton::South => "A",
            GamepadButton::East => "B",
            GamepadButton::West => "X",
            GamepadButton::North => "Y",
            GamepadButton::LeftShoulder => "LB",
            GamepadButton::RightShoulder => "RB",
            GamepadButton::Select => "Back",
            GamepadButton::Start => "Start",
            GamepadButton::Guide => "Guide",
            GamepadButton::LeftThumb => "LS",
            GamepadButton::RightThumb => "RS",
            GamepadButton::DpadUp => "D-Up",
            GamepadButton::DpadDown => "D-Down",
            GamepadButton::DpadLeft => "D-Left",
            GamepadButton::DpadRight => "D-Right",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Stick {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Trigger {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ControllerAction {
    Button(GamepadButton),
    Stick { stick: Stick, direction: Direction },
    Trigger(Trigger),
}

impl fmt::Display for ControllerAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ControllerAction::Button(button) => write!(f, "{button}"),
            ControllerAction::Stick { stick, direction } => {
                let stick = match stick {
                    Stick::Left => "Left stick",
                    Stick::Right => "Right stick",
                };
                let direction = match direction {
                    Direction::Up => "up",
                    Direction::Down => "down",
                    Direction::Left => "left",
                    Direction::Right => "right",
                };
                write!(f, "{stick} {direction}")
            }
            ControllerAction::Trigger(trigger) => f.write_str(match trigger {
                Trigger::Left => "Left trigger",
                Trigger::Right => "Right trigger",
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct KeyCode(pub u16);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyBinding {
    pub key: KeyCode,
    pub label: String,
    pub action: ControllerAction,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControllerAxes {
    pub left_x: i16,
    pub left_y: i16,
    pub right_x: i16,
    pub right_y: i16,
    pub left_trigger: u8,
    pub right_trigger: u8,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControllerState {
    pub buttons: u16,
    pub axes: ControllerAxes,
}

impl ControllerState {
    pub fn button_pressed(&self, button: GamepadButton) -> bool {
        self.buttons & button.bit() != 0
    }

    pub fn set_button(&mut self, button: GamepadButton, pressed: bool) {
        if pressed {
            self.buttons |= button.bit();
        } else {
            self.buttons &= !button.bit();
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotStatus {
    pub slot: ControllerSlot,
    pub lifecycle: SlotLifecycle,
    pub device_id: Option<DeviceId>,
    pub locked: bool,
    pub controller_ready: bool,
    pub state: ControllerState,
    pub last_error: Option<String>,
    pub bindings: Vec<KeyBinding>,
}

impl SlotStatus {
    pub fn empty(slot: ControllerSlot) -> Self {
        Self {
            slot,
            lifecycle: SlotLifecycle::Empty,
            device_id: None,
            locked: false,
            controller_ready: false,
            state: ControllerState::default(),
            last_error: None,
            bindings: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSnapshot {
    pub devices: Vec<KeyboardDevice>,
    pub slots: Vec<SlotStatus>,
    pub active_profile: String,
    pub profile_names: Vec<String>,
    pub permission_warnings: Vec<String>,
    pub event_log: Vec<EventLogEntry>,
    pub capture_status: CaptureStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventLogEntry {
    pub millis: u128,
    pub level: LogLevel,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogLevel {
    Info,
    Warning,
    Error,
}

/// A request to capture the next key press for a specific controller action
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindingCapture {
    pub slot: ControllerSlot,
    pub action: ControllerAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaptureStatus {
    Waiting {
        slot: ControllerSlot,
        action: ControllerAction,
    },
    None,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_fingerprint_stable_id() {
        let fp = DeviceFingerprint {
            bustype: Some(0x0003),
            vendor: Some(0x045e),
            product: Some(0x028e),
            version: Some(0x0110),
            name: "Microsoft Xbox 360 Controller".to_owned(),
            phys: Some("usb-0000:00:14.0-1/input0".to_owned()),
            uniq: Some("A1B2C3D4".to_owned()),
        };
        let id = fp.stable_id();
        assert_eq!(
            id.0,
            "0003:045e:028e:microsoft-xbox-360-controller:usb-0000-00-14-0-1-input0:a1b2c3d4"
        );
    }

    #[test]
    fn test_controller_slot_all() {
        assert_eq!(ControllerSlot::ALL.len(), 4);
        assert_eq!(
            ControllerSlot::ALL,
            [
                ControllerSlot::One,
                ControllerSlot::Two,
                ControllerSlot::Three,
                ControllerSlot::Four
            ]
        );
    }

    #[test]
    fn test_controller_slot_from_number() {
        assert_eq!(ControllerSlot::from_number(1), Some(ControllerSlot::One));
        assert_eq!(ControllerSlot::from_number(2), Some(ControllerSlot::Two));
        assert_eq!(ControllerSlot::from_number(3), Some(ControllerSlot::Three));
        assert_eq!(ControllerSlot::from_number(4), Some(ControllerSlot::Four));
        assert_eq!(ControllerSlot::from_number(0), None);
        assert_eq!(ControllerSlot::from_number(5), None);
    }

    #[test]
    fn test_controller_state_default() {
        let state = ControllerState::default();
        assert_eq!(state.buttons, 0);
        for button in GamepadButton::ALL {
            assert!(!state.button_pressed(button));
        }
        assert_eq!(state.axes.left_x, 0);
        assert_eq!(state.axes.left_y, 0);
        assert_eq!(state.axes.right_x, 0);
        assert_eq!(state.axes.right_y, 0);
        assert_eq!(state.axes.left_trigger, 0);
        assert_eq!(state.axes.right_trigger, 0);
    }

    #[test]
    fn test_key_code_ordering() {
        assert!(KeyCode(10) > KeyCode(5));
        assert!(KeyCode(3) < KeyCode(8));
        assert_eq!(KeyCode(7), KeyCode(7));
    }
}
