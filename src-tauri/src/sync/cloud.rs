//! The way to CloudKit. On macOS built with the `icloud` feature it is the
//! Swift static library of packages/BangsCloud (the C ABI of docs/sync.md);
//! everywhere else there is nothing to talk to and `supported()` says so.
//!
//! None of these may be called while the hub's lock is held: the library can
//! answer through `on_event` on the very same thread, and `on_event` takes
//! that lock.

#[cfg(all(target_os = "macos", feature = "icloud"))]
mod native {
    use std::ffi::{c_char, CStr, CString};
    use std::path::Path;
    use std::sync::OnceLock;

    use tauri::AppHandle;

    extern "C" {
        fn bangs_cloud_supported() -> i32;
        fn bangs_cloud_start(state_dir: *const c_char, callback: extern "C" fn(*const c_char));
        fn bangs_cloud_push(records_json: *const c_char);
        fn bangs_cloud_pull();
        fn bangs_cloud_stop();
    }

    static APP: OnceLock<AppHandle> = OnceLock::new();

    extern "C" fn on_event(json: *const c_char) {
        if json.is_null() {
            return;
        }
        // Only valid during this call, so it is copied right away.
        let text = unsafe { CStr::from_ptr(json) }.to_string_lossy().into_owned();
        if let Some(app) = APP.get() {
            super::super::on_event(app, &text);
        }
    }

    /// True when this binary was signed with the iCloud entitlements. Asked
    /// before anything else: CloudKit crashes the process, uncatchably, when
    /// it is used without them.
    pub fn supported() -> bool {
        unsafe { bangs_cloud_supported() == 1 }
    }

    pub fn start(app: &AppHandle, dir: &Path) {
        let _ = APP.set(app.clone());
        if let Ok(dir) = CString::new(dir.to_string_lossy().into_owned()) {
            unsafe { bangs_cloud_start(dir.as_ptr(), on_event) };
        }
    }

    pub fn push(records_json: &str) {
        if let Ok(json) = CString::new(records_json) {
            unsafe { bangs_cloud_push(json.as_ptr()) };
        }
    }

    pub fn pull() {
        unsafe { bangs_cloud_pull() };
    }

    pub fn stop() {
        unsafe { bangs_cloud_stop() };
    }
}

#[cfg(not(all(target_os = "macos", feature = "icloud")))]
mod native {
    use std::path::Path;

    use tauri::AppHandle;

    pub fn supported() -> bool {
        false
    }
    pub fn start(_app: &AppHandle, _dir: &Path) {}
    pub fn push(_records_json: &str) {}
    pub fn pull() {}
    pub fn stop() {}
}

pub use native::*;
