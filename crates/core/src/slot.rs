use kbdsplit_shared::{
    ControllerAction, ControllerSlot, ControllerState, Direction, KeyBinding, KeyCode,
    SlotLifecycle, SlotStatus, Stick, Trigger,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

const STICK_MAX: i16 = 32767;

#[derive(Debug, Clone)]
pub struct RuntimeSlot {
    pub status: SlotStatus,
    held_keys: BTreeMap<KeyCode, ControllerAction>,
    action_refcount: HashMap<ControllerAction, u32>,
}

impl RuntimeSlot {
    pub fn new(slot: ControllerSlot) -> Self {
        Self {
            status: SlotStatus::empty(slot),
            held_keys: BTreeMap::new(),
            action_refcount: HashMap::new(),
        }
    }

    pub fn set_lifecycle(&mut self, lifecycle: SlotLifecycle) {
        self.status.lifecycle = lifecycle;
    }

    pub fn set_bindings(&mut self, bindings: Vec<KeyBinding>) {
        self.rebind_held_keys(&bindings);
        self.status.bindings = bindings;
    }

    /// Called when a key is pressed. `action` comes from the current binding lookup.
    /// Returns true if the key was not already held.
    pub fn key_down(&mut self, key_code: KeyCode, action: ControllerAction) -> bool {
        if self.held_keys.contains_key(&key_code) {
            return false;
        }
        self.held_keys.insert(key_code, action);
        increment_action(&mut self.action_refcount, &mut self.status.state, action);
        self.update_lifecycle();
        true
    }

    /// Called when a key is released. Uses the action stored at press time,
    /// so it is correct even if bindings changed since the key was pressed.
    /// Returns true if the key was actually held.
    pub fn key_up(&mut self, key_code: KeyCode) -> bool {
        let Some(stored_action) = self.held_keys.remove(&key_code) else {
            return false;
        };
        decrement_action(
            &mut self.action_refcount,
            &mut self.status.state,
            stored_action,
        );
        self.update_lifecycle();
        true
    }

    /// Directly apply an action without key tracking (used by test injection).
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
    /// Works at key level using key_down/key_up for correct refcounting.
    pub fn reconcile_from_bitmap(&mut self, bitmap: &[u8], bindings: &[KeyBinding]) -> bool {
        let mut physical: BTreeSet<KeyCode> = BTreeSet::new();
        for binding in bindings {
            let bit = binding.key.0 as usize;
            if bitmap
                .get(bit / 8)
                .is_some_and(|byte| byte & (1 << (bit % 8)) != 0)
            {
                physical.insert(binding.key);
            }
        }

        let held_set: BTreeSet<KeyCode> = self.held_keys.keys().copied().collect();
        if held_set == physical {
            return false;
        }

        let mut changed = false;

        for key_code in held_set.difference(&physical) {
            changed |= self.key_up(*key_code);
        }

        for key_code in physical.difference(&held_set) {
            if let Some(binding) = bindings.iter().find(|b| &b.key == key_code) {
                changed |= self.key_down(*key_code, binding.action);
            }
        }

        changed
    }

    pub fn clear_inputs(&mut self) {
        self.held_keys.clear();
        self.action_refcount.clear();
        self.status.state = ControllerState::default();
    }

    fn rebind_held_keys(&mut self, new_bindings: &[KeyBinding]) {
        let current: Vec<(KeyCode, ControllerAction)> =
            self.held_keys.iter().map(|(k, a)| (*k, *a)).collect();

        for (key_code, old_action) in &current {
            match new_bindings.iter().find(|b| b.key == *key_code) {
                Some(binding) if binding.action != *old_action => {
                    decrement_action(
                        &mut self.action_refcount,
                        &mut self.status.state,
                        *old_action,
                    );
                    increment_action(
                        &mut self.action_refcount,
                        &mut self.status.state,
                        binding.action,
                    );
                    self.held_keys.insert(*key_code, binding.action);
                }
                None => {
                    self.held_keys.remove(key_code);
                    decrement_action(
                        &mut self.action_refcount,
                        &mut self.status.state,
                        *old_action,
                    );
                }
                _ => {}
            }
        }

        self.update_lifecycle();
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

fn increment_action(
    refcount: &mut HashMap<ControllerAction, u32>,
    state: &mut ControllerState,
    action: ControllerAction,
) {
    let rc = refcount.entry(action).or_insert(0);
    if *rc == 0 {
        apply_action_delta(state, action, true);
    }
    *rc += 1;
}

fn decrement_action(
    refcount: &mut HashMap<ControllerAction, u32>,
    state: &mut ControllerState,
    action: ControllerAction,
) {
    if let std::collections::hash_map::Entry::Occupied(mut entry) = refcount.entry(action) {
        let rc = entry.get_mut();
        *rc = rc.saturating_sub(1);
        if *rc == 0 {
            entry.remove();
            apply_action_delta(state, action, false);
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
