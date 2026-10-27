//! WebKitGTK owns transport; GTK owns consent; the shared coordinator owns
//! admission, persistence, cancellation, bounded workers and publication.
use super::*;
use gtk::glib::{self, object::ObjectType, ControlFlow};
use gtk::prelude::*;
use webkit2gtk::{DownloadExt, URIResponseExt};

pub(super) const CAN_PAUSE_RESUME: bool = false;

pub(super) type Native = webkit2gtk::Download;
pub(super) type Delegate = Rc<Cell<Option<DownloadError>>>;
pub(super) type Timer = glib::SourceId;
pub(super) type SavePanel = gtk::FileChooserDialog;
pub(super) type DirectoryPanel = gtk::FileChooserDialog;

// All download dialogs share one main-thread lease. This prevents pages from
// flooding the desktop with native save panels or covering directory consent.
thread_local! { static DIALOG_OPEN: Cell<bool> = const { Cell::new(false) }; }
pub(super) struct DialogLease;
impl DialogLease {
    fn acquire() -> Option<Self> {
        DIALOG_OPEN.with(|open| {
            if open.get() {
                None
            } else {
                open.set(true);
                Some(Self)
            }
        })
    }
}
impl Drop for DialogLease {
    fn drop(&mut self) {
        DIALOG_OPEN.with(|open| open.set(false));
    }
}

pub(super) struct Source {
    permit: EventPermit,
    surface_intent: Arc<AtomicBool>,
    navigation: NavigationEpochTracker,
    activity: NavigationActivity,
    view: webkit2gtk::WebView,
    window: gtk::Window,
}
impl Source {
    pub(super) fn live(&self) -> bool {
        self.permit.active_token().is_some()
            && self.surface_intent.load(Ordering::Acquire)
            && self.navigation.matches_activity(self.activity)
            && self.view.is_visible()
            && self.view.is_mapped()
            && self.window.is_visible()
            && self.view.toplevel().as_ref() == Some(self.window.upcast_ref())
    }
}

pub(super) fn progress(native: &Native) -> Option<(u64, Option<u64>)> {
    Some((
        native.received_data_length(),
        native
            .response()
            .map(|r| r.content_length())
            .filter(|n| *n > 0),
    ))
}
pub(super) fn completion_bytes(native: &Native) -> Result<Option<u64>, DownloadError> {
    // WebKitGTK counts decoded data written to the destination, unlike the
    // potentially compressed response's Content-Length.
    Ok(Some(native.received_data_length()))
}
pub(super) fn stop_timer(timer: Timer) {
    timer.remove();
}
pub(super) fn cancel_directory(panel: &DirectoryPanel) {
    panel.close();
}

fn desktop_open(path: &std::path::Path) -> Result<(), DownloadError> {
    let uri = glib::filename_to_uri(path, None).map_err(|_| DownloadError::Destination)?;
    gtk::gio::AppInfo::launch_default_for_uri(&uri, None::<&gtk::gio::AppLaunchContext>)
        .map_err(|_| DownloadError::Unavailable)
}
pub(super) fn reveal(path: &std::path::Path) -> Result<(), DownloadError> {
    // Opening the verified parent works across desktops without invoking a
    // shell or interpolating an untrusted path into a command line.
    desktop_open(path.parent().ok_or(DownloadError::Destination)?)
}
pub(super) fn open(path: &std::path::Path) -> Result<(), DownloadError> {
    // Desktop files and scripts can be executable even without a Unix execute
    // bit. Do not dispatch them through desktop launchers as ordinary documents.
    let (kind, uncertain) = gtk::gio::content_type_guess(Some(path), &[]);
    if uncertain || (kind != "text/plain" && gtk::gio::content_type_can_be_executable(&kind)) {
        return Err(DownloadError::Protection);
    }
    desktop_open(path)
}

impl Downloads {
    pub(super) fn user_pause(
        self: &Rc<Self>,
        _partition: Partition,
        _id: DownloadId,
    ) -> Result<(), DownloadError> {
        // Cancelling a WebKitGTK download provides no retained resume state.
        // Refuse pause before changing or stopping the active transfer.
        Err(DownloadError::Unsupported)
    }

    pub(super) fn resume(
        &self,
        _partition: Partition,
        _id: DownloadId,
    ) -> Result<(), DownloadError> {
        // WebKitGTK provides no retained resume operation.
        Err(DownloadError::Unsupported)
    }
    pub(super) fn ensure_timer(self: &Rc<Self>) {
        let (period, slow) = self.tick_period();
        if self.timer.borrow().is_some() && self.timer_slow.get() == slow {
            return;
        }
        if let Some(timer) = self.timer.borrow_mut().take() {
            stop_timer(timer);
        }
        self.timer_slow.set(slow);
        let weak = Rc::downgrade(self);
        *self.timer.borrow_mut() = Some(glib::timeout_add_local(period, move || {
            if let Some(manager) = weak.upgrade() {
                manager.tick();
                ControlFlow::Continue
            } else {
                ControlFlow::Break
            }
        }));
    }

    pub(in crate::host) fn admit(
        self: &Rc<Self>,
        partition: Partition,
        permit: EventPermit,
        surface_intent: Arc<AtomicBool>,
        navigation: NavigationEpochTracker,
        native: &Native,
    ) {
        if self.stopping.get()
            || self.retired.borrow().contains(&partition.profile())
            || self.active.borrow().len() >= MAX_ACTIVE_DOWNLOADS
            || self.work.get() >= MAX_BACKGROUND_WORK
            || !self.private_cleanup_capacity(partition)
            || permit.active_token().is_none()
            || !surface_intent.load(Ordering::Acquire)
        {
            native.cancel();
            return;
        }
        let Some(view) = native.web_view() else {
            native.cancel();
            return;
        };
        let Some(window) = view
            .toplevel()
            .and_then(|w| w.downcast::<gtk::Window>().ok())
        else {
            native.cancel();
            return;
        };
        let Some(activity) = navigation.activity_snapshot() else {
            native.cancel();
            return;
        };
        let source = Source {
            permit,
            surface_intent,
            navigation,
            activity,
            view,
            window,
        };
        if !source.live() || !source.window.is_active() {
            native.cancel();
            return;
        }
        let request = native.request().and_then(|r| unsafe {
            bounded_uri(webkit2gtk::ffi::webkit_uri_request_get_uri(r.as_ptr()))
        });
        if !request.as_deref().is_some_and(allowed_download_uri) {
            native.cancel();
            return;
        }
        // GTK does not expose the initiating frame's origin. Label the native
        // top-level page as context, rather than claiming it is the source.
        let origin = unsafe {
            bounded_uri(webkit2gtk::ffi::webkit_web_view_get_uri(
                source.view.as_ptr(),
            ))
        }
        .and_then(|uri| url::Url::parse(&uri).ok())
        .and_then(|url| PageOrigin::from_url(&url).ok());
        let Some(origin) = origin else {
            native.cancel();
            return;
        };
        let id = DownloadId::generate();
        let native_failure = Rc::new(Cell::new(None));
        self.active.borrow_mut().insert(
            id,
            Transfer {
                partition,
                record: DownloadRecord {
                    id,
                    session: self.session,
                    revision: 1,
                    created_at: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_or(0, |t| t.as_secs() as i64),
                    filename: "download".into(),
                    source: origin.as_str().into(),
                    source_is_context: true,
                    state: DownloadState::Pending,
                    received: 0,
                    total: None,
                    error: None,
                    destination: None,
                    staging: None,
                    staging_identity: None,
                    identity: None,
                    writer: None,
                    writer_released: false,
                },
                native: native.clone(),
                _delegate: native_failure.clone(),
                source: Some(source),
                destination_reply: None,
                destination: None,
                panel: None,
                panel_lease: None,
                on_started: None,
                authorized: false,
                cancelling: false,
                persisting_terminal: false,
                kept: false,
                deadline: DecisionDeadline::new(DecisionPhase::Admission, Instant::now()),
            },
        );
        let weak = Rc::downgrade(self);
        native.connect_decide_destination(move |native, suggested| {
            if let Some(manager) = weak.upgrade() {
                manager.destination(id, native, suggested);
            } else {
                native.cancel();
            }
            true // 2.40+ explicitly waits for our asynchronous destination.
        });
        let failure = native_failure.clone();
        native.connect_failed(move |_, error| {
            failure.set(Some(native_error(error)));
        });
        let weak = Rc::downgrade(self);
        native.connect_finished(move |_| {
            if let Some(manager) = weak.upgrade() {
                // WebKit emits finished after failed. Only finished proves
                // that the writer has released the destination for cleanup.
                if let Some(error) = native_failure.get() {
                    manager.native_failed_with_error(id, error);
                } else {
                    manager.native_finished(id);
                }
            }
        });
        native.set_allow_overwrite(false);
        self.ensure_recovery(partition);
        self.ensure_timer();
        (self.notify)(partition.profile());
    }

    fn destination(self: &Rc<Self>, id: DownloadId, native: &Native, suggested: &str) {
        let response_uri = native.response().and_then(|response| unsafe {
            bounded_uri(webkit2gtk::ffi::webkit_uri_response_get_uri(
                response.as_ptr(),
            ))
        });
        if !response_uri.as_deref().is_some_and(allowed_download_uri) {
            self.cancel(id, Some(DownloadError::Source));
            return;
        }
        let partition = {
            let mut active = self.active.borrow_mut();
            let Some(transfer) = active.get_mut(&id).filter(|t| {
                t.record.state == DownloadState::Pending
                    && !t.cancelling
                    && t.destination_reply.is_none()
                    && t.destination.is_none()
            }) else {
                native.cancel();
                return;
            };
            transfer.record.filename = if suggested.len() <= 4096 {
                safe_filename(suggested)
            } else {
                "download".into()
            };
            transfer.record.total = native
                .response()
                .map(|r| r.content_length())
                .filter(|n| *n > 0);
            let native = native.clone();
            transfer.destination_reply = Some(DestinationReply::new(move |path| {
                if let Some(uri) = path.and_then(|p| glib::filename_to_uri(p, None).ok()) {
                    native.set_destination(&uri);
                } else {
                    native.cancel();
                }
            }));
            transfer.partition
        };
        if let Some(preferences) = self.preferences.borrow().get(&partition.profile()).cloned() {
            self.choose_destination(id, preferences);
        } else if matches!(partition, Partition::Ephemeral(_)) {
            self.choose_destination(id, DownloadPreferences::default());
        } else {
            self.work.set(self.work.get() + 1);
            let sender = self.sender.clone();
            if !self.store.download_call(
                partition.profile(),
                DownloadStoreCall::Preferences,
                Box::new(move |reply| {
                    let _ = sender.send(Message::Preferences(id, reply));
                }),
            ) {
                let _ = self.sender.send(Message::Preferences(
                    id,
                    DownloadStoreReply::Error(DownloadError::Storage),
                ));
            }
        }
    }

    pub(super) fn choose_destination(
        self: &Rc<Self>,
        id: DownloadId,
        preferences: DownloadPreferences,
    ) {
        let context = self.active.borrow().get(&id).and_then(|t| {
            t.source.as_ref().map(|s| {
                (
                    s.live() && s.window.is_active(),
                    s.window.clone(),
                    t.record.filename.clone(),
                    t.record.source.clone(),
                )
            })
        });
        let Some((true, window, filename, source)) = context else {
            self.cancel(id, None);
            return;
        };
        let Some(lease) = DialogLease::acquire() else {
            self.cancel(id, Some(DownloadError::Capacity));
            return;
        };
        // GTK has no trustworthy download user-gesture flag; every transfer
        // requires native consent, even when automatic save is preferred.
        let panel = gtk::FileChooserDialog::new(
            Some(&format!("Save download from {source}")),
            Some(&window),
            gtk::FileChooserAction::Save,
        );
        panel.add_buttons(&[
            ("Cancel", gtk::ResponseType::Cancel),
            ("Save", gtk::ResponseType::Accept),
        ]);
        panel.set_modal(true);
        panel.set_do_overwrite_confirmation(true);
        panel.set_local_only(true);
        panel.set_current_name(&filename);
        if let Some(directory) = preferences
            .directory
            .map(PathBuf::from)
            .or_else(dirs::download_dir)
        {
            panel.set_current_folder(directory);
        }
        if let Some(transfer) = self.active.borrow_mut().get_mut(&id) {
            transfer.panel = Some(panel.clone());
            transfer.panel_lease = Some(lease);
            transfer.deadline = DecisionDeadline::new(DecisionPhase::NativePicker, Instant::now());
        }
        let weak = Rc::downgrade(self);
        panel.connect_response(move |panel, response| {
            let path = (response == gtk::ResponseType::Accept)
                .then(|| panel.filename())
                .flatten();
            panel.hide();
            if let Some(manager) = weak.upgrade() {
                if let Some(transfer) = manager.active.borrow_mut().get_mut(&id) {
                    transfer.panel = None;
                    transfer.panel_lease.take();
                }
                if let Some(path) = path {
                    manager.prepare(id, path, None);
                } else {
                    manager.cancel(id, None);
                }
            }
            unsafe {
                panel.destroy();
            }
        });
        panel.show();
    }

    pub(super) fn cancel(&self, id: DownloadId, error: Option<DownloadError>) {
        let native = {
            let mut active = self.active.borrow_mut();
            let Some(transfer) = active.get_mut(&id) else {
                return;
            };
            if transfer.record.state.terminal()
                || transfer.record.state == DownloadState::Finalizing
                || transfer.cancelling
            {
                return;
            }
            transfer.cancelling = true;
            transfer.record.state = DownloadState::Cancelling;
            transfer.record.error = transfer.record.error.or(error);
            // Move callbacks and panels out before calling native operations:
            // GTK/WebKit may synchronously reenter the coordinator.
            (
                transfer.native.clone(),
                transfer.panel.take(),
                transfer.destination_reply.take(),
                transfer.panel_lease.take(),
            )
        };
        if let Some(panel) = native.1 {
            panel.close();
        }
        drop(native.2);
        drop(native.3);
        native.0.cancel();
    }

    pub(super) fn choose_directory(
        self: &Rc<Self>,
        token: u64,
        partition: Partition,
        preferences: DownloadPreferences,
        done: DownloadCompletion,
    ) {
        let Some(window) = gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|widget| widget.downcast::<gtk::Window>().ok())
            .find(|w| w.is_active() && w.is_visible())
        else {
            done.finish(DownloadResponse::Error {
                error: DownloadError::Unavailable,
            });
            return;
        };
        let Some(lease) = DialogLease::acquire() else {
            done.finish(DownloadResponse::Error {
                error: DownloadError::Capacity,
            });
            return;
        };
        let panel = gtk::FileChooserDialog::new(
            Some("Choose where Zephium saves downloads"),
            Some(&window),
            gtk::FileChooserAction::SelectFolder,
        );
        panel.add_buttons(&[
            ("Cancel", gtk::ResponseType::Cancel),
            ("Choose", gtk::ResponseType::Accept),
        ]);
        panel.set_local_only(true);
        panel.set_modal(true);
        self.calls.borrow_mut().insert(
            token,
            UiCall {
                partition,
                call: DownloadCall::ChooseDirectory,
                done,
            },
        );
        self.directory_panels
            .borrow_mut()
            .insert(token, (panel.clone(), lease));
        let weak = Rc::downgrade(self);
        panel.connect_response(move |panel, response| {
            let path = (response == gtk::ResponseType::Accept)
                .then(|| panel.filename())
                .flatten();
            panel.hide();
            unsafe {
                panel.destroy();
            }
            let Some(manager) = weak.upgrade() else {
                return;
            };
            manager.directory_panels.borrow_mut().remove(&token);
            let Some(request) = manager.calls.borrow_mut().remove(&token) else {
                return;
            };
            let Some(path) = path else {
                request.done.finish(DownloadResponse::Error {
                    error: DownloadError::Cancelled,
                });
                return;
            };
            manager.work.set(manager.work.get() + 1);
            let sender = manager.sender.clone();
            let preferences = preferences.clone();
            if std::thread::Builder::new()
                .name("zephium-download-directory".into())
                .spawn(move || {
                    let result = super::super::download_files::select_directory(path);
                    let _ = sender.send(Message::DirectorySelected(request, preferences, result));
                })
                .is_err()
            {
                manager.work.set(manager.work.get() - 1);
            }
            manager.ensure_timer();
        });
        panel.show();
    }
}

fn native_error(error: &glib::Error) -> DownloadError {
    use webkit2gtk::DownloadError as NativeError;
    if error.matches(NativeError::CancelledByUser) {
        DownloadError::Cancelled
    } else if error.matches(NativeError::Destination) {
        DownloadError::Destination
    } else {
        DownloadError::Network
    }
}

// Native WebKit strings may be hostile. Bound the scan before allocating a
// Rust copy; keep the native object alive throughout this operation.
unsafe fn bounded_uri(pointer: *const libc::c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    let length = libc::strnlen(pointer, 8193);
    if length > 8192 {
        return None;
    }
    std::str::from_utf8(std::slice::from_raw_parts(pointer.cast::<u8>(), length))
        .ok()
        .map(str::to_owned)
}

fn allowed_download_uri(uri: &str) -> bool {
    uri.len() <= 8192
        && (zephium_core::navigation::is_allowed_str(uri)
            || uri.starts_with("blob:")
            || uri.starts_with("data:"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use webkit2gtk::WebViewExt;
    use wry::{WebViewBuilder, WebViewBuilderExtUnix, WebViewExtUnix};

    struct PrivateStore;
    impl Store for PrivateStore {
        fn save_session(&self, _: zephium_core::session::SessionState) {}
        fn flush(&self) -> bool {
            true
        }
        fn load_session(&self) -> zephium_core::ports::store::SessionLoad {
            zephium_core::ports::store::SessionLoad::Absent
        }
        fn record_visit(&self, _: ProfileId, _: String, _: String) {}
        fn app_setting(&self, _: &str) -> Option<String> {
            None
        }
        fn set_app_setting(&self, _: String, _: String) -> bool {
            false
        }
        fn search_history(
            &self,
            _: ProfileId,
            _: &str,
            _: u32,
        ) -> Vec<zephium_core::ports::store::HistoryHit> {
            vec![]
        }
        fn history_page(
            &self,
            _: ProfileId,
            _: &str,
            _: Option<i64>,
            _: Option<i64>,
            _: u32,
        ) -> Vec<zephium_core::ports::store::HistoryVisit> {
            vec![]
        }
        fn forget_history_urls(&self, _: ProfileId, _: &[String]) -> u32 {
            0
        }
        fn clear_history(&self, _: ProfileId, _: Option<i64>) -> u32 {
            0
        }
        fn amend_visit_title(&self, _: ProfileId, _: String, _: String) -> bool {
            false
        }
        fn favicon_age(&self, _: ProfileId, _: &str) -> Option<i64> {
            None
        }
        fn save_favicon(&self, _: ProfileId, _: String, _: Option<String>, _: Vec<u8>) {}
        fn favicon_bytes(&self, _: ProfileId, _: &str) -> Option<(Option<String>, Vec<u8>)> {
            None
        }
        fn download_recovery_profiles(
            &self,
            done: Box<dyn FnOnce(DownloadStoreReply) + Send>,
        ) -> bool {
            done(DownloadStoreReply::Profiles(vec![]));
            true
        }
        fn download_call(
            &self,
            _: ProfileId,
            _: DownloadStoreCall,
            _: Box<dyn FnOnce(DownloadStoreReply) + Send>,
        ) -> bool {
            panic!("private downloads must never reach durable Store");
        }
    }

    fn pump_until(mut condition: impl FnMut() -> bool, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while !condition() && Instant::now() < deadline {
            while gtk::events_pending() {
                gtk::main_iteration_do(false);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            condition(),
            "native condition did not settle before its deadline"
        );
    }

    #[test]
    #[ignore = "requires a native X11/Wayland display and supported WebKitGTK; run with Xvfb"]
    fn native_coordinator_requires_consent_publishes_and_drains() {
        crate::enforce_runtime_security_floor().unwrap();
        gtk::init().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let fixture_stop = stop.clone();
        let fixture = std::thread::spawn(move || {
            while !fixture_stop.load(Ordering::Acquire) {
                if let Ok((mut stream, _)) = listener.accept() {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut request = [0u8; 2048];
                    let length = stream.read(&mut request).unwrap_or(0);
                    let (kind, body) = if request[..length].starts_with(b"GET /page ") {
                        ("text/html", "<title>ready</title>")
                    } else {
                        ("application/octet-stream", "coordinator bytes")
                    };
                    let header = format!("HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nContent-Disposition: {}\r\nConnection: close\r\n\r\n", body.len(),
                        if kind == "text/html" { "inline" } else { "attachment; filename=coordinator.txt" });
                    let _ = stream.write_all(header.as_bytes());
                    let _ = stream.write_all(body.as_bytes());
                } else {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        });
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_default_size(640, 480);
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        window.add(&container);
        let profile = ProfileId::from(901);
        let manager = Downloads::new(Arc::new(PrivateStore), Arc::new(|_| {}));
        for call in [
            DownloadCall::Updates,
            DownloadCall::List {
                before: None,
                limit: 50,
            },
        ] {
            let (sender, receiver) = std::sync::mpsc::channel();
            manager.call(
                Partition::Ephemeral(profile),
                call,
                DownloadCompletion::new(move |response| sender.send(response).unwrap()),
            );
            match receiver.recv_timeout(Duration::from_secs(1)).unwrap() {
                DownloadResponse::Updates {
                    can_pause_resume, ..
                }
                | DownloadResponse::Page {
                    can_pause_resume, ..
                } => {
                    assert!(
                        !can_pause_resume,
                        "Linux must not advertise native pause/resume"
                    );
                }
                response => panic!("unexpected snapshot: {response:?}"),
            }
        }
        let preferences = DownloadPreferences {
            directory: Some(directory.path().to_str().unwrap().to_owned()),
            ..DownloadPreferences::default()
        };
        manager
            .preferences
            .borrow_mut()
            .insert(profile, preferences);
        let token = Arc::new(AtomicBool::new(true));
        let permit = EventPermit::bound(&token);
        let intent = Arc::new(AtomicBool::new(true));
        let navigation = NavigationEpochTracker::new();
        let page = format!("http://{address}/page");
        navigation.begin(&page).unwrap();
        let downloads = Rc::downgrade(&manager);
        let source_permit = permit.clone();
        let view = WebViewBuilder::new()
            .with_url(&page)
            .with_download_policy(wry::DownloadPolicy::UseHandlers)
            .with_native_download_handler(move |native| {
                downloads.upgrade().unwrap().admit(
                    Partition::Ephemeral(profile),
                    source_permit.clone(),
                    intent.clone(),
                    navigation.clone(),
                    native,
                );
            })
            .build_gtk(&container)
            .unwrap();
        window.show_all();
        window.present();
        window.window().unwrap().focus(0);
        pump_until(
            || view.webview().title().as_deref() == Some("ready") && window.is_active(),
            Duration::from_secs(10),
        );
        let native = view
            .webview()
            .download_uri(&format!("http://{address}/attachment"))
            .unwrap();
        pump_until(
            || {
                manager
                    .active
                    .borrow()
                    .values()
                    .any(|transfer| transfer.panel.is_some())
            },
            Duration::from_secs(5),
        );
        assert!(
            native.destination().is_none(),
            "the native writer waits for explicit GTK consent"
        );
        assert!(!directory.path().join("coordinator.txt").exists());
        let active_id = *manager.active.borrow().keys().next().unwrap();
        assert_eq!(
            manager.user_pause(Partition::Ephemeral(profile), active_id),
            Err(DownloadError::Unsupported),
        );
        assert_eq!(
            manager.resume(Partition::Ephemeral(profile), active_id),
            Err(DownloadError::Unsupported),
        );
        let panel = manager
            .active
            .borrow()
            .values()
            .find_map(|transfer| transfer.panel.clone())
            .unwrap();
        assert!(panel.set_current_folder(directory.path()));
        panel.set_current_name("coordinator.txt");
        panel.response(gtk::ResponseType::Accept);
        pump_until(
            || {
                manager
                    .recent
                    .borrow()
                    .iter()
                    .any(|(_, record)| record.state == DownloadState::Completed)
            },
            Duration::from_secs(10),
        );
        assert!(manager.active.borrow().is_empty());
        let record = manager.recent.borrow().front().unwrap().1.clone();
        assert!(record.writer_released && record.source_is_context);
        assert_eq!(
            std::fs::read(record.destination.as_ref().unwrap()).unwrap(),
            b"coordinator bytes"
        );
        verify_file(
            record.destination.as_ref().unwrap(),
            record.identity.as_ref().unwrap(),
        )
        .unwrap();
        let (sender, receiver) = mpsc::channel();
        manager.quiesce(
            Some(profile),
            Box::new(move |clean| sender.send(clean).unwrap()),
        );
        let mut drained = None;
        pump_until(
            || {
                drained = receiver.try_recv().ok().or(drained);
                drained.is_some()
            },
            Duration::from_secs(5),
        );
        assert_eq!(drained, Some(true));
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        stop.store(true, Ordering::Release);
        fixture.join().unwrap();
        drop(view);
        unsafe {
            window.destroy();
        }
    }

    #[test]
    fn dialog_admission_is_exclusive_and_released_on_drop() {
        let lease = DialogLease::acquire().unwrap();
        for _ in 0..3 {
            assert!(DialogLease::acquire().is_none());
        }
        drop(lease);
        assert!(DialogLease::acquire().is_some());
    }

    #[test]
    fn native_uri_scan_has_an_allocation_ceiling() {
        let bounded = std::ffi::CString::new("https://example.com").unwrap();
        assert_eq!(
            unsafe { bounded_uri(bounded.as_ptr()) }.as_deref(),
            Some("https://example.com")
        );
        let over = std::ffi::CString::new("x".repeat(8193)).unwrap();
        assert!(unsafe { bounded_uri(over.as_ptr()) }.is_none());
        assert!(unsafe { bounded_uri(std::ptr::null()) }.is_none());
    }

    #[test]
    #[ignore = "requires a native X11/Wayland display and supported WebKitGTK; run with Xvfb"]
    fn native_attachment_download_is_view_scoped_async_exclusive_and_cancellable() {
        crate::enforce_runtime_security_floor().unwrap();
        gtk::init().unwrap();
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("report.txt");
        std::fs::write(&output, b"existing user document").unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let fixture_stop = stop.clone();
        let fixture = std::thread::spawn(move || {
            while !fixture_stop.load(Ordering::Acquire) {
                if let Ok((mut stream, _)) = listener.accept() {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut request = [0u8; 2048];
                    let _ = stream.read(&mut request);
                    let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=report.txt\r\nContent-Encoding: gzip\r\nContent-Length: 34\r\nConnection: close\r\n\r\n");
                    let _ = stream.write_all(b"\x1f\x8b\x08\x00\x00\x00\x00\x00\x02\xffK\xc9/\xcf\xcb\xc9OLQH\xaa,I-\x06\x00fp\x97d\x0e\x00\x00\x00");
                } else {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        });

        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        window.add(&container);
        let mut context = wry::WebContext::new_ephemeral().unwrap();
        // This deny handler shares the exact native context. It must not
        // cancel the second view's transfer or observe its metadata.
        let denied = WebViewBuilder::new_with_web_context(&mut context)
            .with_download_policy(wry::DownloadPolicy::DenyWithoutMetadata)
            .build_gtk(&container)
            .unwrap();
        let calls = Rc::new(Cell::new(0));
        let completed = Rc::new(RefCell::new(None));
        let cancelled = Rc::new(Cell::new(false));
        let panicked = Rc::new(Cell::new(false));
        let panic_cancelled = panicked.clone();
        let admissions = calls.clone();
        let complete = completed.clone();
        let cancellation = cancelled.clone();
        let target = output.clone();
        let view = WebViewBuilder::new_with_web_context(&mut context)
            .with_download_policy(wry::DownloadPolicy::UseHandlers)
            .with_native_download_handler(move |native| {
                let ordinal = admissions.get();
                admissions.set(ordinal + 1);
                if ordinal == 2 {
                    let cancelled = panic_cancelled.clone();
                    native.connect_failed(move |_, error| {
                        cancelled.set(native_error(error) == DownloadError::Cancelled);
                    });
                    panic!("intentional native-download embedder panic");
                }
                if ordinal == 1 {
                    let cancellation = cancellation.clone();
                    native.connect_failed(move |_, error| {
                        cancellation.set(native_error(error) == DownloadError::Cancelled);
                    });
                    native.connect_decide_destination(|native, _| {
                        native.cancel();
                        true
                    });
                    return;
                }
                let destination =
                    Destination::prepare(DownloadId::generate(), target.clone(), None).unwrap();
                let destination = Rc::new(RefCell::new(Some(destination)));
                let pending = destination.clone();
                native.connect_decide_destination(move |native, _| {
                    let uri =
                        glib::filename_to_uri(pending.borrow().as_ref().unwrap().payload(), None)
                            .unwrap();
                    let native = native.clone();
                    // Exercise WebKitGTK's actual asynchronous destination
                    // wait rather than completing inside the signal stack.
                    glib::timeout_add_local_once(Duration::from_millis(25), move || {
                        native.set_allow_overwrite(false);
                        native.set_destination(&uri);
                    });
                    true
                });
                let complete = complete.clone();
                native.connect_finished(move |native| {
                    let destination = destination.borrow_mut().take().unwrap();
                    *complete.borrow_mut() = Some(destination.finish(
                        &format!("http://{address}"),
                        completion_bytes(native).unwrap(),
                    ));
                });
            })
            .build_gtk(&container)
            .unwrap();
        window.show_all();
        while gtk::events_pending() {
            gtk::main_iteration_do(false);
        }
        let token = Arc::new(AtomicBool::new(true));
        let intent = Arc::new(AtomicBool::new(true));
        let navigation = NavigationEpochTracker::new();
        navigation.begin(&format!("http://{address}/page")).unwrap();
        let mut source = Source {
            permit: EventPermit::bound(&token),
            surface_intent: intent.clone(),
            activity: navigation.activity_snapshot().unwrap(),
            navigation: navigation.clone(),
            view: view.webview(),
            window: window.clone(),
        };
        assert!(source.live(), "mapped page has live download authority");
        intent.store(false, Ordering::Release);
        assert!(!source.live());
        intent.store(true, Ordering::Release);
        navigation
            .begin(&format!("http://{address}/replacement"))
            .unwrap();
        assert!(
            !source.live(),
            "a later navigation revokes pending destination consent"
        );
        source.activity = navigation.activity_snapshot().unwrap();
        assert!(source.live());
        source.permit.revoke();
        assert!(!source.live());
        let download = view
            .webview()
            .download_uri(&format!("http://{address}/attachment"));
        let deadline = Instant::now() + Duration::from_secs(10);
        while completed.borrow().is_none() && Instant::now() < deadline {
            while gtk::events_pending() {
                gtk::main_iteration_do(false);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let (published, identity) = completed
            .borrow_mut()
            .take()
            .expect("native attachment finished")
            .unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), b"existing user document");
        assert_eq!(published.file_name().unwrap(), "report (1).txt");
        assert_eq!(std::fs::read(&published).unwrap(), b"download bytes");
        verify_file(&published, &identity).unwrap();
        assert_eq!(calls.get(), 1);
        let cancelled_download = view
            .webview()
            .download_uri(&format!("http://{address}/cancelled"));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !cancelled.get() && Instant::now() < deadline {
            while gtk::events_pending() {
                gtk::main_iteration_do(false);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            cancelled.get(),
            "native cancellation settled with the cancellation error"
        );
        assert!(cancelled_download.unwrap().destination().is_none());
        assert!(download.unwrap().destination().is_some());
        assert_eq!(calls.get(), 2);
        let panic_download = view
            .webview()
            .download_uri(&format!("http://{address}/panic"));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !panicked.get() && Instant::now() < deadline {
            while gtk::events_pending() {
                gtk::main_iteration_do(false);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            panicked.get(),
            "an embedder panic cancels the native transfer"
        );
        assert!(panic_download.unwrap().destination().is_none());
        assert_eq!(calls.get(), 3);
        use webkit2gtk::WebContextExt;
        let orphan = view
            .webview()
            .context()
            .unwrap()
            .download_uri(&format!("http://{address}/orphan"))
            .unwrap();
        let orphan_finished = Rc::new(Cell::new(false));
        let finished = orphan_finished.clone();
        orphan.connect_finished(move |_| finished.set(true));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !orphan_finished.get() && Instant::now() < deadline {
            while gtk::events_pending() {
                gtk::main_iteration_do(false);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(orphan_finished.get(), "context-only downloads are denied");
        assert!(orphan.destination().is_none());
        assert_eq!(
            calls.get(),
            3,
            "unowned transfers never enter the embedder callback"
        );
        assert_eq!(
            std::fs::read_dir(root.path()).unwrap().count(),
            2,
            "no staging residue after publication or cancellation"
        );
        stop.store(true, Ordering::Release);
        fixture.join().unwrap();
        drop(denied);
        drop(view);
        unsafe {
            window.destroy();
        }
    }
}
