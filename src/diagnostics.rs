//! Small persistent startup report. Never record command-line arguments or credentials.
use std::{io::Write, path::{Path, PathBuf}, sync::OnceLock};

static REPORT: OnceLock<PathBuf> = OnceLock::new();

pub fn install(directory: &Path) {
    let path = directory.join("startup-diagnostics.log");
    if REPORT.set(path.clone()).is_err() { return; }
    // Bound the report without discarding the immediately previous launch.
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 1_048_576) {
        let _ = std::fs::remove_file(directory.join("startup-diagnostics.previous.log"));
        let _ = std::fs::rename(&path, directory.join("startup-diagnostics.previous.log"));
    }
    record(&format!("launch {} {}", std::env::consts::OS, std::env::consts::ARCH));
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Panic payloads can contain arbitrary user data. Capture location/stack only.
        record(&format!("panic at {:?}\n{}", info.location(), std::backtrace::Backtrace::force_capture()));
        previous(info);
    }));
}

pub fn path() -> String {
    REPORT.get().map(|p| p.display().to_string()).unwrap_or_else(|| "stderr (report unavailable)".into())
}

pub fn record(message: &str) {
    if let Some(path) = REPORT.get()
        && let Ok(mut out) = std::fs::OpenOptions::new().create(true).append(true).open(path)
    {
        let time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        let _ = writeln!(out, "[{time}] {message}");
    }
}

#[cfg(windows)]
pub fn show_fatal(message: &str) {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBoxW(window: *mut std::ffi::c_void, text: *const u16, caption: *const u16, flags: u32) -> i32;
    }
    let text: Vec<u16> = format!("{message}\n\nDiagnostic report: {}", path()).encode_utf16().chain(Some(0)).collect();
    let title: Vec<u16> = "AIContinuum could not continue".encode_utf16().chain(Some(0)).collect();
    // SAFETY: both UTF-16 buffers are terminated and live for the modal call.
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), 0x10); }
}
