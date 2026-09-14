/*
 * task_manager.rs - Global Task Manager.
 *
 * Responsible for managing registration, cancellation, and unregistration of all background tasks.
 * Uses a global LazyLock<Mutex<HashMap>> to store the task registry.
 * Each registered task receives an Arc<AtomicBool> cancellation token.
 *
 * Follows Soul.md conventions: organized via struct + impl, no free functions.
 */

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use crate::common::task::task_logger::TaskLogger;

// ============================================================
// TaskManager - Global Task Registry
// ============================================================

/// Global task manager utility.
///
/// Uses a module-level LazyLock to store the registry, ensuring only one instance exists
/// throughout the application lifecycle.
/// All tasks that need cancellation and tracking should be registered through this manager.
pub struct TaskManager;

/// Atomic counter for generating unique task IDs
static TASK_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Global task registry: task_id -> cancellation token
/// Each entry in the registry identifies a running background task.
fn task_registry() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static REGISTRY: LazyLock<Mutex<HashMap<String, Arc<AtomicBool>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));
    &REGISTRY
}

/// Lock the registry, tolerating a poisoned mutex.
///
/// After any panic that happened while the registry was locked, subsequent
/// task operations must keep working instead of cascading into more panics.
fn lock_registry() -> std::sync::MutexGuard<'static, HashMap<String, Arc<AtomicBool>>> {
    task_registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl TaskManager {
    // ================================================================
    // Task ID Generation
    // ================================================================

    /// Generate a unique task ID.
    ///
    /// Format: "{prefix}_{timestamp_ms}_{counter}"
    /// e.g. "parse_bundle_1717000000000_42"
    ///
    /// @param prefix  Task type prefix (e.g. "parse_bundle", "export_batch", etc.)
    /// @return        Globally unique task identifier string
    pub fn generate_id(prefix: &str) -> String {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let count = TASK_COUNTER.fetch_add(1, Ordering::Relaxed);
        format!("{}_{}_{}", prefix, timestamp, count)
    }

    // ================================================================
    // Task Lifecycle Management
    // ================================================================

    /// Register a new task and return its cancellation token.
    ///
    /// Tasks should call this method before starting time-consuming operations.
    /// If the same ID already exists (rare collision), the old task will be overwritten.
    ///
    /// @param task_id  Task unique identifier
    /// @return         Cancellation token (Arc<AtomicBool>), tasks should check this token in loops
    pub fn register(task_id: &str) -> Arc<AtomicBool> {
        let token = Arc::new(AtomicBool::new(false));
        lock_registry().insert(task_id.to_string(), token.clone());
        token
    }

    /// Cancel a specific task.
    ///
    /// Sets the task's cancellation token to true and immediately removes it from the registry,
    /// so that queries like get_running_tasks no longer return this task.
    ///
    /// @param task_id  ID of the task to cancel
    /// @return         true if the task existed and was marked cancelled, false if not found
    pub fn cancel(task_id: &str) -> bool {
        let mut registry = lock_registry();
        if let Some(token) = registry.get(task_id) {
            token.store(true, Ordering::SeqCst);
            registry.remove(task_id);
            true
        } else {
            false
        }
    }

    /// Cancel all running tasks.
    ///
    /// Sets the cancellation token of all registered tasks to true and immediately removes them
    /// from the registry, so that query methods like get_running_tasks return an empty list
    /// immediately, preventing the frontend from spinning.
    /// Background threads detecting the cancellation flag will stop on their own; at that point
    /// unregister is a no-op.
    ///
    /// @return  Number of cancelled tasks
    pub fn cancel_all() -> usize {
        let count = {
            let mut registry = lock_registry();
            let count = registry.len();
            for (_, token) in registry.iter() {
                token.store(true, Ordering::SeqCst);
            }
            registry.clear();
            count
        };
        // Send logs after releasing the lock to avoid blocking other operations with
        // Channel::send() while holding the lock
        if count > 0 {
            TaskLogger::info(
                "system",
                "System",
                &format!("Cancellation requested for {} running tasks", count),
            );
        }
        count
    }

    /// Unregister a completed task.
    ///
    /// Tasks should call this method after completion (including normal finish, cancellation,
    /// or failure).
    /// After unregistration, the cancellation token is released and the task no longer appears
    /// in the running list.
    ///
    /// @param task_id  ID of the task to unregister
    pub fn unregister(task_id: &str) {
        lock_registry().remove(task_id);
    }

    // ================================================================
    // Query Methods
    // ================================================================

    /// Check whether a specific task has been cancelled.
    ///
    /// @param task_id  ID of the task to check
    /// @return         true if the task has been marked cancelled
    #[allow(dead_code)]
    pub fn is_cancelled(task_id: &str) -> bool {
        lock_registry()
            .get(task_id)
            .map(|t| t.load(Ordering::SeqCst))
            .unwrap_or(false)
    }

    /// Get all running task IDs.
    ///
    /// @return  All registered but not yet unregistered task IDs
    pub fn running_task_ids() -> Vec<String> {
        lock_registry().keys().cloned().collect()
    }

    /// Get the number of running tasks.
    ///
    /// @return  Total number of registered tasks
    #[allow(dead_code)]
    pub fn running_count() -> usize {
        lock_registry().len()
    }

    /// Check if any tasks are running.
    ///
    /// @return  true if there is at least one registered task
    #[allow(dead_code)]
    pub fn has_running() -> bool {
        !lock_registry().is_empty()
    }
}
