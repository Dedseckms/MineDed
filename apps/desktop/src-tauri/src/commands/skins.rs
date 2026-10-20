//! Owner: accounts.

use lc_types::skin::{Cape, Skin, SkinModel};
use lc_types::{AccountId, AppResult, SkinId};
use tauri::State;

use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn skin_library(state: State<'_, AppState>) -> AppResult<Vec<Skin>> {
    state.skins.library().await
}

#[tauri::command]
#[specta::specta]
pub async fn skin_add(
    state: State<'_, AppState>,
    png_path: String,
    name: String,
    model: SkinModel,
) -> AppResult<Skin> {
    state.skins.add(&png_path, &name, model).await
}

#[tauri::command]
#[specta::specta]
pub async fn skin_remove(state: State<'_, AppState>, id: SkinId) -> AppResult<()> {
    state.skins.remove(id).await
}

#[tauri::command]
#[specta::specta]
pub async fn skin_apply(
    state: State<'_, AppState>,
    account: AccountId,
    skin: SkinId,
) -> AppResult<()> {
    state.skins.apply(account, skin).await
}

#[tauri::command]
#[specta::specta]
pub async fn cape_list(state: State<'_, AppState>, account: AccountId) -> AppResult<Vec<Cape>> {
    state.skins.capes(account).await
}

#[tauri::command]
#[specta::specta]
pub async fn cape_set(
    state: State<'_, AppState>,
    account: AccountId,
    cape_id: Option<String>,
) -> AppResult<()> {
    state.skins.set_cape(account, cape_id).await
}
