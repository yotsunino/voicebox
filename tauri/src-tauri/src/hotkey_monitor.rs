//! Global keyboard tap + chord dispatcher.
//!
//! Spawns a dedicated thread running `rdev::listen` (which internally owns a
//! CGEventTap on macOS / `SetWindowsHookEx` on Windows / `XRecord` on Linux).
//! Feeds raw key events into a shared `ChordEngine` and translates engine
//! effects into Tauri events + window show/hide calls.
//!
//! Left- and right-hand modifier variants are deliberately kept distinct.
//! Defaults bind to right-hand Cmd + right-hand Option so that the usual
//! left-hand shortcuts — Cmd+Option+I to open devtools, Cmd+Option+Esc for
//! force-quit, etc. — continue to work untouched. Matches ghost-pepper's
//! default.

use std::sync::{Arc, Mutex};
use std::thread;

use rdev::{listen, EventType, Key};
use tauri::{AppHandle, Emitter, Manager};

use crate::chord_engine::{Bindings, ChordAction, ChordEngine, Effect, InputEvent, KeyChord};
use crate::focus_capture;
use crate::DICTATE_WINDOW_LABEL;

/// Hardcoded Pass 1 defaults. Right-hand Cmd + right-hand Option so left-hand
/// modifier shortcuts pass through unaffected. Replaced in Pass 2 by reading
/// from the server-side `capture_settings` table via a Tauri command the
/// frontend invokes whenever `useCaptureSettings` resolves.
///
/// macOS key mapping in rdev:
/// - `Key::MetaRight` — right Command
/// - `Key::AltGr` — right Option (rdev labels right-option as "AltGr" for
///   Linux-convention symmetry; on macOS it's the physical right-option key)
pub fn default_bindings() -> Bindings {
    let mut b = Bindings::new();
    b.insert(ChordAction::PushToTalk, {
        let mut s = KeyChord::new();
        s.insert(Key::MetaRight);
        s.insert(Key::AltGr);
        s
    });
    b.insert(ChordAction::ToggleToTalk, {
        let mut s = KeyChord::new();
        s.insert(Key::MetaRight);
        s.insert(Key::AltGr);
        s.insert(Key::Space);
        s
    });
    b
}

pub struct HotkeyMonitor {
    engine: Arc<Mutex<ChordEngine>>,
}

impl HotkeyMonitor {
    pub fn spawn(app: AppHandle, bindings: Bindings) -> Self {
        let engine = Arc::new(Mutex::new(ChordEngine::new(bindings)));
        let engine_for_thread = engine.clone();
        let app_for_thread = app.clone();

        thread::spawn(move || {
            // Without this call, rdev's convert() calls TSMGetInputSourceProperty
            // on this background thread, which trips a main-queue assertion on
            // macOS 14+ and traps the whole process (see Narsil/rdev#165 / #147).
            #[cfg(target_os = "macos")]
            rdev::set_is_main_thread(false);

            let result = listen(move |event| {
                let input = match event.event_type {
                    EventType::KeyPress(k) => InputEvent::KeyDown(k),
                    EventType::KeyRelease(k) => InputEvent::KeyUp(k),
                    _ => return,
                };

                let effects = match engine_for_thread.lock() {
                    Ok(mut engine) => engine.handle(input),
                    Err(_) => return,
                };

                for effect in effects {
                    apply_effect(&app_for_thread, effect);
                }
            });

            if let Err(err) = result {
                eprintln!(
                    "HotkeyMonitor: rdev::listen failed ({:?}). Global chord detection is disabled. On macOS, grant Input Monitoring in System Settings → Privacy & Security → Input Monitoring and relaunch.",
                    err
                );
            }
        });

        Self { engine }
    }

    pub fn update_bindings(&self, bindings: Bindings) {
        if let Ok(mut engine) = self.engine.lock() {
            engine.update_bindings(bindings);
        }
    }
}

fn apply_effect(app: &AppHandle, effect: Effect) {
    match effect {
        Effect::StartRecording(_) => {
            // Snapshot focus BEFORE we touch the window — any AppKit
            // reshuffle triggered by set_position / show could in principle
            // steal key focus and poison the reading. In practice those
            // calls leave keyWindow alone, but capturing first is free.
            let focus = focus_capture::capture_focus().ok();

            if let Some(window) = app.get_webview_window(DICTATE_WINDOW_LABEL) {
                // The previous hide-cycle parked the window off-screen and
                // made it click-through — undo both before showing, so the
                // pill lands at top-center and the user can actually click
                // the error pill / stop button.
                //
                // `current_monitor()` returns None when the window is off
                // any display (our hide handler parks it at -10_000, -10_000
                // precisely so it never intercepts clicks), so fall back to
                // the primary monitor for the reposition.
                let monitor = window
                    .current_monitor()
                    .ok()
                    .flatten()
                    .or_else(|| window.primary_monitor().ok().flatten());
                if let Some(monitor) = monitor {
                    let monitor_pos = monitor.position();
                    let monitor_size = monitor.size();
                    if let Ok(win_size) = window.outer_size() {
                        let x = monitor_pos.x
                            + (monitor_size.width as i32 - win_size.width as i32) / 2;
                        let y = monitor_pos.y + (monitor_size.height as f64 * 0.04) as i32;
                        let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
                    }
                }
                let _ = window.set_ignore_cursor_events(false);
                // Deliberately no set_focus() — taking key focus would yank
                // it out of whatever app the user was typing in, which is
                // the opposite of what a dictation overlay should do.
                let _ = window.show();
                let payload = serde_json::json!({ "focus": focus });
                let _ = window.emit("dictate:start", payload);
            }
        }
        Effect::StopRecording(_) => {
            if let Some(window) = app.get_webview_window(DICTATE_WINDOW_LABEL) {
                let _ = window.emit("dictate:stop", ());
            }
        }
        Effect::RestartRecording(_) => {
            if let Some(window) = app.get_webview_window(DICTATE_WINDOW_LABEL) {
                let _ = window.emit("dictate:restart", ());
            }
        }
    }
}
