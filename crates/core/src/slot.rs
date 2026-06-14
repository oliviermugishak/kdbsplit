use kbdsplit_shared::{
    ControllerAction, ControllerSlot, ControllerState, Direction,
    KeyBinding, KeyCode, SlotLifecycle, SlotStatus, Stick, Trigger,
};
use std::collections::{BTreeSet, HashMap};

const STICK_MAX: i16 = 32767;

#[derive(Debug, Clone)]
pub struct RuntimeSlot {
    pub status: SlotStatus,
    held_keys: BTreeSet<KeyCode>,
    action_refcount: HashMap<ControllerAction, u32>,
}

impl RuntimeSlot {
    pub fn new(slot: ControllerSlot) -> Self {
        Self {
            status: SlotStatus::empty(slot),
            held_keys: BTreeSet::new(),
            action_refcount: HashMap::new(),
        }
    }

    pub fn set_lifecycle(&mut self, lifecycle: SlotLifecycle) {
        self.status.lifecycle = lifecycle;
    }

    pub fn set_bindings(&mut self, bindings: Vec<KeyBinding>) {
        self.status.bindings = bindings;
    }

    /// Apply a key press/release with per-key refcounting.
    /// Returns true if the key state actually changed.
    pub fn apply_key(&mut self, key_code: KeyCode, action: ControllerAction, pressed: bool) -> bool {
        if pressed {
            if !self.held_keys.insert(key_code) {
                return false;
            }
            let rc = self.action_refcount.entry(action).or_insert(0);
            let was_inactive = *rc == 0;
            *rc += 1;
            if was_inactive {
                apply_action_delta(&mut self.status.state, action, true);
            }
        } else {
            if !self.held_keys.remove(&key_code) {
                return false;
            }
            if let std::collections::hash_map::Entry::Occupied(mut entry) =
                self.action_refcount.entry(action)
            {
                let rc = entry.get_mut();
                *rc = rc.saturating_sub(1);
                if *rc == 0 {
                    entry.remove();
                    apply_action_delta(&mut self.status.state, action, false);
                }
            }
        }
        self.update_lifecycle();
        true
    }

    /// Directly apply an action without key tracking (used by test injection).
    /// This does not update refcounts — use only for debug purposes.
    pub fn apply_action(&mut self, action: ControllerAction, pressed: bool) {
        apply_action_delta(&mut self.status.state, action, pressed);
        if self.status.device_id.is_some() && self.status.lifecycle != SlotLifecycle::Error {
            if pressed {
                self.status.lifecycle = SlotLifecycle::Active;
            } else if self.held_keys.is_empty() && self.status.state == ControllerState::default() {
                self.status.lifecycle = if self.status.locked {
                    SlotLifecycle::Locked
                } else {
                    SlotLifecycle::Bound
                };
            }
        }
    }

    /// Reconcile held keys against a kernel evdev key bitmap.
    /// Works at key level: diffs physical keys against our held_keys,
    /// then processes each diff through apply_key for correct refcounting.
    /// Returns true if any state changed and re-emit is needed.
    pub fn reconcile_from_bitmap(&mut self, bitmap: &[u8], bindings: &[KeyBinding]) -> bool {
        let mut physical: BTreeSet<KeyCode> = BTreeSet::new();
        for binding in bindings {
            let bit = binding.key.0 as usize;
            if bitmap.get(bit / 8).is_some_and(|byte| byte & (1 << (bit % 8)) != 0) {
                physical.insert(binding.key);
            }
        }

        if self.held_keys == physical {
            return false;
        }

        // Collect diffs first to avoid borrow conflicts with self.apply_key
        let to_release: Vec<(KeyCode, ControllerAction)> = self
            .held_keys
            .difference(&physical)
            .filter_map(|key_code| {
                bindings
                    .iter()
                    .find(|b| &b.key == key_code)
                    .map(|b| (b.key, b.action))
            })
            .collect();
        let to_press: Vec<(KeyCode, ControllerAction)> = physical
            .difference(&self.held_keys)
            .filter_map(|key_code| {
                bindings
                    .iter()
                    .find(|b| &b.key == key_code)
                    .map(|b| (b.key, b.action))
            })
            .collect();

        let mut changed = false;
        for (key_code, action) in &to_release {
            changed |= self.apply_key(*key_code, *action, false);
        }
        for (key_code, action) in &to_press {
            changed |= self.apply_key(*key_code, *action, true);
        }
        changed
    }

    pub fn clear_inputs(&mut self) {
        self.held_keys.clear();
        self.action_refcount.clear();
        self.status.state = ControllerState::default();
    }

    fn update_lifecycle(&mut self) {
        if self.status.device_id.is_some() && self.status.lifecycle != SlotLifecycle::Error {
            self.status.lifecycle = if self.held_keys.is_empty() {
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
