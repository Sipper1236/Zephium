use gtk::prelude::*;
use std::sync::{Arc, Mutex};
use tauri::{Manager, WebviewWindow};

use crate::linux_shortcut::LinuxLauncherShortcut;
use crate::linux_shortcut_portal::{ActivationContext, ActivationTarget, GlobalRegistration};
use crate::linux_x11_shortcut::X11Shortcut;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DisplayBackend {
    Wayland,
    X11,
    Unsupported,
}

pub(crate) struct LinuxGlobalShortcuts {
    backend: DisplayBackend,
    state: Mutex<ShortcutState>,
    target: Arc<ActivationTarget>,
}

struct ShortcutState {
    shortcut: Option<LinuxLauncherShortcut>,
    recording: bool,
    mapped: bool,
    registration: GlobalRegistration,
    worker_target: Option<Arc<ActivationTarget>>,
    x11: Option<X11Shortcut>,
}

impl LinuxGlobalShortcuts {
    pub(crate) fn snapshot(&self) -> (Option<LinuxLauncherShortcut>, bool) {
        let state = lock_recover(&self.state);
        (state.shortcut, state.registration.is_live())
    }

    pub(crate) fn editable(&self) -> bool {
        self.backend != DisplayBackend::Unsupported && self.target.is_active()
    }

    pub(crate) fn install(
        window: &WebviewWindow,
        shortcut: Option<LinuxLauncherShortcut>,
        registration: GlobalRegistration,
        activation: impl Fn(ActivationContext) + Send + Sync + 'static,
    ) -> Self {
        let state = Self {
            backend: detect_display_backend(window),
            state: Mutex::new(ShortcutState {
                shortcut,
                recording: false,
                mapped: false,
                registration,
                worker_target: None,
                x11: None,
            }),
            target: Arc::new(ActivationTarget::new(activation)),
        };
        state.start_worker(&mut lock_recover(&state.state), false);
        state
    }

    pub(crate) fn start_after_main_mapped(&self) {
        let mut state = lock_recover(&self.state);
        state.mapped = true;
        if state.worker_target.is_none() && !state.recording {
            self.start_worker(&mut state, false);
        }
    }

    pub(crate) fn set_shortcut(
        &self,
        shortcut: LinuxLauncherShortcut,
        committed: impl FnOnce(),
    ) -> bool {
        if !self.editable() {
            return false;
        }
        // Core X11 grabs deliberately exclude text keys until the adapter can
        // prove layout-safe resolution. A portal may bind these natively.
        if self.backend == DisplayBackend::X11
            && matches!(
                shortcut.key(),
                crate::linux_shortcut::LinuxLauncherKey::AsciiAlphanumeric(_)
            )
        {
            return false;
        }
        self.target.set_suppressed(true);
        let mut state = lock_recover(&self.state);
        let previous = state.shortcut;
        self.stop_worker(&mut state);
        state.recording = false;
        state.shortcut = Some(shortcut);
        let applied = self.start_worker(&mut state, true);
        if !applied {
            self.stop_worker(&mut state);
            state.shortcut = previous;
            self.start_worker(&mut state, false);
        } else {
            // Keep the native selection, focused table and persisted setting
            // ordered even if two IPC calls arrive concurrently.
            committed();
        }
        self.target.set_suppressed(false);
        applied
    }

    pub(crate) fn recording(&self, active: bool, changed: impl FnOnce()) {
        let mut state = lock_recover(&self.state);
        // Order focused-table suppression with edits of the native binding.
        changed();
        if state.recording == active || !self.target.is_active() {
            return;
        }
        self.target.set_suppressed(active);
        state.recording = active;
        if active {
            // Release the grab/session so recording the old chord reaches GTK.
            self.stop_worker(&mut state);
        } else {
            self.start_worker(&mut state, false);
        }
    }

    pub(crate) fn shutdown(&self) {
        self.target.stop();
        self.stop_worker(&mut lock_recover(&self.state));
    }

    fn stop_worker(&self, state: &mut ShortcutState) {
        if let Some(target) = state.worker_target.take() {
            target.stop();
        }
        state.registration = state.registration.next_generation();
        if let Some(shortcut) = state.x11.take() {
            shortcut.shutdown();
        }
    }

    fn start_worker(&self, state: &mut ShortcutState, checked: bool) -> bool {
        if !self.target.is_active() || state.recording {
            return false;
        }
        let Some(shortcut) = state.shortcut else {
            return false;
        };
        let root = Arc::downgrade(&self.target);
        let target = Arc::new(ActivationTarget::new(move |context| {
            if let Some(root) = root.upgrade() {
                root.activate(context);
            }
        }));
        match self.backend {
            DisplayBackend::X11 => {
                match X11Shortcut::spawn(
                    shortcut,
                    Arc::downgrade(&target),
                    state.registration.clone(),
                ) {
                    Ok(worker) => {
                        let accepted = !checked
                            || worker.wait_registered(std::time::Duration::from_millis(500));
                        state.worker_target = Some(target);
                        state.x11 = Some(worker);
                        accepted
                    }
                    Err(error) => {
                        crate::write_diagnostic(format_args!(
                            "global shortcut unavailable on X11: {error}"
                        ));
                        false
                    }
                }
            }
            DisplayBackend::Wayland => {
                if !state.mapped {
                    // Configuration is accepted; registration starts only after
                    // the main window maps. The snapshot stays unregistered.
                    return true;
                }
                let weak = Arc::downgrade(&target);
                let registration = state.registration.clone();
                match std::thread::Builder::new()
                    .name("zephium-wayland-shortcut".to_owned())
                    .spawn(move || {
                        if let Err(error) = crate::linux_shortcut_portal::run(weak, registration, shortcut) {
                            if !error.is_stopped() {
                                crate::write_diagnostic(format_args!("global shortcut: Wayland portal unavailable; focused-window fallback remains active: {error}"));
                            }
                        }
                    }) {
                    Ok(_) => {
                        state.worker_target = Some(target);
                        true
                    }
                    Err(error) => {
                        crate::write_diagnostic(format_args!("global shortcut: could not start Wayland portal worker: {error}"));
                        false
                    }
                }
            }
            DisplayBackend::Unsupported => false,
        }
    }
}

pub(crate) fn main_window_mapped(window: &WebviewWindow) {
    if let Some(state) = window.app_handle().try_state::<LinuxGlobalShortcuts>() {
        state.start_after_main_mapped();
    }
}

pub(crate) fn shutdown(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<LinuxGlobalShortcuts>() {
        state.shutdown();
    }
}

fn detect_display_backend(window: &WebviewWindow) -> DisplayBackend {
    let Ok(gtk_window) = window.gtk_window() else {
        return DisplayBackend::Unsupported;
    };
    let display = gtk_window.display();
    classify_display_type(display.type_().name())
}

fn classify_display_type(type_name: &str) -> DisplayBackend {
    match type_name {
        "GdkWaylandDisplay" => DisplayBackend::Wayland,
        "GdkX11Display" => DisplayBackend::X11,
        _ => DisplayBackend::Unsupported,
    }
}

fn lock_recover<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wayland_configuration_is_pending_until_native_portal_registration() {
        let selected = LinuxLauncherShortcut::parse("Ctrl+Shift+F5").expect("shortcut");
        let state = LinuxGlobalShortcuts {
            backend: DisplayBackend::Wayland,
            state: Mutex::new(ShortcutState {
                shortcut: None,
                recording: false,
                mapped: false,
                registration: GlobalRegistration::default(),
                worker_target: None,
                x11: None,
            }),
            target: Arc::new(ActivationTarget::new(|_| {})),
        };
        let committed = std::cell::Cell::new(false);
        assert!(state.set_shortcut(selected, || committed.set(true)));
        assert!(committed.get());
        assert_eq!(state.snapshot(), (Some(selected), false));
        assert!(state.editable());
        assert!(lock_recover(&state.state).worker_target.is_none());
        state.shutdown();
        assert!(!state.editable());
        assert!(!state.set_shortcut(selected, || panic!("shutdown committed")));
    }

    #[test]
    #[ignore = "Requires an isolated X11 server (xvfb-run)"]
    fn x11_editing_restores_collision_and_releases_grabs_while_recording() {
        use std::time::Duration;
        let first = LinuxLauncherShortcut::parse("Ctrl+Shift+F5").expect("first");
        let next = LinuxLauncherShortcut::parse("Ctrl+Shift+F6").expect("next");
        let collision = LinuxLauncherShortcut::parse("Ctrl+Shift+F7").expect("collision");
        let state = LinuxGlobalShortcuts {
            backend: DisplayBackend::X11,
            state: Mutex::new(ShortcutState {
                shortcut: Some(first),
                recording: false,
                mapped: true,
                registration: GlobalRegistration::default(),
                worker_target: None,
                x11: None,
            }),
            target: Arc::new(ActivationTarget::new(|_| {})),
        };
        assert!(state.set_shortcut(first, || {}));
        assert!(state.snapshot().1);
        assert!(state.set_shortcut(next, || {}));
        assert_eq!(state.snapshot(), (Some(next), true));
        let owner = Arc::new(ActivationTarget::new(|_| {}));
        let occupied = X11Shortcut::spawn(
            collision,
            Arc::downgrade(&owner),
            GlobalRegistration::default(),
        )
        .expect("collision owner");
        assert!(occupied.wait_registered(Duration::from_secs(1)));
        assert!(!state.set_shortcut(collision, || panic!("collision committed")));
        assert!(lock_recover(&state.state)
            .x11
            .as_ref()
            .expect("restored worker")
            .wait_registered(Duration::from_secs(1)));
        assert_eq!(state.snapshot(), (Some(next), true));
        state.recording(true, || {});
        assert!(!state.snapshot().1);
        let recorded =
            X11Shortcut::spawn(next, Arc::downgrade(&owner), GlobalRegistration::default())
                .expect("recorder can acquire old chord");
        assert!(recorded.wait_registered(Duration::from_secs(1)));
        recorded.shutdown();
        state.recording(false, || {});
        assert!(lock_recover(&state.state)
            .x11
            .as_ref()
            .expect("resumed worker")
            .wait_registered(Duration::from_secs(1)));
        assert_eq!(state.snapshot(), (Some(next), true));
        occupied.shutdown();
        state.shutdown();
        assert!(!state.snapshot().1);
    }

    #[test]
    fn gdk_backend_detection_is_type_exact() {
        assert_eq!(
            classify_display_type("GdkWaylandDisplay"),
            DisplayBackend::Wayland
        );
        assert_eq!(classify_display_type("GdkX11Display"), DisplayBackend::X11);
        assert_eq!(
            classify_display_type("wayland-0"),
            DisplayBackend::Unsupported
        );
        assert_eq!(classify_display_type(""), DisplayBackend::Unsupported);
    }

    #[test]
    fn production_adapter_has_no_panicking_diagnostics() {
        let production = include_str!("linux_global_shortcuts.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("production Linux global-shortcut adapter");
        for forbidden in [
            "eprintln!",
            ".unwrap(",
            ".expect(",
            "panic!",
            "unreachable!",
        ] {
            assert!(!production.contains(forbidden));
        }
    }
}
