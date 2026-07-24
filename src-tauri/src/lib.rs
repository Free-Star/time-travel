mod library;
mod safety;

use library::{AppState, LibrarySummary};
use tauri::{AppHandle, Manager, State};

#[tauri::command]
fn configure_library(
    root: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<LibrarySummary, String> {
    library::configure(&app, &state, root)
}

#[tauri::command]
fn current_library(state: State<'_, AppState>) -> Result<Option<LibrarySummary>, String> {
    library::current(&state)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .setup(|app| {
            if let Ok(Some(root)) = library::load_saved_root(app.handle()) {
                let state = app.state::<AppState>();
                state.set_root(root)?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![configure_library, current_library])
        .run(tauri::generate_context!())
        .expect("error while running time album");
}
