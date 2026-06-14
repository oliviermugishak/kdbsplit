use kbdsplit_shared::{
    ControllerAction, ControllerAxes, ControllerSlot, ControllerState, Direction, GamepadButton,
    KeyBinding, SlotLifecycle, SlotStatus, Stick, Trigger,
};
use std::collections::{BTreeMap, BTreeSet};

const STICK_MAX: i16 = 32767;

#[derive(Debug, Clone)]
pub struct RuntimeSlot {
    pub status: SlotStatus,
    held_actions: BTreeSet<ControllerAction>,
}

impl RuntimeSlot {
    pub fn new(slot: ControllerSlot) -> Self {
        Self {
            status: SlotStatus::empty(slot),
            held_actions: BTreeSet::new(),
        }
    }

    pub fn set_lifecycle(&mut self, lifecycle: SlotLifecycle) {
        self.status.lifecycle = lifecycle;
    }

    pub fn set_bindings(&mut self, bindings: Vec<KeyBinding>) {
        self.status.bindings = bindings;
    }

    pub fn apply_action(&mut self, action: ControllerAction, pressed: bool) {
        if pressed {
            self.held_actions.insert(action);
        } else {
            self.held_actions.remove(&action);
        }
        self.status.state = state_from_actions(&self.held_actions);
        if self.status.device_id.is_some() && self.status.lifecycle != SlotLifecycle::Error {
            self.status.lifecycle = if self.held_actions.is_empty() {
                if self.status.locked {
                    SlotLifecycle::Locked
                } else {
                    SlotLifecycle::Bound
                }
            } else {
                SlotLifecycle::Active
            };
        }
    }

    pub fn clear_inputs(&mut self) {
        self.held_actions.clear();
        self.status.state = ControllerState::default();
    }
}

pub fn state_from_actions(actions: &BTreeSet<ControllerAction>) -> ControllerState {
    let mut buttons: BTreeMap<GamepadButton, bool> = GamepadButton::ALL
        .into_iter()
        .map(|button| (button, false))
        .collect();
    let mut axes = ControllerAxes::default();

    for action in actions {
        match action {
            ControllerAction::Button(button) => {
                buttons.insert(*button, true);
            }
            ControllerAction::Trigger(Trigger::Left) => axes.left_trigger = u8::MAX,
            ControllerAction::Trigger(Trigger::Right) => axes.right_trigger = u8::MAX,
            ControllerAction::Stick { stick, direction } => {
                let (x, y) = match direction {
                    Direction::Up => (0, -STICK_MAX),
                    Direction::Down => (0, STICK_MAX),
                    Direction::Left => (-STICK_MAX, 0),
                    Direction::Right => (STICK_MAX, 0),
                };
                match stick {
                    Stick::Left => {
                        axes.left_x = axes.left_x.saturating_add(x);
                        axes.left_y = axes.left_y.saturating_add(y);
                    }
                    Stick::Right => {
                        axes.right_x = axes.right_x.saturating_add(x);
                        axes.right_y = axes.right_y.saturating_add(y);
                    }
                }
            }
        }
    }

    ControllerState { buttons, axes }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opposing_stick_actions_cancel() {
        let mut actions = BTreeSet::new();
        actions.insert(ControllerAction::Stick {
            stick: Stick::Left,
            direction: Direction::Left,
        });
        actions.insert(ControllerAction::Stick {
            stick: Stick::Left,
            direction: Direction::Right,
        });
        assert_eq!(state_from_actions(&actions).axes.left_x, 0);
    }
}
