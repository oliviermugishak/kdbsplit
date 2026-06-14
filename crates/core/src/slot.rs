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
        if pressed {
            self.held_actions.insert(action);
        } else {
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

    /// Access held actions for reconciliation (SYN_DROPPED recovery).
    pub fn held_actions(&self) -> &BTreeSet<ControllerAction> {
        &self.held_actions
    }

    /// Reconcile held actions against a kernel evdev key bitmap.
    /// Returns true if any state changed and re-emit is needed.
    pub fn reconcile_from_bitmap(&mut self, bitmap: &[u8], bindings: &[KeyBinding]) -> bool {
        let mut expected: BTreeSet<ControllerAction> = BTreeSet::new();
        for binding in bindings {
            let bit = binding.key.0 as usize;
            if bitmap.get(bit / 8).is_some_and(|byte| byte & (1 << (bit % 8)) != 0) {
                expected.insert(binding.action);
            }
        }

        if self.held_actions == expected {
            return false;
        }

        // Collect diffs first to avoid borrowing issues
        let to_release: Vec<ControllerAction> = self
            .held_actions
            .difference(&expected)
            .copied()
            .collect();
        let to_press: Vec<ControllerAction> = expected
            .difference(&self.held_actions)
            .copied()
            .collect();

        for action in &to_release {
            apply_action_delta(&mut self.status.state, *action, false);
        }
        for action in &to_press {
            apply_action_delta(&mut self.status.state, *action, true);
        }
        self.held_actions = expected;

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
        true
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
            state.axes.left_trigger = if pressed { 255 } else { 0 };
        }
        ControllerAction::Trigger(Trigger::Right) => {
            state.axes.right_trigger = if pressed { 255 } else { 0 };
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
