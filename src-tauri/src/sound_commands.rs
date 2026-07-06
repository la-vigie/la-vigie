//! Tauri command glue for custom notification sounds. Thin wrappers over
//! `crate::sound` (which holds the testable logic) — not unit-tested per the
//! project's testing conventions.

use std::path::PathBuf;

use tauri::State;

use crate::sound::{self, CustomSound};
use crate::state::AppState;

#[tauri::command]
pub fn import_custom_sound(
    state: State<'_, AppState>,
    src_path: String,
    label: String,
) -> Result<CustomSound, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    sound::import_sound(&store, &state.sounds_root, &PathBuf::from(src_path), &label)
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub fn list_custom_sounds(state: State<'_, AppState>) -> Result<Vec<CustomSound>, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    sound::list_sounds(&store).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub fn read_sound_bytes(state: State<'_, AppState>, id: String) -> Result<Vec<u8>, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    sound::read_bytes(&store, &state.sounds_root, &id).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub fn delete_custom_sound(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    sound::delete_sound(&store, &state.sounds_root, &id).map_err(|e| format!("{e:#}"))
}
