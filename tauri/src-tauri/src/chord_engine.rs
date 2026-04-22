//! Chord state machine for global hotkey detection.
//!
//! Ported from ghost-pepper's `ChordEngine.swift`. The engine is pure — it
//! owns no I/O, just tracks which physical keys are currently pressed and
//! which bound action (push-to-talk vs. toggle-to-talk) is currently active.
//! Feed it `InputEvent`s as keys press/release; it returns the `Effect`s the
//! host should apply (start/stop/restart recording).

use std::collections::{HashMap, HashSet};

use rdev::Key;

/// Semantic action a chord can be bound to. `PushToTalk` = hold chord to
/// record, release to stop. `ToggleToTalk` = press chord to start recording,
/// press again to stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChordAction {
    PushToTalk,
    ToggleToTalk,
}

/// The output of the engine after consuming an input event. Hosts translate
/// these into UI/recorder calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    StartRecording(ChordAction),
    StopRecording(ChordAction),
    /// Emitted when a push-to-talk chord is "upgraded" into the toggle chord
    /// mid-hold — hosts may want to discard the captured audio and restart
    /// so the transition moment isn't in the recording.
    RestartRecording(ChordAction),
}

#[derive(Debug, Clone)]
pub enum InputEvent {
    KeyDown(Key),
    KeyUp(Key),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MatchResult {
    None,
    Prefix,
    Exact(ChordAction),
}

pub type KeyChord = HashSet<Key>;
pub type Bindings = HashMap<ChordAction, KeyChord>;

pub struct ChordEngine {
    bindings: Bindings,
    pressed_keys: KeyChord,
    active_recording_action: Option<ChordAction>,
}

impl ChordEngine {
    pub fn new(bindings: Bindings) -> Self {
        Self {
            bindings,
            pressed_keys: KeyChord::new(),
            active_recording_action: None,
        }
    }

    pub fn update_bindings(&mut self, bindings: Bindings) {
        self.bindings = bindings;
    }

    pub fn handle(&mut self, event: InputEvent) -> Vec<Effect> {
        if !self.update_pressed_keys(&event) {
            return Vec::new();
        }
        self.evaluate_state_transition()
    }

    #[allow(dead_code)] // Used by the chord picker UI in Pass 2 to suspend matching during capture.
    pub fn reset(&mut self) {
        self.pressed_keys.clear();
        self.active_recording_action = None;
    }

    fn update_pressed_keys(&mut self, event: &InputEvent) -> bool {
        match event {
            InputEvent::KeyDown(k) => self.pressed_keys.insert(*k),
            InputEvent::KeyUp(k) => self.pressed_keys.remove(k),
        }
    }

    fn evaluate_state_transition(&mut self) -> Vec<Effect> {
        match self.active_recording_action {
            Some(ChordAction::PushToTalk) => {
                if self.match_result() == MatchResult::Exact(ChordAction::ToggleToTalk) {
                    self.active_recording_action = Some(ChordAction::ToggleToTalk);
                    return vec![Effect::RestartRecording(ChordAction::ToggleToTalk)];
                }

                let still_held = self
                    .bindings
                    .get(&ChordAction::PushToTalk)
                    .map(|chord| chord.is_subset(&self.pressed_keys))
                    .unwrap_or(false);

                if !still_held {
                    self.active_recording_action = None;
                    return vec![Effect::StopRecording(ChordAction::PushToTalk)];
                }
                Vec::new()
            }
            Some(ChordAction::ToggleToTalk) => {
                if self.match_result() == MatchResult::Exact(ChordAction::ToggleToTalk) {
                    self.active_recording_action = None;
                    return vec![Effect::StopRecording(ChordAction::ToggleToTalk)];
                }
                Vec::new()
            }
            None => match self.match_result() {
                MatchResult::Exact(action) => {
                    self.active_recording_action = Some(action);
                    vec![Effect::StartRecording(action)]
                }
                MatchResult::None | MatchResult::Prefix => Vec::new(),
            },
        }
    }

    fn match_result(&self) -> MatchResult {
        if self.pressed_keys.is_empty() {
            return MatchResult::None;
        }

        // Exact match wins even if the pressed set is also a prefix of another
        // binding — matches ghost-pepper's contract.
        for (action, chord) in &self.bindings {
            if self.pressed_keys == *chord {
                return MatchResult::Exact(*action);
            }
        }

        let is_prefix = self
            .bindings
            .values()
            .any(|c| self.pressed_keys.is_subset(c) && self.pressed_keys != *c);

        if is_prefix {
            MatchResult::Prefix
        } else {
            MatchResult::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(keys: &[Key]) -> KeyChord {
        keys.iter().copied().collect()
    }

    fn default_bindings() -> Bindings {
        let mut b = Bindings::new();
        b.insert(ChordAction::PushToTalk, chord(&[Key::MetaLeft, Key::Alt]));
        b.insert(
            ChordAction::ToggleToTalk,
            chord(&[Key::MetaLeft, Key::Alt, Key::Space]),
        );
        b
    }

    #[test]
    fn push_to_talk_starts_on_exact_hold_and_stops_on_release() {
        let mut e = ChordEngine::new(default_bindings());
        assert_eq!(e.handle(InputEvent::KeyDown(Key::MetaLeft)), vec![]);
        assert_eq!(
            e.handle(InputEvent::KeyDown(Key::Alt)),
            vec![Effect::StartRecording(ChordAction::PushToTalk)],
        );
        assert_eq!(
            e.handle(InputEvent::KeyUp(Key::Alt)),
            vec![Effect::StopRecording(ChordAction::PushToTalk)],
        );
    }

    #[test]
    fn toggle_starts_on_exact_and_stops_on_second_exact() {
        let mut e = ChordEngine::new(default_bindings());
        e.handle(InputEvent::KeyDown(Key::MetaLeft));
        e.handle(InputEvent::KeyDown(Key::Alt));
        // At this point PTT is active.
        assert_eq!(e.active_recording_action, Some(ChordAction::PushToTalk));
        assert_eq!(
            e.handle(InputEvent::KeyDown(Key::Space)),
            vec![Effect::RestartRecording(ChordAction::ToggleToTalk)],
        );
        // Releasing cmd/opt must not stop toggle recording.
        assert_eq!(e.handle(InputEvent::KeyUp(Key::MetaLeft)), vec![]);
        assert_eq!(e.handle(InputEvent::KeyUp(Key::Alt)), vec![]);
        assert_eq!(e.handle(InputEvent::KeyUp(Key::Space)), vec![]);

        // Second press of toggle chord stops it.
        e.handle(InputEvent::KeyDown(Key::MetaLeft));
        e.handle(InputEvent::KeyDown(Key::Alt));
        assert_eq!(
            e.handle(InputEvent::KeyDown(Key::Space)),
            vec![Effect::StopRecording(ChordAction::ToggleToTalk)],
        );
    }

    #[test]
    fn toggle_from_idle_starts_immediately_on_full_chord() {
        let mut e = ChordEngine::new(default_bindings());
        e.handle(InputEvent::KeyDown(Key::MetaLeft));
        e.handle(InputEvent::KeyDown(Key::Alt));
        // Drop MetaLeft before Space — we're not in the exact toggle match
        // yet, just prefix. No start for toggle.
        assert_eq!(e.active_recording_action, Some(ChordAction::PushToTalk));
    }
}
