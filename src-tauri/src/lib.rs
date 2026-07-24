mod database;
mod library;
mod metadata;
mod safety;
mod scanner;
mod thumbnails;

use database::IndexSummary;
use library::{AppState, LibrarySummary};
use scanner::{ScanControl, ScanReport};
use tauri::{AppHandle, Manager, State};
use thumbnails::{ThumbnailControl, ThumbnailReport};

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

#[tauri::command]
fn thumbnail_status(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<database::ThumbnailStatus>, String> {
    state
        .root()?
        .map(|root| thumbnails::status(&app, &root))
        .transpose()
}

#[tauri::command]
fn thumbnail_previews(
    app: AppHandle,
    state: State<'_, AppState>,
    limit: usize,
) -> Result<Vec<database::ThumbnailPreview>, String> {
    let root = state
        .root()?
        .ok_or_else(|| "请先选择相册目录".to_string())?;
    thumbnails::previews(&app, &root, limit)
}

#[tauri::command]
async fn generate_thumbnails(
    app: AppHandle,
    library_state: State<'_, AppState>,
    thumbnail_control: State<'_, ThumbnailControl>,
    limit: usize,
) -> Result<ThumbnailReport, String> {
    let root = library_state
        .root()?
        .ok_or_else(|| "请先选择相册目录".to_string())?;
    let cancel = thumbnail_control.begin()?;
    let task_result = tauri::async_runtime::spawn_blocking(move || {
        thumbnails::generate(&app, &root, &cancel, limit)
    })
    .await;
    thumbnail_control.finish();
    task_result.map_err(|error| format!("缩略图任务异常退出：{error}"))?
}

#[tauri::command]
fn cancel_thumbnails(thumbnail_control: State<'_, ThumbnailControl>) -> bool {
    thumbnail_control.cancel()
}

#[tauri::command]
fn clear_thumbnail_cache(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<database::ThumbnailStatus, String> {
    let root = state
        .root()?
        .ok_or_else(|| "请先选择相册目录".to_string())?;
    thumbnails::clear(&app, &root)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .manage(ScanControl::default())
        .manage(ThumbnailControl::default())
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
            cancel_scan,
            thumbnail_status,
            thumbnail_previews,
            generate_thumbnails,
            cancel_thumbnails,
            clear_thumbnail_cache
        ])
        .run(tauri::generate_context!())
        .expect("error while running time album");
}
