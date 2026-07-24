mod database;
mod library;
mod metadata;
mod safety;
mod scanner;

use database::IndexSummary;
use library::{AppState, LibrarySummary};
use scanner::{ScanControl, ScanReport};
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

#[tauri::command]
fn current_index(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<IndexSummary>, String> {
    state
        .root()?
        .map(|root| scanner::index_summary(&app, &root))
        .transpose()
}

#[tauri::command]
async fn scan_library(
    app: AppHandle,
    library_state: State<'_, AppState>,
    scan_control: State<'_, ScanControl>,
) -> Result<ScanReport, String> {
    let root = library_state
        .root()?
        .ok_or_else(|| "请先选择相册目录".to_string())?;
    let cancel = scan_control.begin()?;
    let task_result =
        tauri::async_runtime::spawn_blocking(move || scanner::run(&app, &root, &cancel)).await;
    scan_control.finish();
    task_result.map_err(|error| format!("扫描任务异常退出：{error}"))?
}

#[tauri::command]
fn cancel_scan(scan_control: State<'_, ScanControl>) -> bool {
    scan_control.cancel()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .manage(ScanControl::default())
        .setup(|app| {
            if let Ok(Some(root)) = library::load_saved_root(app.handle()) {
                let state = app.state::<AppState>();
                state.set_root(root)?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            configure_library,
            current_library,
            current_index,
            scan_library,
            cancel_scan
        ])
        .run(tauri::generate_context!())
        .expect("error while running time album");
}
