use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use chrono::Local;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use walkdir::{DirEntry, WalkDir};

use crate::{
    database::{self, IndexSummary, MediaRecord},
    metadata, safety,
};

const PHOTO_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "heif", "heic", "dng"];
const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mov", "3gp", "avi", "mkv", "vob"];

#[derive(Default)]
pub struct ScanControl {
    active_cancel: Mutex<Option<Arc<AtomicBool>>>,
}

impl ScanControl {
    pub fn begin(&self) -> Result<Arc<AtomicBool>, String> {
        let mut guard = self
            .active_cancel
            .lock()
            .map_err(|_| "扫描状态锁已损坏".to_string())?;
        if guard.is_some() {
            return Err("已有扫描正在运行".to_string());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *guard = Some(cancel.clone());
        Ok(cancel)
    }

    pub fn finish(&self) {
        if let Ok(mut guard) = self.active_cancel.lock() {
            *guard = None;
        }
    }

    pub fn cancel(&self) -> bool {
        self.active_cancel
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
            .map(|cancel| {
                cancel.store(true, Ordering::Relaxed);
                true
            })
            .unwrap_or(false)
    }
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    pub status: String,
    pub discovered: usize,
    pub inserted: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub errors: usize,
    pub current_path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanReport {
    pub status: String,
    pub discovered: usize,
    pub inserted: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub errors: usize,
    pub removed: usize,
    pub summary: IndexSummary,
}

pub fn run(
    app: &AppHandle,
    library_root: &Path,
    cancel: &AtomicBool,
) -> Result<ScanReport, String> {
    let database_path = database::database_path(app)?;
    run_core(Some(app), library_root, &database_path, cancel)
}

fn run_core(
    app: Option<&AppHandle>,
    library_root: &Path,
    database_path: &Path,
    cancel: &AtomicBool,
) -> Result<ScanReport, String> {
    let root = safety::canonical_existing(library_root)?;
    let workspace = safety::workspace_dir()?;
    let mut connection = database::open_at(database_path, &root)?;
    let started_at = Local::now().to_rfc3339();
    let scan_id = database::begin_scan(&connection, &root, &started_at)?;
    let mut progress = ScanProgress {
        status: "scanning".to_string(),
        ..ScanProgress::default()
    };
    let removed = {
        let transaction = connection
            .transaction()
            .map_err(|error| format!("无法开始索引事务：{error}"))?;

        for entry in WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| include_entry(entry, &workspace))
        {
            if cancel.load(Ordering::Relaxed) {
                drop(transaction);
                progress.status = "cancelled".to_string();
                emit_progress(app, &progress);
                database::finish_scan(
                    &connection,
                    scan_id,
                    "cancelled",
                    &Local::now().to_rfc3339(),
                    progress.discovered,
                    progress.inserted,
                    progress.updated,
                    progress.unchanged,
                    progress.errors,
                )?;
                return Ok(report(&connection, &root, progress, 0)?);
            }

            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    progress.errors += 1;
                    continue;
                }
            };
            if !entry.file_type().is_file() {
                continue;
            }

            let Some(media_kind) = media_kind(entry.path()) else {
                continue;
            };
            let canonical_path = match safety::canonical_existing(entry.path()) {
                Ok(path)
                    if safety::is_same_or_descendant(&path, &root)
                        && !safety::is_same_or_descendant(&path, &workspace) =>
                {
                    path
                }
                _ => {
                    progress.errors += 1;
                    continue;
                }
            };

            progress.discovered += 1;
            progress.current_path = canonical_path.to_string_lossy().to_string();
            let file_metadata = match canonical_path.metadata() {
                Ok(metadata) => metadata,
                Err(_) => {
                    progress.errors += 1;
                    continue;
                }
            };
            let modified = file_metadata.modified().unwrap_or(UNIX_EPOCH);
            let modified_ns = system_time_ns(modified);
            let size_bytes = i64::try_from(file_metadata.len()).unwrap_or(i64::MAX);
            let path_string = canonical_path.to_string_lossy().to_string();
            let fingerprint = database::existing_fingerprint(&transaction, &path_string)?;

            if fingerprint == Some((size_bytes, modified_ns, database::METADATA_VERSION)) {
                database::mark_seen(&transaction, &path_string, scan_id)?;
                progress.unchanged += 1;
            } else {
                let relative_path = canonical_path
                    .strip_prefix(&root)
                    .map_err(|_| "媒体路径无法转换为相对路径".to_string())?;
                let extracted = metadata::extract(
                    &canonical_path,
                    relative_path,
                    &root,
                    media_kind == "photo",
                    modified,
                );
                let record = MediaRecord {
                    path: path_string,
                    relative_path: relative_path.to_string_lossy().to_string(),
                    media_kind: media_kind.to_string(),
                    extension: canonical_path
                        .extension()
                        .and_then(|value| value.to_str())
                        .unwrap_or_default()
                        .to_ascii_lowercase(),
                    size_bytes,
                    modified_ns,
                    captured_at: extracted.captured_at,
                    captured_source: extracted.captured_source,
                    captured_precision: extracted.captured_precision,
                    latitude: extracted.latitude,
                    longitude: extracted.longitude,
                    width: extracted.width,
                    height: extracted.height,
                    metadata_error: extracted.error,
                };
                database::upsert_media(
                    &transaction,
                    &root,
                    &record,
                    scan_id,
                    &Local::now().to_rfc3339(),
                )?;
                if fingerprint.is_some() {
                    progress.updated += 1;
                } else {
                    progress.inserted += 1;
                }
            }

            if progress.discovered.is_multiple_of(100) {
                emit_progress(app, &progress);
            }
        }

        let removed = database::remove_stale(&transaction, &root, scan_id)?;
        transaction
            .commit()
            .map_err(|error| format!("无法提交媒体索引：{error}"))?;
        removed
    };

    progress.status = "completed".to_string();
    progress.current_path.clear();
    database::finish_scan(
        &connection,
        scan_id,
        "completed",
        &Local::now().to_rfc3339(),
        progress.discovered,
        progress.inserted,
        progress.updated,
        progress.unchanged,
        progress.errors,
    )?;
    emit_progress(app, &progress);
    report(&connection, &root, progress, removed)
}

pub fn index_summary(app: &AppHandle, root: &Path) -> Result<IndexSummary, String> {
    let connection = database::open(app, root)?;
    database::summary(&connection, root)
}

fn report(
    connection: &rusqlite::Connection,
    root: &Path,
    progress: ScanProgress,
    removed: usize,
) -> Result<ScanReport, String> {
    Ok(ScanReport {
        status: progress.status,
        discovered: progress.discovered,
        inserted: progress.inserted,
        updated: progress.updated,
        unchanged: progress.unchanged,
        errors: progress.errors,
        removed,
        summary: database::summary(connection, root)?,
    })
}

fn include_entry(entry: &DirEntry, workspace: &Path) -> bool {
    safety::canonical_existing(entry.path())
        .map(|path| !safety::is_same_or_descendant(&path, workspace))
        .unwrap_or(false)
}

fn media_kind(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?;
    if PHOTO_EXTENSIONS
        .iter()
        .any(|value| extension.eq_ignore_ascii_case(value))
    {
        Some("photo")
    } else if VIDEO_EXTENSIONS
        .iter()
        .any(|value| extension.eq_ignore_ascii_case(value))
    {
        Some("video")
    } else {
        None
    }
}

fn system_time_ns(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_nanos()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn emit_progress(app: Option<&AppHandle>, progress: &ScanProgress) {
    if let Some(app) = app {
        let _ = app.emit("scan-progress", progress);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, time::Duration};

    #[test]
    fn recognizes_supported_media_case_insensitively() {
        assert_eq!(media_kind(Path::new("photo.JPG")), Some("photo"));
        assert_eq!(media_kind(Path::new("clip.MP4")), Some("video"));
        assert_eq!(media_kind(Path::new("notes.txt")), None);
    }

    #[test]
    fn cancellation_is_idempotent() {
        let control = ScanControl::default();
        assert!(!control.cancel());
        let token = control.begin().expect("scan begins");
        assert!(control.cancel());
        assert!(token.load(Ordering::Relaxed));
        control.finish();
        assert!(!control.cancel());
    }

    #[test]
    fn scan_is_incremental_and_does_not_modify_media() {
        let unique = format!(
            "time-album-scan-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        );
        let test_root = std::env::temp_dir().join(unique);
        let library = test_root.join("library").join("2024年").join("7月");
        let database_path = test_root.join("data").join("index.sqlite3");
        fs::create_dir_all(&library).expect("create test library");
        let media_path = library.join("IMG_20240731_121955.jpg");
        let media_bytes = b"not-a-real-jpeg-but-safe-for-scanner";
        fs::write(&media_path, media_bytes).expect("write test media");
        let before = fs::metadata(&media_path).expect("read media metadata");
        let before_modified = before.modified().expect("media has modified time");

        let first = run_core(
            None,
            &test_root.join("library"),
            &database_path,
            &AtomicBool::new(false),
        )
        .expect("first scan succeeds");
        assert_eq!(first.inserted, 1);
        assert_eq!(first.summary.total, 1);

        std::thread::sleep(Duration::from_millis(20));
        let second = run_core(
            None,
            &test_root.join("library"),
            &database_path,
            &AtomicBool::new(false),
        )
        .expect("second scan succeeds");
        assert_eq!(second.inserted, 0);
        assert_eq!(second.unchanged, 1);

        assert_eq!(fs::read(&media_path).expect("read test media"), media_bytes);
        let after = fs::metadata(&media_path).expect("read media metadata after scan");
        assert_eq!(after.len(), before.len());
        assert_eq!(
            after.modified().expect("media has modified time"),
            before_modified
        );

        fs::remove_dir_all(test_root).expect("remove test directories");
    }

    #[test]
    #[ignore = "requires TIME_ALBUM_REAL_DB"]
    fn scans_a_real_library_when_explicitly_requested() {
        let library = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("project lives directly inside the media root");
        let database = std::env::var("TIME_ALBUM_REAL_DB").expect("TIME_ALBUM_REAL_DB must be set");
        let report = run_core(None, library, Path::new(&database), &AtomicBool::new(false))
            .expect("real library scan succeeds");
        assert_eq!(report.status, "completed");
        assert!(report.summary.total > 0);
        println!("{report:#?}");
    }
}
