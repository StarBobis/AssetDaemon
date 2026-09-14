/*
 * task_types.rs - Task system shared type definitions.
 *
 * Contains log levels, log entries, task status types shared by frontend and backend.
 * All types implement Serialize for transmission via Tauri Channel.
 *
 * Follows Soul.md conventions: enum + struct organization, no free functions.
 */

use serde::Serialize;

// ============================================================
// TaskLogLevel - Log level (corresponds to frontend LogEntry.type)
// ============================================================

/// Log level enum.
/// Serialized as lowercase strings, mapping 1:1 to frontend LogEntry.type's 'info'|'success'|'warn'|'error'.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskLogLevel {
    /// General information
    Info,
    /// Success / completion
    Success,
    /// Warning
    Warn,
    /// Error
    Error,
}

// ============================================================
// TaskLogEntry - A single log entry pushed from backend to frontend
// ============================================================

/// Log entry struct.
/// Backend commands broadcast instances of this struct via TaskLogger during execution,
/// and the frontend log panel appends them to the log list upon receipt.
/// The optional progress fields are used for progress bar display.
#[derive(Debug, Clone, Serialize)]
pub struct TaskLogEntry {
    /// Task ID that produced this log (for distinguishing concurrent task logs)
    pub task_id: String,
    /// Task type label (e.g. "Build Map", "Batch Scan", "Parse Bundle" etc.)
    pub task_label: String,
    /// Log level
    pub level: TaskLogLevel,
    /// Log message content
    pub message: String,
    /// Unix timestamp (milliseconds), populated by backend, used directly by frontend
    pub timestamp_ms: u64,
    /// Progress: current step description (optional, present means this log carries progress info)
    pub progress_step: Option<String>,
    /// Progress: current completion count (optional)
    pub progress_current: Option<usize>,
    /// Progress: total count (optional)
    pub progress_total: Option<usize>,
}

// ============================================================
// TaskInfo - Task summary (for querying running status, reserved for future enhancement)
// ============================================================

/// Task summary information.
/// Reserved for future use by the get_running_tasks_detail command.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize)]
pub struct TaskInfo {
    /// Task unique identifier
    pub task_id: String,
    /// Task type (e.g. "parse_bundle", "export_batch" etc.)
    pub task_type: String,
    /// Task status string: "running" | "cancelled" | "completed" | "failed"
    pub status: String,
    /// Task start timestamp (milliseconds)
    pub started_at_ms: u64,
}
