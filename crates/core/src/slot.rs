use kbdsplit_shared::{
    ControllerAction, ControllerSlot, ControllerState, Direction,
    KeyBinding, SlotLifecycle, SlotStatus, Stick, Trigger,
};
use std::collections::BTreeSet;

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
        self.held_actions.insert(action);
        if !pressed {
            self.held_actions.remove(&action);
        }
        apply_action_delta(&mut self.status.state, action, pressed);
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

    pub fn reset_state_from_actions(&mut self) {
        self.status.state = state_from_actions(&self.held_actions);
    }

    pub fn clear_inputs(&mut self) {
        self.held_actions.clear();
        self.status.state = ControllerState::default();
    }
}

fn apply_action_delta(state: &mut ControllerState, action: ControllerAction, pressed: bool) {
    match action {
        ControllerAction::Button(btn) => {
            state.set_button(btn, pressed);
        }
        ControllerAction::Trigger(Trigger::Left) => {
            let delta: i8 = if pressed { 1 } else { -1 };
            state.axes.left_trigger = state.axes.left_trigger.wrapping_add_signed(delta);
        }
        ControllerAction::Trigger(Trigger::Right) => {
            let delta: i8 = if pressed { 1 } else { -1 };
            state.axes.right_trigger = state.axes.right_trigger.wrapping_add_signed(delta);
        }
        ControllerAction::Stick { stick, direction } => {
            let delta: i16 = if pressed { 1 } else { -1 };
            let (dx, dy) = match direction {
                Direction::Up => (0i16, -STICK_MAX * delta),
                Direction::Down => (0i16, STICK_MAX * delta),
                Direction::Left => (-STICK_MAX * delta, 0i16),
                Direction::Right => (STICK_MAX * delta, 0i16),
            };
            match stick {
                Stick::Left => {
                    state.axes.left_x = state.axes.left_x.saturating_add(dx);
                    state.axes.left_y = state.axes.left_y.saturating_add(dy);
                }
                Stick::Right => {
                    state.axes.right_x = state.axes.right_x.saturating_add(dx);
                    state.axes.right_y = state.axes.right_y.saturating_add(dy);
                }
            }
        }
    }
}

pub fn state_from_actions(actions: &BTreeSet<ControllerAction>) -> ControllerState {
    let mut state = ControllerState::default();
    for action in actions {
        apply_action_delta(&mut state, *action, true);
    }
    state
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
