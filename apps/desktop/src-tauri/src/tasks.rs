//! Long-running work: every command that may take >100 ms spawns a task,
//! returns its `TaskId` immediately and streams `TaskEvent`s to the caller's
//! `Channel`. The UI never awaits the work itself.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use lc_runtime::{CancelToken, ProgressSink};
use lc_types::task::{TaskEvent, TaskKind, TaskStatus, TaskSummary};
use lc_types::{AppError, AppResult, ErrorCode, TaskId};
use tauri::ipc::Channel;
use tauri::AppHandle;
use tauri_specta::Event;

use crate::events::TasksChangedEvent;

struct Entry {
    summary: TaskSummary,
    cancel: CancelToken,
}

#[derive(Default)]
pub struct TaskRegistry {
    inner: Mutex<HashMap<TaskId, Entry>>,
}

impl TaskRegistry {
    pub fn list(&self) -> Vec<TaskSummary> {
        let map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut v: Vec<_> = map.values().map(|e| e.summary.clone()).collect();
        v.sort_by_key(|s| s.id);
        v
    }

    pub fn cancel(&self, id: TaskId) -> AppResult<()> {
        let map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let entry = map
            .get(&id)
            .ok_or_else(|| AppError::new(ErrorCode::NotFound, format!("task {id}")))?;
        entry.cancel.cancel();
        Ok(())
    }

    fn insert(&self, summary: TaskSummary, cancel: CancelToken) {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        map.insert(summary.id, Entry { summary, cancel });
    }

    fn update(&self, event: &TaskEvent) {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let Some(entry) = map.get_mut(&event.id()) else {
            return;
        };
        match event {
            TaskEvent::Progress {
                done, total, stage, ..
            } => {
                entry.summary.done = *done;
                entry.summary.total = *total;
                entry.summary.stage.clone_from(stage);
            }
            TaskEvent::Finished { error, .. } => {
                entry.summary.status = match error {
                    None => TaskStatus::Succeeded,
                    Some(e) if e.code == ErrorCode::Cancelled => TaskStatus::Cancelled,
                    Some(_) => TaskStatus::Failed,
                };
            }
            TaskEvent::Started { .. } | TaskEvent::Log { .. } => {}
        }
    }
}

/// `ProgressSink` backed by a Tauri channel + the global registry.
pub struct ChannelSink {
    id: TaskId,
    channel: Channel<TaskEvent>,
    cancel: CancelToken,
    registry: Arc<TaskRegistry>,
}

impl ProgressSink for ChannelSink {
    fn task_id(&self) -> TaskId {
        self.id
    }

    fn emit(&self, event: TaskEvent) {
        self.registry.update(&event);
        // The window may be gone; the task still runs to completion.
        let _ = self.channel.send(event);
    }

    fn cancel_token(&self) -> &CancelToken {
        &self.cancel
    }
}

fn notify(app: &AppHandle, registry: &TaskRegistry) {
    let _ = TasksChangedEvent(registry.list()).emit(app);
}

/// Spawns `work` on the async runtime and returns its id immediately.
pub fn spawn_task<F, Fut>(
    app: &AppHandle,
    registry: &Arc<TaskRegistry>,
    kind: TaskKind,
    title_key: &str,
    channel: Channel<TaskEvent>,
    work: F,
) -> TaskId
where
    F: FnOnce(Arc<ChannelSink>) -> Fut + Send + 'static,
    Fut: Future<Output = AppResult<()>> + Send + 'static,
{
    let id = TaskId::new();
    let cancel = CancelToken::new();
    registry.insert(
        TaskSummary {
            id,
            kind,
            title_key: title_key.to_owned(),
            status: TaskStatus::Running,
            done: 0,
            total: None,
            stage: String::new(),
        },
        cancel.clone(),
    );
    let sink = Arc::new(ChannelSink {
        id,
        channel,
        cancel,
        registry: registry.clone(),
    });
    sink.emit(TaskEvent::Started {
        id,
        kind,
        title_key: title_key.to_owned(),
    });
    notify(app, registry);

    let app = app.clone();
    let registry = registry.clone();
    tauri::async_runtime::spawn(async move {
        let error = work(sink.clone()).await.err();
        sink.emit(TaskEvent::Finished { id, error });
        notify(&app, &registry);
    });
    id
}
