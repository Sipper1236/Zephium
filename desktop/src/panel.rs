//! Configures the Tao-owned overlay window without changing its Objective-C
//! class. Tao allocates a private `NSWindow` subclass with its own ivars and
//! method overrides; replacing that live object's class with an unrelated
//! `NSPanel` subclass violates Objective-C's layout and dispatch invariants.

mod shapes;

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};

use block2::RcBlock;

use objc2::rc::Retained;
use objc2::Message;
use objc2_app_kit::{
    NSAnimatablePropertyContainer, NSAnimationContext, NSApplication,
    NSApplicationActivationOptions, NSColor, NSRunningApplication, NSWindow,
    NSWindowAnimationBehavior, NSWindowCollectionBehavior, NSWorkspace,
};
use objc2_foundation::{MainThreadMarker, NSProcessInfo, NSRect};
use objc2_quartz_core::CAMediaTimingFunction;
use objc2_web_kit::WKWebView;
use tauri::WebviewWindow;

thread_local! {
    /// The application that was frontmost when the launcher took activation,
    /// so an explicit dismissal can hand the keyboard straight back to it.
    static PREVIOUS: RefCell<Option<Retained<NSRunningApplication>>> = const { RefCell::new(None) };
    /// Whether the launcher is, or is on its way to being, on screen. The
    /// window stays visible while it fades out, so this is the truth.
    static SHOWN: Cell<bool> = const { Cell::new(false) };
    /// Latest show or hide, so a fade-out finishing after the launcher was
    /// called back does not order it out.
    static FADE: Cell<u64> = const { Cell::new(0) };
}

/// Zephium's own Reduce Motion preference, alongside the system's.
static REDUCE_MOTION: AtomicBool = AtomicBool::new(false);

pub fn set_reduce_motion(reduce: bool) {
    REDUCE_MOTION.store(reduce, Ordering::Relaxed);
}

fn calm() -> bool {
    REDUCE_MOTION.load(Ordering::Relaxed)
        || NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

fn fade(window: &NSWindow, to: f64, duration: f64, curve: [f32; 4], done: impl Fn() + 'static) {
    let target = window.retain();
    let changes = RcBlock::new(move |context: NonNull<NSAnimationContext>| {
        // SAFETY: AppKit passes the live context for the duration of the block.
        let context = unsafe { context.as_ref() };
        context.setDuration(duration);
        let [a, b, c, d] = curve;
        context.setTimingFunction(Some(&CAMediaTimingFunction::functionWithControlPoints(
            a, b, c, d,
        )));
        target.animator().setAlphaValue(to);
    });
    let done = RcBlock::new(done);
    NSAnimationContext::runAnimationGroup_completionHandler(&changes, Some(&done));
}

fn with_window(window: &WebviewWindow, f: impl FnOnce(&NSWindow)) {
    if MainThreadMarker::new().is_none() {
        return;
    }
    if let Ok(ptr) = window.ns_window() {
        // SAFETY: `ns_window` returns Tao's live `NSWindow` subclass. We only
        // borrow it for this main-thread call and never change its class.
        f(unsafe { &*(ptr as *const NSWindow) });
    }
}

pub fn configure(window: &WebviewWindow) {
    with_window(window, |window| {
        window.setCollectionBehavior(
            window.collectionBehavior()
                | NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
        window.setHidesOnDeactivate(false);
        // The launcher animates itself; AppKit's own ordering animation would
        // run on top of it.
        window.setAnimationBehavior(NSWindowAnimationBehavior::None);
        // Clipping and the shadow depend on the material, so `install_shapes`
        // owns them.
        window.setOpaque(false);
        window.setBackgroundColor(Some(&NSColor::clearColor()));
    });
}

/// Installs the launcher's native shapes for the current appearance.
pub fn install_shapes(
    window: &WebviewWindow,
    reduce_transparency: bool,
    dark: bool,
) -> crate::material::Material {
    let mut material = crate::material::Material::None;
    with_window(window, |window| {
        material = shapes::install(window, reduce_transparency, dark);
    });
    material
}

pub fn shapes_active() -> bool {
    MainThreadMarker::new().is_some() && shapes::active()
}

pub fn layout(layout: &zephium_ipc::PanelLayout, animate: bool) {
    if MainThreadMarker::new().is_some() {
        shapes::apply(layout, animate);
    }
}

/// Activation on macOS 14 and later is cooperative: `activate` asks, and the
/// system grants it when the user's own action (the launcher's shortcut) led
/// here. `activateIgnoringOtherApps` is only a deprecated hint there.
fn activate(app: &NSApplication) {
    if NSProcessInfo::processInfo()
        .operatingSystemVersion()
        .majorVersion
        >= 14
    {
        app.activate();
    } else {
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
    }
}

/// Makes the launcher's page the first responder, so typing and Escape reach
/// it the moment the window is key. The content view is wry's parent view,
/// which refuses first responder and would leave the page without the keys
/// until it was clicked.
fn focus_page(window: &WebviewWindow) {
    let _ = window.with_webview(|webview| {
        if MainThreadMarker::new().is_none() {
            return;
        }
        // SAFETY: Tauri supplies a live WKWebView pointer for the duration of
        // this main-thread callback.
        let page: &WKWebView = unsafe { &*webview.inner().cast() };
        if let Some(window) = page.window() {
            window.makeFirstResponder(Some(page));
        }
    });
}

/// Takes the keyboard from whichever application is frontmost. The launcher
/// is made main as well as key before activating, because activation brings
/// the app's main window forward too, and that must be the launcher rather
/// than a browser window sitting behind the user's current app. The page is
/// given the keyboard once, on entering; a repeat presentation of a launcher
/// already up leaves wherever focus has moved to alone.
pub fn show(window: &WebviewWindow) {
    let Some(main) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(main);
    if !app.isActive() {
        let frontmost = NSWorkspace::sharedWorkspace().frontmostApplication();
        let ours = NSRunningApplication::currentApplication();
        PREVIOUS.with(|previous| {
            *previous.borrow_mut() =
                frontmost.filter(|app| app.processIdentifier() != ours.processIdentifier());
        });
    }
    let entering = !SHOWN.replace(true);
    if entering {
        FADE.set(FADE.get().wrapping_add(1));
    }
    with_window(window, |window| {
        if entering && !window.isVisible() {
            window.setAlphaValue(0.0);
        }
        window.orderFrontRegardless();
        window.makeKeyWindow();
        window.makeMainWindow();
        if entering {
            let calm = calm();
            shapes::present(calm);
            // Quick to arrive, so the field takes typing on the first frame;
            // the glass carries the motion.
            fade(
                window,
                1.0,
                if calm { 0.12 } else { 0.18 },
                [0.2, 0.8, 0.2, 1.0],
                || {},
            );
        }
    });
    if entering {
        focus_page(window);
    }
    if !app.isActive() {
        activate(&app);
    }
}

pub fn hide(window: &WebviewWindow) {
    with_window(window, |window| {
        if !SHOWN.replace(false) {
            if window.isVisible() {
                window.orderOut(None);
            }
            return;
        }
        let generation = FADE.get().wrapping_add(1);
        FADE.set(generation);
        let target = window.retain();
        // Leaving is faster than arriving: the user has already moved on.
        fade(window, 0.0, 0.12, [0.4, 0.0, 1.0, 1.0], move || {
            if FADE.get() == generation {
                target.orderOut(None);
                target.setAlphaValue(1.0);
            }
        });
    });
}

/// Returns the keyboard to the application the launcher was summoned over.
/// Only an explicit dismissal does this; leaving by clicking elsewhere has
/// already chosen where focus goes.
pub fn return_to_previous() {
    if MainThreadMarker::new().is_none() {
        return;
    }
    if let Some(app) = PREVIOUS.with(|previous| previous.borrow_mut().take()) {
        if !app.isTerminated() {
            app.activateWithOptions(NSApplicationActivationOptions::empty());
        }
    }
}

pub fn forget_previous() {
    PREVIOUS.with(|previous| previous.borrow_mut().take());
}

/// Brings a browser window forward even when it is minimized. Tao's
/// `set_focus` ignores a miniaturized window and `unminimize` is asynchronous,
/// so the pair silently does nothing for the window that most needs it.
pub fn raise(window: &WebviewWindow) {
    let Some(main) = MainThreadMarker::new() else {
        return;
    };
    with_window(window, |window| {
        if window.isMiniaturized() {
            window.deminiaturize(None);
        }
        window.makeKeyAndOrderFront(None);
    });
    activate(&NSApplication::sharedApplication(main));
}

/// Resizes in one frame change with the top edge held, so results grow
/// downward. `setContentSize` keeps the bottom-left origin instead, and a
/// separate reposition afterwards shows one frame at the wrong place.
pub fn set_height(window: &WebviewWindow, height: f64) {
    with_window(window, |window| {
        let frame = window.frame();
        if (frame.size.height - height).abs() < 0.5 {
            return;
        }
        let top = frame.origin.y + frame.size.height;
        let mut next: NSRect = frame;
        next.size.height = height;
        next.origin.y = top - height;
        window.setFrame_display(next, true);
    });
}

pub fn is_visible(window: &WebviewWindow) -> bool {
    let mut visible = false;
    with_window(window, |window| visible = window.isVisible());
    visible
}

pub fn application_active() -> bool {
    let Some(main) = MainThreadMarker::new() else {
        return false;
    };
    objc2_app_kit::NSApplication::sharedApplication(main).isActive()
}
