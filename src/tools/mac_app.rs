//! Start from the Finder (macOS): when the program runs inside an `.app` bundle without
//! arguments it serves the interface and stays in the Dock like any other app, with a menu to
//! reopen the interface and to quit. The interface itself opens in the default browser.
//!
//! AppKit is called through the Objective-C runtime directly, so the launcher is this same Rust
//! program and needs no Swift code and no extra crate.

use std::ffi::{c_char, c_void, CString};
use std::sync::OnceLock;

type Id = *mut c_void;
type Sel = *mut c_void;

#[link(name = "objc")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Sel;
    fn objc_msgSend();
    fn objc_allocateClassPair(superclass: Id, name: *const c_char, extra: usize) -> Id;
    fn objc_registerClassPair(class: Id);
    fn class_addMethod(class: Id, sel: Sel, imp: *const c_void, types: *const c_char) -> bool;
}
#[link(name = "AppKit", kind = "framework")]
extern "C" {}

static URL: OnceLock<String> = OnceLock::new();

/// True when this executable is the main program of an `.app` bundle.
pub fn in_bundle() -> bool {
    std::env::current_exe().map(|p| p.to_string_lossy().contains(".app/Contents/MacOS/")).unwrap_or(false)
}

/// A program started from the Finder gets only the system's short search path; ffmpeg from
/// Homebrew lives outside it.
pub fn extend_path() {
    let old = std::env::var("PATH").unwrap_or_default();
    let mut parts: Vec<String> = old.split(':').filter(|s| !s.is_empty()).map(str::to_string).collect();
    for extra in ["/opt/homebrew/bin", "/usr/local/bin"] {
        if !parts.iter().any(|p| p == extra) {
            parts.push(extra.to_string());
        }
    }
    std::env::set_var("PATH", parts.join(":"));
}

fn class(name: &str) -> Id {
    let c = CString::new(name).unwrap();
    unsafe { objc_getClass(c.as_ptr()) }
}
fn sel(name: &str) -> Sel {
    let c = CString::new(name).unwrap();
    unsafe { sel_registerName(c.as_ptr()) }
}
unsafe fn send0(obj: Id, s: &str) -> Id {
    let f: unsafe extern "C" fn(Id, Sel) -> Id = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
    f(obj, sel(s))
}
unsafe fn send1(obj: Id, s: &str, a: Id) -> Id {
    let f: unsafe extern "C" fn(Id, Sel, Id) -> Id = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
    f(obj, sel(s), a)
}
unsafe fn send_int(obj: Id, s: &str, a: isize) -> Id {
    let f: unsafe extern "C" fn(Id, Sel, isize) -> Id = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
    f(obj, sel(s), a)
}
unsafe fn send_bool(obj: Id, s: &str, a: bool) -> Id {
    let f: unsafe extern "C" fn(Id, Sel, bool) -> Id = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
    f(obj, sel(s), a)
}
unsafe fn nsstring(text: &str) -> Id {
    let c = CString::new(text).unwrap_or_default();
    let f: unsafe extern "C" fn(Id, Sel, *const c_char) -> Id = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
    f(class("NSString"), sel("stringWithUTF8String:"), c.as_ptr())
}
unsafe fn menu_item(title: &str, action: Option<&str>, key: &str) -> Id {
    let item = send0(class("NSMenuItem"), "alloc");
    let f: unsafe extern "C" fn(Id, Sel, Id, Sel, Id) -> Id = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
    f(item, sel("initWithTitle:action:keyEquivalent:"), nsstring(title), action.map(sel).unwrap_or(std::ptr::null_mut()), nsstring(key))
}

fn open_interface() {
    if let Some(url) = URL.get() {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
}

extern "C" fn open_ui(_this: Id, _cmd: Sel, _sender: Id) {
    open_interface();
}
/// A click on the Dock icon brings the interface back.
extern "C" fn handle_reopen(_this: Id, _cmd: Sel, _app: Id, _visible: bool) -> bool {
    open_interface();
    false
}
/// Quitting must not leave a detection run behind.
extern "C" fn will_terminate(_this: Id, _cmd: Sel, _note: Id) {
    super::ui::cancel_all();
}

/// Runs the app until the user quits it. Never returns.
pub fn run(url: String) -> ! {
    let _ = URL.set(url);
    unsafe {
        let app = send0(class("NSApplication"), "sharedApplication");
        send_int(app, "setActivationPolicy:", 0); // regular app: Dock icon and menu bar

        let name = CString::new("RecoMinimapAppDelegate").unwrap();
        let delegate_class = objc_allocateClassPair(class("NSObject"), name.as_ptr(), 0);
        let types = |t: &str| CString::new(t).unwrap();
        let (t_void, t_bool) = (types("v@:@"), types("B@:@B"));
        class_addMethod(delegate_class, sel("openInterface:"), open_ui as *const c_void, t_void.as_ptr());
        class_addMethod(delegate_class, sel("applicationShouldHandleReopen:hasVisibleWindows:"), handle_reopen as *const c_void, t_bool.as_ptr());
        class_addMethod(delegate_class, sel("applicationWillTerminate:"), will_terminate as *const c_void, t_void.as_ptr());
        objc_registerClassPair(delegate_class);
        let delegate = send0(send0(delegate_class, "alloc"), "init");
        send1(app, "setDelegate:", delegate);

        let bar = send0(send0(class("NSMenu"), "alloc"), "init");
        let app_item = send0(send0(class("NSMenuItem"), "alloc"), "init");
        send1(bar, "addItem:", app_item);
        let menu = send0(send0(class("NSMenu"), "alloc"), "init");
        let open = menu_item(crate::trs!("Oberfläche öffnen", "Open interface", "Ouvrir l'interface", "Abrir la interfaz"), Some("openInterface:"), "o");
        send1(open, "setTarget:", delegate);
        send1(menu, "addItem:", open);
        send1(menu, "addItem:", send0(class("NSMenuItem"), "separatorItem"));
        send1(menu, "addItem:", menu_item(crate::trs!("Reco Minimap beenden", "Quit Reco Minimap", "Quitter Reco Minimap", "Salir de Reco Minimap"), Some("terminate:"), "q"));
        send1(app_item, "setSubmenu:", menu);
        send1(app, "setMainMenu:", bar);

        send_bool(app, "activateIgnoringOtherApps:", true);
        send0(app, "run");
    }
    std::process::exit(0)
}
