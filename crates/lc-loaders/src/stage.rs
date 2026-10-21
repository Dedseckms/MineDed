//! Sink wrapper that labels downloader progress with an i18n stage key.

use lc_runtime::{CancelToken, ProgressSink};
use lc_types::task::TaskEvent;
use lc_types::TaskId;

pub(crate) struct StageSink<'a> {
    inner: &'a dyn ProgressSink,
    stage: &'static str,
}

impl<'a> StageSink<'a> {
    pub fn new(inner: &'a dyn ProgressSink, stage: &'static str) -> Self {
        Self { inner, stage }
    }
}

impl ProgressSink for StageSink<'_> {
    fn task_id(&self) -> TaskId {
        self.inner.task_id()
    }

    fn emit(&self, event: TaskEvent) {
        match event {
            TaskEvent::Progress {
                id,
                done,
                total,
                speed_bps,
                ..
            } => self.inner.emit(TaskEvent::Progress {
                id,
                done,
                total,
                stage: self.stage.to_owned(),
                speed_bps,
            }),
            other => self.inner.emit(other),
        }
    }

    fn cancel_token(&self) -> &CancelToken {
        self.inner.cancel_token()
    }
}
