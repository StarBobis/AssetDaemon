/*
 * task_commands.rs -- Task system Tauri commands.
 *
 * Provides task management entry points for frontend calls:
 *  - register_log_channel: Register a persistent log Channel
 *  - stop_all_tasks:       Stop all running background tasks
 *  - get_running_tasks:    Query the list of currently running tasks
 *
 * Follows Soul.md convention: commands and service classes are separated; this file only contains #[tauri::command] functions.
 */

use tauri::ipc::Channel;

use crate::common::task::task_logger::TaskLogger;
use crate::common::task::task_manager::TaskManager;
use crate::common::task::task_types::TaskLogEntry;

// ================================================================
// Log Channel Registration
// ================================================================

/// Register the frontend log Channel.
///
/// The frontend calls this command once in WorkSpace page's onMounted,
/// passing a persistent Tauri IPC Channel to the backend store.
/// Afterwards, logs from all background tasks are pushed to the frontend log panel through this Channel.
///
/// @param channel  IPC Channel for receiving TaskLogEntry
/// @return         Returns Ok on success, error message on failure
#[tauri::command]
pub fn register_log_channel(channel: Channel<TaskLogEntry>) -> Result<(), String> {
    TaskLogger::register_channel(channel);
    TaskLogger::info(
        "system",
        "system",
        "Log Channel registered, backend logs will be pushed to the frontend panel in real time",
    );
    Ok(())
}

// ================================================================
// Task Control
// ================================================================

/// Stop all running background tasks.
///
/// Sets the cancellation token of all registered tasks to true.
/// Each task terminates execution and unregisters upon detecting cancellation at the next checkpoint.
/// This command returns immediately without waiting for tasks to actually terminate.
///
/// @return  Number of tasks marked for cancellation. 0 means no tasks are currently running.
#[tauri::command]
pub fn stop_all_tasks() -> Result<usize, String> {
    let count = TaskManager::cancel_all();
    Ok(count)
}

// ================================================================
// Task Query
// ================================================================

/// Get the list of all currently running task IDs.
///
/// @return  List of task ID strings that are registered but not yet unregistered
#[tauri::command]
pub fn get_running_tasks() -> Result<Vec<String>, String> {
    Ok(TaskManager::running_task_ids())
}
