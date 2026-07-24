use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::safety;

const DATABASE_FILE: &str = "time-album.sqlite3";

#[derive(Debug)]
pub struct MediaRecord {
    pub path: String,
    pub relative_path: String,
    pub media_kind: String,
    pub extension: String,
    pub size_bytes: i64,
    pub modified_ns: i64,
    pub captured_at: String,
    pub captured_source: String,
    pub captured_precision: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub metadata_error: Option<String>,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexSummary {
    pub total: i64,
    pub photos: i64,
    pub videos: i64,
    pub with_location: i64,
    pub last_scan_at: Option<String>,
}

pub fn open(app: &AppHandle, library_root: &Path) -> Result<Connection, String> {
    let database_path = database_path(app)?;
    open_at(&database_path, library_root)
}

pub fn open_at(database_path: &Path, library_root: &Path) -> Result<Connection, String> {
    let data_directory = database_path
        .parent()
        .ok_or_else(|| "数据库路径没有父目录".to_string())?;
    safety::create_directory_outside_library(data_directory, library_root)?;
    safety::ensure_write_outside_library(&database_path, library_root)?;

    let connection =
        Connection::open(database_path).map_err(|error| format!("无法打开索引数据库：{error}"))?;
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(|error| format!("无法启用数据库 WAL：{error}"))?;
    connection
        .pragma_update(None, "foreign_keys", true)
        .map_err(|error| format!("无法启用数据库外键：{error}"))?;
    connection
        .execute_batch(
            "
            CREATE TABLE IF NOT EXISTS scans (
                id              INTEGER PRIMARY KEY,
                library_root    TEXT NOT NULL,
                started_at      TEXT NOT NULL,
                finished_at     TEXT,
                status          TEXT NOT NULL,
                discovered      INTEGER NOT NULL DEFAULT 0,
                inserted        INTEGER NOT NULL DEFAULT 0,
                updated         INTEGER NOT NULL DEFAULT 0,
                unchanged       INTEGER NOT NULL DEFAULT 0,
                errors          INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS media (
                id                  INTEGER PRIMARY KEY,
                library_root        TEXT NOT NULL,
                path                TEXT NOT NULL UNIQUE,
                relative_path       TEXT NOT NULL,
                media_kind          TEXT NOT NULL CHECK (media_kind IN ('photo', 'video')),
                extension           TEXT NOT NULL,
                size_bytes          INTEGER NOT NULL,
                modified_ns         INTEGER NOT NULL,
                captured_at         TEXT NOT NULL,
                captured_source     TEXT NOT NULL,
                captured_precision  TEXT NOT NULL,
                latitude            REAL,
                longitude           REAL,
                width               INTEGER,
                height              INTEGER,
                metadata_error      TEXT,
                last_seen_scan      INTEGER NOT NULL REFERENCES scans(id),
                indexed_at          TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_media_captured_at ON media(captured_at);
            CREATE INDEX IF NOT EXISTS idx_media_location ON media(latitude, longitude);
            CREATE INDEX IF NOT EXISTS idx_media_library ON media(library_root);
            ",
        )
        .map_err(|error| format!("无法初始化索引数据库：{error}"))?;

    Ok(connection)
}

pub fn database_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|directory| directory.join(DATABASE_FILE))
        .map_err(|error| format!("无法确定应用数据目录：{error}"))
}

pub fn begin_scan(connection: &Connection, root: &Path, started_at: &str) -> Result<i64, String> {
    connection
        .execute(
            "INSERT INTO scans (library_root, started_at, status) VALUES (?1, ?2, 'running')",
            params![root.to_string_lossy(), started_at],
        )
        .map_err(|error| format!("无法创建扫描记录：{error}"))?;
    Ok(connection.last_insert_rowid())
}

pub fn existing_fingerprint(
    transaction: &Transaction<'_>,
    path: &str,
) -> Result<Option<(i64, i64)>, String> {
    transaction
        .query_row(
            "SELECT size_bytes, modified_ns FROM media WHERE path = ?1",
            [path],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| format!("无法读取已有索引：{error}"))
}

pub fn mark_seen(transaction: &Transaction<'_>, path: &str, scan_id: i64) -> Result<(), String> {
    transaction
        .execute(
            "UPDATE media SET last_seen_scan = ?2 WHERE path = ?1",
            params![path, scan_id],
        )
        .map(|_| ())
        .map_err(|error| format!("无法更新增量扫描标记：{error}"))
}

pub fn upsert_media(
    transaction: &Transaction<'_>,
    root: &Path,
    record: &MediaRecord,
    scan_id: i64,
    indexed_at: &str,
) -> Result<(), String> {
    transaction
        .execute(
            "
            INSERT INTO media (
                library_root, path, relative_path, media_kind, extension,
                size_bytes, modified_ns, captured_at, captured_source,
                captured_precision, latitude, longitude, width, height,
                metadata_error, last_seen_scan, indexed_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                ?13, ?14, ?15, ?16, ?17
            )
            ON CONFLICT(path) DO UPDATE SET
                library_root = excluded.library_root,
                relative_path = excluded.relative_path,
                media_kind = excluded.media_kind,
                extension = excluded.extension,
                size_bytes = excluded.size_bytes,
                modified_ns = excluded.modified_ns,
                captured_at = excluded.captured_at,
                captured_source = excluded.captured_source,
                captured_precision = excluded.captured_precision,
                latitude = excluded.latitude,
                longitude = excluded.longitude,
                width = excluded.width,
                height = excluded.height,
                metadata_error = excluded.metadata_error,
                last_seen_scan = excluded.last_seen_scan,
                indexed_at = excluded.indexed_at
            ",
            params![
                root.to_string_lossy(),
                record.path,
                record.relative_path,
                record.media_kind,
                record.extension,
                record.size_bytes,
                record.modified_ns,
                record.captured_at,
                record.captured_source,
                record.captured_precision,
                record.latitude,
                record.longitude,
                record.width,
                record.height,
                record.metadata_error,
                scan_id,
                indexed_at,
            ],
        )
        .map(|_| ())
        .map_err(|error| format!("无法写入媒体索引：{error}"))
}

pub fn remove_stale(
    transaction: &Transaction<'_>,
    root: &Path,
    scan_id: i64,
) -> Result<usize, String> {
    transaction
        .execute(
            "DELETE FROM media WHERE library_root = ?1 AND last_seen_scan <> ?2",
            params![root.to_string_lossy(), scan_id],
        )
        .map_err(|error| format!("无法移除失效索引：{error}"))
}

#[allow(clippy::too_many_arguments)]
pub fn finish_scan(
    connection: &Connection,
    scan_id: i64,
    status: &str,
    finished_at: &str,
    discovered: usize,
    inserted: usize,
    updated: usize,
    unchanged: usize,
    errors: usize,
) -> Result<(), String> {
    connection
        .execute(
            "
            UPDATE scans SET
                finished_at = ?2,
                status = ?3,
                discovered = ?4,
                inserted = ?5,
                updated = ?6,
                unchanged = ?7,
                errors = ?8
            WHERE id = ?1
            ",
            params![
                scan_id,
                finished_at,
                status,
                discovered as i64,
                inserted as i64,
                updated as i64,
                unchanged as i64,
                errors as i64,
            ],
        )
        .map(|_| ())
        .map_err(|error| format!("无法完成扫描记录：{error}"))
}

pub fn summary(connection: &Connection, root: &Path) -> Result<IndexSummary, String> {
    let root = root.to_string_lossy();
    let mut summary = connection
        .query_row(
            "
            SELECT
                COUNT(*),
                SUM(CASE WHEN media_kind = 'photo' THEN 1 ELSE 0 END),
                SUM(CASE WHEN media_kind = 'video' THEN 1 ELSE 0 END),
                SUM(CASE WHEN latitude IS NOT NULL AND longitude IS NOT NULL THEN 1 ELSE 0 END)
            FROM media WHERE library_root = ?1
            ",
            [root.as_ref()],
            |row| {
                Ok(IndexSummary {
                    total: row.get(0)?,
                    photos: row.get::<_, Option<i64>>(1)?.unwrap_or(0),
                    videos: row.get::<_, Option<i64>>(2)?.unwrap_or(0),
                    with_location: row.get::<_, Option<i64>>(3)?.unwrap_or(0),
                    last_scan_at: None,
                })
            },
        )
        .map_err(|error| format!("无法汇总媒体索引：{error}"))?;

    summary.last_scan_at = connection
        .query_row(
            "
            SELECT finished_at FROM scans
            WHERE library_root = ?1 AND status = 'completed'
            ORDER BY id DESC LIMIT 1
            ",
            [root.as_ref()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("无法读取最近扫描时间：{error}"))?;

    Ok(summary)
}
