use crate::common::panic_reporter::PanicReporter;
use crate::common::task::task_context::TaskContext;

pub struct TaskRunner;

impl TaskRunner {
    fn is_cancelled_error(message: &str) -> bool {
        let lowered = message.to_ascii_lowercase();
        lowered.contains("task cancelled")
            || lowered.contains("task canceled")
            || lowered.contains("cancelled")
            || lowered.contains("canceled")
            || lowered.contains("interrupted")
    }

    /// Global panic fallback for background task workers.
    ///
    /// Catches any panic escaping the worker, makes sure the panic is written to
    /// the on-disk panic log, reveals the log in the OS file manager, notifies
    /// the frontend, and converts the panic into a normal task error so the
    /// application itself keeps running.
    fn report_worker_panic(
        ctx: &TaskContext,
        payload: &(dyn std::any::Any + Send + 'static),
    ) -> String {
        let payload_message = PanicReporter::payload_to_string(payload);
        let log_path = PanicReporter::handle_task_panic(ctx.task_label(), payload);
        let message = if log_path.is_empty() {
            format!(
                "Task '{}' crashed unexpectedly (panic): {}",
                ctx.task_label(),
                payload_message
            )
        } else {
            format!(
                "Task '{}' crashed unexpectedly (panic): {}. Panic log saved to: {}",
                ctx.task_label(),
                payload_message,
                log_path
            )
        };
        ctx.error(&message);
        message
    }

    pub fn create(prefix: &str, task_label: &str) -> TaskContext {
        TaskContext::new(prefix, task_label)
    }

    pub fn create_with_id(task_id: String, task_label: &str) -> TaskContext {
        TaskContext::with_id(task_id, task_label)
    }

    pub async fn run_blocking<T, F>(
        prefix: &str,
        task_label: &str,
        start_message: impl Into<String>,
        success_message: Option<String>,
        worker: F,
    ) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(TaskContext) -> Result<T, String> + Send + 'static,
    {
        let ctx = Self::create(prefix, task_label);
        ctx.info(start_message.into());
        Self::run_blocking_with_context(ctx, success_message, worker).await
    }

    pub async fn run_blocking_with_id<T, F>(
        task_id: String,
        task_label: &str,
        start_message: impl Into<String>,
        success_message: Option<String>,
        worker: F,
    ) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(TaskContext) -> Result<T, String> + Send + 'static,
    {
        let ctx = Self::create_with_id(task_id, task_label);
        ctx.info(start_message.into());
        Self::run_blocking_with_context(ctx, success_message, worker).await
    }

    pub async fn run_blocking_with_context<T, F>(
        ctx: TaskContext,
        success_message: Option<String>,
        worker: F,
    ) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(TaskContext) -> Result<T, String> + Send + 'static,
    {
        let worker_ctx = ctx.clone();
        let panic_ctx = ctx.clone();
        let result = tokio::task::spawn_blocking(move || {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| worker(worker_ctx)))
        })
        .await;
        match result {
            Ok(Ok(Ok(value))) => {
                if let Some(message) = success_message {
                    ctx.success(message);
                }
                Ok(value)
            }
            Ok(Ok(Err(error))) => {
                if Self::is_cancelled_error(&error) {
                    ctx.warn(&error);
                } else {
                    ctx.error(&error);
                }
                Err(error)
            }
            Ok(Err(payload)) => Err(Self::report_worker_panic(&panic_ctx, payload.as_ref())),
            Err(error) => {
                let message = format!("Task worker failed: {}", error);
                ctx.error(&message);
                Err(message)
            }
        }
    }

    pub fn spawn_blocking_detached<F>(ctx: TaskContext, worker: F)
    where
        F: FnOnce(TaskContext) -> Result<(), String> + Send + 'static,
    {
        tokio::task::spawn_blocking(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                worker(ctx.clone())
            }));
            match result {
                Ok(Ok(())) => {}
                Ok(Err(error)) => ctx.error(error),
                Err(payload) => {
                    let _ = Self::report_worker_panic(&ctx, payload.as_ref());
                }
            }
        });
    }
}
