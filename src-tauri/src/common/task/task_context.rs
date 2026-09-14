use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::common::task::task_logger::TaskLogger;
use crate::common::task::task_manager::TaskManager;

struct TaskState {
    task_id: String,
    task_label: String,
    cancel_token: Arc<AtomicBool>,
    unregister_on_drop: bool,
}

impl Drop for TaskState {
    fn drop(&mut self) {
        if self.unregister_on_drop {
            TaskManager::unregister(&self.task_id);
        }
    }
}

#[derive(Clone)]
pub struct TaskContext {
    state: Arc<TaskState>,
}

impl TaskContext {
    pub fn new(prefix: &str, task_label: &str) -> Self {
        let task_id = TaskManager::generate_id(prefix);
        Self::with_id(task_id, task_label)
    }

    pub fn with_id(task_id: String, task_label: &str) -> Self {
        let cancel_token = TaskManager::register(&task_id);
        Self::with_cancel_token(task_id, task_label, cancel_token, true)
    }

    pub fn borrowed(task_id: String, task_label: &str, cancel_token: Arc<AtomicBool>) -> Self {
        Self::with_cancel_token(task_id, task_label, cancel_token, false)
    }

    fn with_cancel_token(
        task_id: String,
        task_label: &str,
        cancel_token: Arc<AtomicBool>,
        unregister_on_drop: bool,
    ) -> Self {
        Self {
            state: Arc::new(TaskState {
                task_id,
                task_label: task_label.to_string(),
                cancel_token,
                unregister_on_drop,
            }),
        }
    }

    pub fn task_id(&self) -> &str {
        &self.state.task_id
    }

    pub fn task_label(&self) -> &str {
        &self.state.task_label
    }

    pub fn cancel_token(&self) -> Arc<AtomicBool> {
        self.state.cancel_token.clone()
    }

    pub fn is_cancelled(&self) -> bool {
        self.state.cancel_token.load(Ordering::SeqCst)
    }

    pub fn check_cancelled(&self) -> Result<(), String> {
        if self.is_cancelled() {
            self.warn("Task cancelled");
            Err("Task cancelled".to_string())
        } else {
            Ok(())
        }
    }

    pub fn progress(&self, step: &str, current: usize, total: usize, message: impl AsRef<str>) {
        TaskLogger::progress(
            self.task_id(),
            self.task_label(),
            step,
            current,
            total.max(1),
            message.as_ref(),
        );
    }

    pub fn info(&self, message: impl AsRef<str>) {
        TaskLogger::info(self.task_id(), self.task_label(), message.as_ref());
    }

    pub fn success(&self, message: impl AsRef<str>) {
        TaskLogger::success(self.task_id(), self.task_label(), message.as_ref());
    }

    pub fn warn(&self, message: impl AsRef<str>) {
        TaskLogger::warn(self.task_id(), self.task_label(), message.as_ref());
    }

    pub fn error(&self, message: impl AsRef<str>) {
        TaskLogger::error(self.task_id(), self.task_label(), message.as_ref());
    }
}
