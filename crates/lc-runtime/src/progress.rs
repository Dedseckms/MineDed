use lc_types::task::TaskEvent;
use lc_types::{AppError, AppResult, ErrorCode, TaskId};

pub use tokio_util::sync::CancellationToken as CancelToken;

/// Receives progress of one long-running task. Implemented in `src-tauri` on
/// top of `tauri::ipc::Channel<TaskEvent>`; services only see this trait.
pub trait ProgressSink: Send + Sync {
    fn task_id(&self) -> TaskId;
    fn emit(&self, event: TaskEvent);
    fn cancel_token(&self) -> &CancelToken;

    fn progress(&self, done: u64, total: Option<u64>, stage: &str) {
        self.emit(TaskEvent::Progress {
            id: self.task_id(),
            done,
            total,
            stage: stage.to_owned(),
            speed_bps: None,
        });
    }

    fn log(&self, line: &str) {
        self.emit(TaskEvent::Log {
            id: self.task_id(),
            line: line.to_owned(),
        });
    }

    /// Call between steps; returns `Cancelled` once the user cancelled.
    fn check_cancelled(&self) -> AppResult<()> {
        if self.cancel_token().is_cancelled() {
            Err(AppError::new(ErrorCode::Cancelled, "cancelled by user"))
        } else {
            Ok(())
        }
    }
}

/// Sink that drops everything. For tests and fire-and-forget calls.
#[derive(Debug, Default)]
pub struct NullSink {
    id: TaskId,
    cancel: CancelToken,
}

impl ProgressSink for NullSink {
    fn task_id(&self) -> TaskId {
        self.id
    }

    fn emit(&self, _event: TaskEvent) {}

    fn cancel_token(&self) -> &CancelToken {
        &self.cancel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_is_reported() {
        let sink = NullSink::default();
        assert!(sink.check_cancelled().is_ok());
        sink.cancel_token().cancel();
        assert_eq!(
            sink.check_cancelled().unwrap_err().code,
            ErrorCode::Cancelled
        );
    }
}
