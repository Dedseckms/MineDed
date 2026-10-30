//! Chat IPC adapters. Authorization and privacy are enforced by the service.

use crate::state::AppState;
use crate::tasks::spawn_task;
use lc_types::social::{
    AdminPresence, AdminServerState, AuditEntry, ModAction, PresenceSummary, ProfileUpdate,
    SocialChannel, SocialContent, SocialFriend, SocialHistory, SocialMessage, SocialPrivacy,
    SocialReport, SocialRole, SocialStatus, SocialUser, StorageTarget,
};
use lc_types::task::{TaskEvent, TaskKind};
use lc_types::{AccountId, AppError, AppResult, ErrorCode, InstanceId, TaskId};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};

/// Publishes a real P2P offer; an existing unrelated room is never replaced.
#[tauri::command]
#[specta::specta]
pub async fn social_share_pack(
    state: State<'_, AppState>,
    channel: String,
    instance: InstanceId,
) -> AppResult<SocialMessage> {
    let allowed = state.social.channels().await?.into_iter().any(|entry| {
        entry.id == channel
            && entry.kind == lc_types::social::SocialChannelKind::Direct
            && entry.peer.is_some_and(|peer| peer.can_invite)
    });
    if !allowed {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Pack invitations are not allowed",
        ));
    }
    let info = state.instances.get(instance).await?;
    let mods = state
        .content
        .list(instance)
        .await?
        .into_iter()
        .filter(|item| item.kind == lc_types::content::ContentKind::Mod)
        .count();
    let room = state.rooms.share_pack(instance).await?;
    let loader = match info.loader.kind {
        lc_types::version::LoaderKind::Vanilla => "vanilla",
        lc_types::version::LoaderKind::Fabric => "fabric",
        lc_types::version::LoaderKind::Quilt => "quilt",
        lc_types::version::LoaderKind::Forge => "forge",
        lc_types::version::LoaderKind::NeoForge => "neoForge",
    };
    state
        .social
        .send(
            &channel,
            SocialContent::PackShare {
                code: Some(room.code),
                password_protected: room.password_protected,
                name: info.name,
                mc: info.mc,
                loader: loader.to_owned(),
                mods: u32::try_from(mods).unwrap_or(u32::MAX),
            },
        )
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn social_attachment_preview(
    state: State<'_, AppState>,
    attachment_id: String,
) -> AppResult<String> {
    state.social.attachment_preview(&attachment_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn social_connect(
    state: State<'_, AppState>,
    account: AccountId,
) -> AppResult<SocialUser> {
    state.social.connect(account).await
}

#[tauri::command]
#[specta::specta]
pub async fn social_disconnect(state: State<'_, AppState>) -> AppResult<()> {
    state.social.disconnect().await
}

#[tauri::command]
#[specta::specta]
pub async fn social_set_presence(
    state: State<'_, AppState>,
    status: SocialStatus,
) -> AppResult<()> {
    state.social.set_presence(status).await
}

#[tauri::command]
#[specta::specta]
pub async fn social_channels(state: State<'_, AppState>) -> AppResult<Vec<SocialChannel>> {
    state.social.channels().await
}

#[tauri::command]
#[specta::specta]
pub async fn social_history(
    state: State<'_, AppState>,
    channel: String,
    before: Option<String>,
) -> AppResult<SocialHistory> {
    state.social.history(&channel, before).await
}

#[tauri::command]
#[specta::specta]
pub async fn social_mark_read(
    state: State<'_, AppState>,
    channel: String,
    message_id: String,
) -> AppResult<()> {
    state.social.mark_read(&channel, &message_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn social_send(
    state: State<'_, AppState>,
    channel: String,
    content: SocialContent,
) -> AppResult<SocialMessage> {
    state.social.send(&channel, content).await
}

#[tauri::command]
#[specta::specta]
pub async fn social_delete_message(
    state: State<'_, AppState>,
    message_id: String,
) -> AppResult<()> {
    state.social.delete_message(&message_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn social_open_direct(
    state: State<'_, AppState>,
    uuid: String,
) -> AppResult<SocialChannel> {
    state.social.open_direct(&uuid).await
}

#[tauri::command]
#[specta::specta]
pub async fn social_search_users(
    state: State<'_, AppState>,
    query: String,
) -> AppResult<Vec<SocialUser>> {
    state.social.search_users(&query).await
}

#[tauri::command]
#[specta::specta]
pub async fn social_profile(state: State<'_, AppState>, uuid: String) -> AppResult<SocialUser> {
    state.social.profile(&uuid).await
}

#[tauri::command]
#[specta::specta]
pub async fn social_friends(state: State<'_, AppState>) -> AppResult<Vec<SocialFriend>> {
    state.social.friends().await
}

#[tauri::command]
#[specta::specta]
pub async fn social_add_friend(
    state: State<'_, AppState>,
    uuid: String,
) -> AppResult<SocialFriend> {
    state.social.add_friend(&uuid).await
}

#[tauri::command]
#[specta::specta]
pub async fn social_remove_friend(state: State<'_, AppState>, uuid: String) -> AppResult<()> {
    state.social.remove_friend(&uuid).await
}

#[tauri::command]
#[specta::specta]
pub async fn social_set_blocked(
    state: State<'_, AppState>,
    uuid: String,
    blocked: bool,
) -> AppResult<()> {
    state.social.set_blocked(&uuid, blocked).await
}

#[tauri::command]
#[specta::specta]
pub async fn social_privacy(state: State<'_, AppState>) -> AppResult<SocialPrivacy> {
    state.social.privacy().await
}

#[tauri::command]
#[specta::specta]
pub async fn social_set_privacy(
    state: State<'_, AppState>,
    privacy: SocialPrivacy,
) -> AppResult<SocialPrivacy> {
    state.social.set_privacy(privacy).await
}

#[tauri::command]
#[specta::specta]
pub async fn social_report(
    state: State<'_, AppState>,
    message_id: String,
    reason: Option<String>,
) -> AppResult<()> {
    state.social.report(&message_id, reason).await
}

#[tauri::command]
#[specta::specta]
pub async fn social_reports(state: State<'_, AppState>) -> AppResult<Vec<SocialReport>> {
    state.social.reports().await
}

#[tauri::command]
#[specta::specta]
pub async fn social_resolve_report(state: State<'_, AppState>, report_id: String) -> AppResult<()> {
    state.social.resolve_report(&report_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn social_moderate_delete(
    state: State<'_, AppState>,
    message_id: String,
) -> AppResult<()> {
    state.social.moderate_delete(&message_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn social_moderate_user(
    state: State<'_, AppState>,
    uuid: String,
    action: ModAction,
    seconds: Option<u64>,
) -> AppResult<()> {
    state.social.moderate_user(&uuid, action, seconds).await
}

#[tauri::command]
#[specta::specta]
pub async fn social_set_role(
    state: State<'_, AppState>,
    uuid: String,
    role: SocialRole,
) -> AppResult<()> {
    state.social.set_role(&uuid, role).await
}

#[tauri::command]
#[specta::specta]
pub async fn social_server_state(state: State<'_, AppState>) -> AppResult<AdminServerState> {
    state.social.server_state().await
}

#[tauri::command]
#[specta::specta]
pub async fn social_migrate_storage(
    state: State<'_, AppState>,
    target: StorageTarget,
) -> AppResult<()> {
    state.social.migrate_storage(target).await
}

#[tauri::command]
#[specta::specta]
pub async fn social_send_file(
    app: AppHandle,
    state: State<'_, AppState>,
    channel: String,
    path: String,
    caption: Option<String>,
    on_event: Channel<TaskEvent>,
) -> AppResult<TaskId> {
    let svc = state.social.clone();
    Ok(spawn_task(
        &app,
        &state.tasks,
        TaskKind::ChatTransfer,
        "tasks.chatSendFile",
        on_event,
        move |sink| async move {
            svc.send_file(&channel, &path, caption, &*sink)
                .await
                .map(|_| ())
        },
    ))
}

#[tauri::command]
#[specta::specta]
pub async fn social_save_attachment(
    app: AppHandle,
    state: State<'_, AppState>,
    attachment_id: String,
    dest: String,
    on_event: Channel<TaskEvent>,
) -> AppResult<TaskId> {
    let svc = state.social.clone();
    Ok(spawn_task(
        &app,
        &state.tasks,
        TaskKind::ChatTransfer,
        "tasks.chatSaveFile",
        on_event,
        move |sink| async move { svc.save_attachment(&attachment_id, &dest, &*sink).await },
    ))
}

#[tauri::command]
#[specta::specta]
pub async fn social_guest_login(state: State<'_, AppState>, name: String) -> AppResult<SocialUser> {
    state.social.guest_login(&name).await
}

#[tauri::command]
#[specta::specta]
pub async fn social_profile_update(
    state: State<'_, AppState>,
    patch: ProfileUpdate,
) -> AppResult<SocialUser> {
    state.social.update_profile(patch).await
}

#[tauri::command]
#[specta::specta]
pub async fn social_avatar_set(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    on_event: Channel<TaskEvent>,
) -> AppResult<TaskId> {
    let svc = state.social.clone();
    Ok(spawn_task(
        &app,
        &state.tasks,
        TaskKind::ChatTransfer,
        "tasks.chatAvatar",
        on_event,
        move |sink| async move { svc.set_avatar(&path, &*sink).await.map(|_| ()) },
    ))
}

#[tauri::command]
#[specta::specta]
pub async fn social_avatar_clear(state: State<'_, AppState>) -> AppResult<SocialUser> {
    state.social.clear_avatar().await
}

#[tauri::command]
#[specta::specta]
pub async fn social_presence(state: State<'_, AppState>) -> AppResult<PresenceSummary> {
    state.social.presence().await
}

#[tauri::command]
#[specta::specta]
pub async fn social_admin_presence(state: State<'_, AppState>) -> AppResult<AdminPresence> {
    state.social.admin_presence().await
}

#[tauri::command]
#[specta::specta]
pub async fn social_admin_audit(state: State<'_, AppState>) -> AppResult<Vec<AuditEntry>> {
    state.social.audit().await
}
