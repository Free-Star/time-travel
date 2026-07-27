mod database;
mod journal;
mod library;
mod metadata;
mod safety;
mod scanner;
mod thumbnails;
#[cfg(target_os = "windows")]
mod windows_thumbnail;

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
fn library_roots(state: State<'_, AppState>) -> Result<Vec<LibrarySummary>, String> {
    library::all(&state)
}

#[tauri::command]
fn activate_library(
    root: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<LibrarySummary, String> {
    library::activate(&app, &state, root)
}

#[tauri::command]
fn remove_library(
    root: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<LibrarySummary>, String> {
    library::remove(&app, &state, root)
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
fn configure_journal(
    root: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<journal::JournalSummary, String> {
    let media_root = state
        .root()?
        .ok_or_else(|| "请先选择相册目录".to_string())?;
    journal::configure(&app, &media_root, &root)
}

#[tauri::command]
fn current_journal(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<journal::JournalSummary>, String> {
    let Some(media_root) = state.root()? else {
        return Ok(None);
    };
    journal::current(&app, &media_root)
}

#[tauri::command]
async fn scan_journal(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<journal::JournalScanReport, String> {
    let media_root = state
        .root()?
        .ok_or_else(|| "请先选择相册目录".to_string())?;
    tauri::async_runtime::spawn_blocking(move || journal::scan(&app, &media_root))
        .await
        .map_err(|error| format!("日记扫描任务异常退出：{error}"))?
}

#[tauri::command]
fn journal_months(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<journal::JournalMonth>, String> {
    let root = state
        .root()?
        .ok_or_else(|| "请先选择相册目录".to_string())?;
    journal::months(&app, &root)
}

#[tauri::command]
fn journal_entries(
    app: AppHandle,
    state: State<'_, AppState>,
    month: String,
) -> Result<Vec<journal::JournalEntry>, String> {
    let root = state
        .root()?
        .ok_or_else(|| "请先选择相册目录".to_string())?;
    journal::entries(&app, &root, &month)
}

#[tauri::command]
fn journal_entries_for_date(
    app: AppHandle,
    state: State<'_, AppState>,
    date: String,
) -> Result<Vec<journal::JournalEntry>, String> {
    let root = state
        .root()?
        .ok_or_else(|| "请先选择相册目录".to_string())?;
    journal::entries_for_date(&app, &root, &date)
}

#[tauri::command]
fn journal_media_for_date(
    app: AppHandle,
    state: State<'_, AppState>,
    date: String,
) -> Result<Vec<database::TimelineItem>, String> {
    let root = state
        .root()?
        .ok_or_else(|| "请先选择相册目录".to_string())?;
    let connection = database::open(&app, &root)?;
    database::media_for_date(&connection, &root, &date, 100)
}

#[tauri::command]
fn open_journal_in_obsidian(app: AppHandle, path: String) -> Result<(), String> {
    journal::open_in_obsidian(&app, &path)
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
async fn generate_timeline_thumbnails(
    app: AppHandle,
    library_state: State<'_, AppState>,
    thumbnail_control: State<'_, ThumbnailControl>,
    media_ids: Vec<i64>,
) -> Result<ThumbnailReport, String> {
    let root = library_state
        .root()?
        .ok_or_else(|| "请先选择相册目录".to_string())?;
    let cancel = thumbnail_control.begin()?;
    let task_result = tauri::async_runtime::spawn_blocking(move || {
        thumbnails::generate_selected(&app, &root, &cancel, &media_ids, true)
    })
    .await;
    thumbnail_control.finish();
    task_result.map_err(|error| format!("可见预览任务异常退出：{error}"))?
}

#[tauri::command]
async fn ensure_timeline_thumbnails(
    app: AppHandle,
    library_state: State<'_, AppState>,
    thumbnail_control: State<'_, ThumbnailControl>,
    media_ids: Vec<i64>,
) -> Result<Option<ThumbnailReport>, String> {
    let root = library_state
        .root()?
        .ok_or_else(|| "请先选择相册目录".to_string())?;
    let Some(cancel) = thumbnail_control.try_begin()? else {
        return Ok(None);
    };
    let task_result = tauri::async_runtime::spawn_blocking(move || {
        thumbnails::generate_selected(&app, &root, &cancel, &media_ids, false)
    })
    .await;
    thumbnail_control.finish();
    task_result
        .map_err(|error| format!("实时预览任务异常退出：{error}"))?
        .map(Some)
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

#[tauri::command]
fn timeline_months(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<database::TimelineMonth>, String> {
    let root = state
        .root()?
        .ok_or_else(|| "请先选择相册目录".to_string())?;
    let connection = database::open(&app, &root)?;
    database::timeline_months(&connection, &root)
}

#[tauri::command]
fn timeline_window(
    app: AppHandle,
    state: State<'_, AppState>,
    month: String,
    offset: usize,
    limit: usize,
) -> Result<database::TimelineWindow, String> {
    let root = state
        .root()?
        .ok_or_else(|| "请先选择相册目录".to_string())?;
    let connection = database::open(&app, &root)?;
    database::timeline_window(&connection, &root, &month, offset, limit)
}

#[tauri::command]
fn map_overview(
    app: AppHandle,
    state: State<'_, AppState>,
    month: Option<String>,
) -> Result<database::MapOverview, String> {
    let root = state
        .root()?
        .ok_or_else(|| "请先选择相册目录".to_string())?;
    let connection = database::open(&app, &root)?;
    database::map_overview(&connection, &root, month.as_deref())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn map_clusters(
    app: AppHandle,
    state: State<'_, AppState>,
    west: f64,
    east: f64,
    south: f64,
    north: f64,
    zoom: u8,
    month: Option<String>,
) -> Result<Vec<database::MapCluster>, String> {
    let root = state
        .root()?
        .ok_or_else(|| "请先选择相册目录".to_string())?;
    let connection = database::open(&app, &root)?;
    database::map_clusters(
        &connection,
        &root,
        west,
        east,
        south,
        north,
        zoom,
        month.as_deref(),
    )
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn map_cluster_items(
    app: AppHandle,
    state: State<'_, AppState>,
    west: f64,
    east: f64,
    south: f64,
    north: f64,
    month: Option<String>,
    limit: usize,
) -> Result<database::MapClusterWindow, String> {
    let root = state
        .root()?
        .ok_or_else(|| "请先选择相册目录".to_string())?;
    let connection = database::open(&app, &root)?;
    database::map_cluster_items(
        &connection,
        &root,
        west,
        east,
        south,
        north,
        month.as_deref(),
        limit,
    )
}

fn prepare_media_for_view(
    app: &AppHandle,
    root: &std::path::Path,
    item: database::TimelineItem,
) -> Result<database::TimelineItem, String> {
    drop(safety::open_media_readonly(
        std::path::Path::new(&item.path),
        root,
    )?);
    app.asset_protocol_scope()
        .allow_file(&item.path)
        .map_err(|error| format!("无法授权媒体只读展示：{error}"))?;
    Ok(item)
}

#[tauri::command]
fn open_timeline_media(
    app: AppHandle,
    state: State<'_, AppState>,
    media_id: i64,
) -> Result<database::TimelineItem, String> {
    let root = state
        .root()?
        .ok_or_else(|| "请先选择相册目录".to_string())?;
    let connection = database::open(&app, &root)?;
    let item = database::timeline_item(&connection, &root, media_id)?
        .ok_or_else(|| "媒体已不在当前索引中".to_string())?;
    prepare_media_for_view(&app, &root, item)
}

#[tauri::command]
fn timeline_neighbor(
    app: AppHandle,
    state: State<'_, AppState>,
    media_id: i64,
    direction: String,
) -> Result<Option<database::TimelineItem>, String> {
    let root = state
        .root()?
        .ok_or_else(|| "请先选择相册目录".to_string())?;
    let connection = database::open(&app, &root)?;
    let current = database::timeline_item(&connection, &root, media_id)?
        .ok_or_else(|| "媒体已不在当前索引中".to_string())?;
    database::timeline_neighbor(
        &connection,
        &root,
        &current.captured_at,
        current.id,
        &direction,
    )?
    .map(|item| prepare_media_for_view(&app, &root, item))
    .transpose()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .manage(ScanControl::default())
        .manage(ThumbnailControl::default())
        .setup(|app| {
            let settings = library::load_saved_settings(app.handle())?;
            app.state::<AppState>().replace_settings(settings)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            configure_library,
            current_library,
            library_roots,
            activate_library,
            remove_library,
            current_index,
            scan_library,
            cancel_scan,
            configure_journal,
            current_journal,
            scan_journal,
            journal_months,
            journal_entries,
            journal_entries_for_date,
            journal_media_for_date,
            open_journal_in_obsidian,
            thumbnail_status,
            thumbnail_previews,
            generate_thumbnails,
            generate_timeline_thumbnails,
            ensure_timeline_thumbnails,
            cancel_thumbnails,
            clear_thumbnail_cache,
            timeline_months,
            timeline_window,
            map_overview,
            map_clusters,
            map_cluster_items,
            open_timeline_media,
            timeline_neighbor
        ])
        .run(tauri::generate_context!())
        .expect("error while running time album");
}
