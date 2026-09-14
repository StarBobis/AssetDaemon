/*
 * panic_reporter.rs -- Global panic capture and reporting.
 *
 * Installs a process-wide std::panic hook that appends every panic (thread name,
 * location, payload, backtrace) to a session panic log file under <app_data>/logs,
 * so crash reports survive even when the frontend log panel is unavailable.
 *
 * Also provides the task-level fallback used by TaskRunner: when a background
 * task dies from a panic, the panic log is revealed in the OS file manager and a
 * "panic-alert" event is emitted so the frontend can notify the user.
 *
 * Follows Soul.md conventions: struct + impl organization, no free functions.
 */

use serde::Serialize;
use std::any::Any;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tauri::{AppHandle, Emitter};

/// Payload pushed to the frontend when a background task dies from a panic.
#[derive(Clone, Serialize)]
pub struct PanicAlertEvent {
    pub task_label: String,
    pub message: String,
    pub log_path: String,
}

pub struct PanicReporter;

struct PanicReporterState {
    log_path: PathBuf,
    app: AppHandle,
}

static STATE: OnceLock<PanicReporterState> = OnceLock::new();

impl PanicReporter {
    pub const PANIC_ALERT_EVENT: &'static str = "panic-alert";

    /// Install the global panic hook and remember where panic logs are written.
    ///
    /// Must be called once during Tauri setup. The hook only performs best-effort
    /// file I/O and never panics itself; rich reporting (file manager reveal +
    /// frontend notification) happens in handle_task_panic at catch sites.
    pub fn install(logs_dir: PathBuf, app: AppHandle) {
        if STATE.get().is_some() {
            return;
        }

        let log_path = Self::create_session_log(&logs_dir);
        let _ = STATE.set(PanicReporterState {
            log_path,
            app: app.clone(),
        });

        std::panic::set_hook(Box::new(move |info| {
            // A panic inside the hook would abort the process, so the hook body
            // is fully guarded and every I/O result is ignored on failure.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let message = Self::payload_message(info.payload());
                let location = info
                    .location()
                    .map(|loc| format!("{}:{}:{}", loc.file(), loc.line(), loc.column()))
                    .unwrap_or_else(|| "unknown location".to_string());
                let thread_name = std::thread::current()
                    .name()
                    .unwrap_or("<unnamed>")
                    .to_string();

                eprintln!(
                    "thread '{}' panicked at {}:\n{}",
                    thread_name, location, message
                );

                if let Some(state) = STATE.get() {
                    let entry = format!(
                        "\n============================================================\n\
                         [{}] thread '{}' panicked at {}\n\
                         payload: {}\n\
                         backtrace:\n{}\n",
                        Self::timestamp(),
                        thread_name,
                        location,
                        message,
                        std::backtrace::Backtrace::force_capture()
                    );
                    Self::append_to_log(&state.log_path, &entry);
                }
            }));
        }));
    }

    /// Convert a catch_unwind panic payload into a readable string.
    pub fn payload_to_string(payload: &(dyn Any + Send + 'static)) -> String {
        Self::payload_message(payload)
    }

    /// Path of the current session panic log, if the reporter is installed.
    pub fn session_log_path() -> Option<PathBuf> {
        STATE.get().map(|state| state.log_path.clone())
    }

    /// Task-level fallback after a worker panic was caught.
    ///
    /// The panic hook has already appended full details to the session log; this
    /// reveals the log in the OS file manager and notifies the frontend so the
    /// user knows a crash happened and where to find the report.
    ///
    /// Returns the log file path to embed in error messages (empty when the log
    /// could not be determined).
    pub fn handle_task_panic(task_label: &str, payload: &(dyn Any + Send + 'static)) -> String {
        let message = Self::payload_to_string(payload);

        let log_path = match Self::session_log_path() {
            Some(path) => {
                // Defensive: make sure the fatal panic is present in the log even
                // if the hook entry was lost (e.g. hook installed after panic).
                let entry = format!(
                    "\n============================================================\n\
                     [{}] fatal panic in background task '{}' \n\
                     payload: {}\n",
                    Self::timestamp(),
                    task_label,
                    message
                );
                Self::append_to_log(&path, &entry);
                path
            }
            None => Self::write_fallback_log(task_label, &message),
        };

        let log_path_string = log_path.to_string_lossy().to_string();

        // Reveal the log file in the OS file manager from a detached thread so
        // the task thread is never blocked by shell/process quirks.
        let reveal_path = log_path.clone();
        std::thread::spawn(move || {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                Self::reveal_in_file_manager(&reveal_path);
            }));
        });

        // Notify the frontend so it can pop an alert dialog for the user.
        if let Some(state) = STATE.get() {
            let _ = state.app.emit(
                Self::PANIC_ALERT_EVENT,
                PanicAlertEvent {
                    task_label: task_label.to_string(),
                    message: message.clone(),
                    log_path: log_path_string.clone(),
                },
            );
        }

        log_path_string
    }

    // ------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------

    fn payload_message(payload: &(dyn Any + Send + 'static)) -> String {
        if let Some(message) = payload.downcast_ref::<&str>() {
            (*message).to_string()
        } else if let Some(message) = payload.downcast_ref::<String>() {
            message.clone()
        } else {
            "<non-string panic payload>".to_string()
        }
    }

    fn timestamp() -> String {
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string()
    }

    fn create_session_log(logs_dir: &Path) -> PathBuf {
        let dir = if fs::create_dir_all(logs_dir).is_ok() {
            logs_dir.to_path_buf()
        } else {
            let fallback = std::env::temp_dir().join("assetdaemon").join("logs");
            let _ = fs::create_dir_all(&fallback);
            fallback
        };
        let file_name = format!(
            "assetdaemon-panic-{}.txt",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        );
        let path = dir.join(file_name);
        let header = format!(
            "AssetDaemon panic log\nsession started: {}\n\
             Every panic captured by the global hook is appended below.\n",
            Self::timestamp()
        );
        let _ = fs::write(&path, header);
        path
    }

    fn append_to_log(path: &Path, entry: &str) {
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = file.write_all(entry.as_bytes());
            let _ = file.flush();
        }
    }

    fn write_fallback_log(task_label: &str, message: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("assetdaemon").join("logs");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join(format!(
            "assetdaemon-panic-{}.txt",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        ));
        let entry = format!(
            "AssetDaemon panic log (fallback, reporter not installed)\n\
             [{}] fatal panic in background task '{}'\npayload: {}\n",
            Self::timestamp(),
            task_label,
            message
        );
        let _ = fs::write(&path, entry);
        path
    }

    #[cfg(target_os = "windows")]
    fn reveal_in_file_manager(path: &Path) {
        // explorer /select,<path> opens the parent folder with the file selected.
        let _ = std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.to_string_lossy()))
            .spawn();
    }

    #[cfg(target_os = "macos")]
    fn reveal_in_file_manager(path: &Path) {
        let _ = std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn();
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    fn reveal_in_file_manager(path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = std::process::Command::new("xdg-open").arg(parent).spawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PanicReporter;

    #[test]
    fn payload_message_extracts_str_and_string() {
        let str_payload: Box<dyn std::any::Any + Send> = Box::new("plain panic");
        assert_eq!(
            PanicReporter::payload_to_string(str_payload.as_ref()),
            "plain panic"
        );

        let string_payload: Box<dyn std::any::Any + Send> = Box::new("owned panic".to_string());
        assert_eq!(
            PanicReporter::payload_to_string(string_payload.as_ref()),
            "owned panic"
        );

        let other_payload: Box<dyn std::any::Any + Send> = Box::new(42_i32);
        assert_eq!(
            PanicReporter::payload_to_string(other_payload.as_ref()),
            "<non-string panic payload>"
        );
    }
}
