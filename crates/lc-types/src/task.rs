use serde::{Deserialize, Serialize};
use specta::Type;

use crate::{AppError, TaskId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum TaskKind {
    JavaInstall,
    InstanceCreate,
    InstanceRepair,
    InstanceImport,
    InstanceExport,
    Launch,
    ContentInstall,
    ContentUpdate,
    PresetApply,
    AccountLogin,
    VpnUpdate,
    AppUpdate,
}

/// Streamed through `tauri::ipc::Channel<TaskEvent>` for every long operation.
/// A task always ends with exactly one `finished` event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TaskEvent {
    Started {
        id: TaskId,
        kind: TaskKind,
        /// i18n key, e.g. `tasks.instanceCreate`.
        title_key: String,
    },
    Progress {
        id: TaskId,
        done: u64,
        total: Option<u64>,
        /// i18n key of the current stage, e.g. `stages.libraries`.
        stage: String,
        speed_bps: Option<u64>,
    },
    Log {
        id: TaskId,
        line: String,
    },
    Finished {
        id: TaskId,
        /// `None` = success. Results are re-queried by the UI.
        error: Option<AppError>,
    },
}

impl TaskEvent {
    pub fn id(&self) -> TaskId {
        match self {
            Self::Started { id, .. }
            | Self::Progress { id, .. }
            | Self::Log { id, .. }
            | Self::Finished { id, .. } => *id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum TaskStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

/// Row of the global task center (`tasks_list`, `tasks://changed`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TaskSummary {
    pub id: TaskId,
    pub kind: TaskKind,
    pub title_key: String,
    pub status: TaskStatus,
    pub done: u64,
    pub total: Option<u64>,
    pub stage: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tagged_camel_case() {
        let e = TaskEvent::Progress {
            id: TaskId::new(),
            done: 1,
            total: Some(2),
            stage: "stages.libraries".into(),
            speed_bps: None,
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["type"], "progress");
        assert!(v.get("speedBps").is_some());
    }
}
