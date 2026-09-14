/*
 * task_logger.rs - Global log broadcaster.
 *
 * Responsible for pushing backend task log messages (with optional progress info)
 * to the frontend UI log panel.
 * Uses a global LazyLock<Mutex<Vec<Channel>>> to store all registered frontend Channels.
 * The frontend registers a persistent Channel on page load, shared by all backend tasks.
 *
 * Follows Soul.md conventions: struct + impl organization, no free functions.
 */
use crate::common::task::task_types::{TaskLogEntry, TaskLogLevel};
use std::sync::{LazyLock, Mutex};
use tauri::ipc::Channel;

pub struct TaskLogger;

fn log_channels() -> &'static Mutex<Vec<Channel<TaskLogEntry>>> {
    static CHANNELS: LazyLock<Mutex<Vec<Channel<TaskLogEntry>>>> =
        LazyLock::new(|| Mutex::new(Vec::new()));
    &CHANNELS
}

/// Lock the channel registry, tolerating a poisoned mutex.
///
/// A panic in any thread that briefly held this lock (e.g. while cloning the
/// channel list) must not turn every subsequent log call into another panic.
fn lock_channels() -> std::sync::MutexGuard<'static, Vec<Channel<TaskLogEntry>>> {
    log_channels()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl TaskLogger {
    pub fn register_channel(channel: Channel<TaskLogEntry>) {
        lock_channels().push(channel);
    }

    /// Send a log (with optional progress info).
    pub fn log(
        task_id: &str,
        task_label: &str,
        level: TaskLogLevel,
        message: &str,
        progress_current: Option<usize>,
        progress_total: Option<usize>,
        progress_step: Option<&str>,
    ) {
        let entry = TaskLogEntry {
            task_id: task_id.to_string(),
            task_label: task_label.to_string(),
            level,
            message: message.to_string(),
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            progress_step: progress_step.map(|s| s.to_string()),
            progress_current,
            progress_total,
        };

        let channels_snapshot = {
            let channels = lock_channels();
            if channels.is_empty() {
                return;
            }
            channels.clone()
        };

        let mut dead_indices: Vec<usize> = Vec::new();
        for (i, ch) in channels_snapshot.iter().enumerate() {
            if ch.send(entry.clone()).is_err() {
                dead_indices.push(i);
            }
        }
        if !dead_indices.is_empty() {
            let mut channels = lock_channels();
            for idx in dead_indices.into_iter().rev() {
                if idx < channels.len() {
                    channels.remove(idx);
                }
            }
        }
    }

    /// Send a log with progress info (convenience method).
    pub fn progress(
        task_id: &str,
        task_label: &str,
        step: &str,
        current: usize,
        total: usize,
        message: &str,
    ) {
        Self::log(
            task_id,
            task_label,
            TaskLogLevel::Info,
            message,
            Some(current),
            Some(total),
            Some(step),
        );
    }

    /// Send an info log without progress.
    pub fn info(task_id: &str, task_label: &str, message: &str) {
        Self::log(
            task_id,
            task_label,
            TaskLogLevel::Info,
            message,
            None,
            None,
            None,
        );
    }

    /// Send a success log without progress.
    pub fn success(task_id: &str, task_label: &str, message: &str) {
        Self::log(
            task_id,
            task_label,
            TaskLogLevel::Success,
            message,
            None,
            None,
            None,
        );
    }

    /// Send a warn log without progress.
    pub fn warn(task_id: &str, task_label: &str, message: &str) {
        Self::log(
            task_id,
            task_label,
            TaskLogLevel::Warn,
            message,
            None,
            None,
            None,
        );
    }

    /// Send an error log without progress.
    pub fn error(task_id: &str, task_label: &str, message: &str) {
        Self::log(
            task_id,
            task_label,
            TaskLogLevel::Error,
            message,
            None,
            None,
            None,
        );
    }
}
