use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::safety;

const DATABASE_FILE: &str = "time-album.sqlite3";
pub const METADATA_VERSION: i64 = 2;

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
    pub needs_metadata_refresh: bool,
}

#[derive(Debug)]
pub struct ThumbnailCandidate {
    pub media_id: i64,
    pub path: String,
    pub media_kind: String,
    pub modified_ns: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailPreview {
    pub media_id: i64,
    pub media_kind: String,
    pub captured_at: String,
    pub cache_path: String,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailStatus {
    pub total_media: i64,
    pub ready: i64,
    pub failed: i64,
    pub cache_bytes: i64,
    pub ffmpeg_available: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineMonth {
    pub key: String,
    pub total: i64,
    pub photos: i64,
    pub videos: i64,
    pub with_location: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineItem {
    pub id: i64,
    pub path: String,
    pub relative_path: String,
    pub media_kind: String,
    pub extension: String,
    pub size_bytes: i64,
    pub captured_at: String,
    pub captured_source: String,
    pub captured_precision: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub thumbnail_path: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineWindow {
    pub month: String,
    pub total: i64,
    pub offset: usize,
    pub items: Vec<TimelineItem>,
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
                metadata_version    INTEGER NOT NULL DEFAULT 2,
                last_seen_scan      INTEGER NOT NULL REFERENCES scans(id),
                indexed_at          TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_media_captured_at ON media(captured_at);
            CREATE INDEX IF NOT EXISTS idx_media_location ON media(latitude, longitude);
            CREATE INDEX IF NOT EXISTS idx_media_library ON media(library_root);
            CREATE INDEX IF NOT EXISTS idx_media_library_captured
                ON media(library_root, captured_at DESC, id DESC);

            CREATE TABLE IF NOT EXISTS thumbnails (
                media_id            INTEGER PRIMARY KEY REFERENCES media(id) ON DELETE CASCADE,
                cache_path          TEXT,
                source_modified_ns  INTEGER NOT NULL,
                width               INTEGER,
                height              INTEGER,
                bytes               INTEGER NOT NULL DEFAULT 0,
                status              TEXT NOT NULL CHECK (status IN ('ready', 'failed')),
                error               TEXT,
                generated_at        TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_thumbnails_status ON thumbnails(status);
            ",
        )
        .map_err(|error| format!("无法初始化索引数据库：{error}"))?;

    let has_metadata_version = connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('media') WHERE name = 'metadata_version'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("无法检查索引版本：{error}"))?
        > 0;
    if !has_metadata_version {
        connection
            .execute(
                "ALTER TABLE media ADD COLUMN metadata_version INTEGER NOT NULL DEFAULT 1",
                [],
            )
            .map_err(|error| format!("无法升级媒体索引版本：{error}"))?;
    }

    Ok(connection)
}

pub fn thumbnail_candidates(
    connection: &Connection,
    root: &Path,
    limit: usize,
) -> Result<Vec<ThumbnailCandidate>, String> {
    let mut statement = connection
        .prepare(
            "
            SELECT m.id, m.path, m.media_kind, m.modified_ns
            FROM media m
            LEFT JOIN thumbnails t ON t.media_id = m.id
            WHERE m.library_root = ?1
              AND (
                t.media_id IS NULL
                OR t.source_modified_ns <> m.modified_ns
              )
            ORDER BY m.captured_at DESC, m.id DESC
            LIMIT ?2
            ",
        )
        .map_err(|error| format!("无法准备缩略图队列：{error}"))?;
    let rows = statement
        .query_map(params![root.to_string_lossy(), limit as i64], |row| {
            Ok(ThumbnailCandidate {
                media_id: row.get(0)?,
                path: row.get(1)?,
                media_kind: row.get(2)?,
                modified_ns: row.get(3)?,
            })
        })
        .map_err(|error| format!("无法读取缩略图队列：{error}"))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("无法解析缩略图队列：{error}"))
}

pub fn thumbnail_candidates_by_ids(
    connection: &Connection,
    root: &Path,
    media_ids: &[i64],
) -> Result<Vec<ThumbnailCandidate>, String> {
    let mut statement = connection
        .prepare(
            "
            SELECT m.id, m.path, m.media_kind, m.modified_ns
            FROM media m
            LEFT JOIN thumbnails t ON t.media_id = m.id
            WHERE m.library_root = ?1
              AND m.id = ?2
              AND (
                t.media_id IS NULL
                OR t.source_modified_ns <> m.modified_ns
                OR t.status = 'failed'
              )
            ",
        )
        .map_err(|error| format!("无法准备可见预览队列：{error}"))?;
    let mut candidates = Vec::with_capacity(media_ids.len());
    for media_id in media_ids {
        let candidate = statement
            .query_row(params![root.to_string_lossy(), media_id], |row| {
                Ok(ThumbnailCandidate {
                    media_id: row.get(0)?,
                    path: row.get(1)?,
                    media_kind: row.get(2)?,
                    modified_ns: row.get(3)?,
                })
            })
            .optional()
            .map_err(|error| format!("无法读取可见预览队列：{error}"))?;
        if let Some(candidate) = candidate {
            candidates.push(candidate);
        }
    }
    Ok(candidates)
}

#[allow(clippy::too_many_arguments)]
pub fn record_thumbnail(
    connection: &Connection,
    media_id: i64,
    source_modified_ns: i64,
    cache_path: Option<&Path>,
    width: Option<u32>,
    height: Option<u32>,
    bytes: i64,
    status: &str,
    error: Option<&str>,
    generated_at: &str,
) -> Result<(), String> {
    connection
        .execute(
            "
            INSERT INTO thumbnails (
                media_id, cache_path, source_modified_ns, width, height,
                bytes, status, error, generated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(media_id) DO UPDATE SET
                cache_path = excluded.cache_path,
                source_modified_ns = excluded.source_modified_ns,
                width = excluded.width,
                height = excluded.height,
                bytes = excluded.bytes,
                status = excluded.status,
                error = excluded.error,
                generated_at = excluded.generated_at
            ",
            params![
                media_id,
                cache_path.map(|path| path.to_string_lossy().to_string()),
                source_modified_ns,
                width.map(i64::from),
                height.map(i64::from),
                bytes,
                status,
                error,
                generated_at,
            ],
        )
        .map(|_| ())
        .map_err(|error| format!("无法记录缩略图结果：{error}"))
}

pub fn thumbnail_status(connection: &Connection, root: &Path) -> Result<ThumbnailStatus, String> {
    connection
        .query_row(
            "
            SELECT
                COUNT(m.id),
                SUM(CASE WHEN t.status = 'ready' AND t.source_modified_ns = m.modified_ns THEN 1 ELSE 0 END),
                SUM(CASE WHEN t.status = 'failed' AND t.source_modified_ns = m.modified_ns THEN 1 ELSE 0 END),
                SUM(CASE WHEN t.status = 'ready' AND t.source_modified_ns = m.modified_ns THEN t.bytes ELSE 0 END)
            FROM media m
            LEFT JOIN thumbnails t ON t.media_id = m.id
            WHERE m.library_root = ?1
            ",
            [root.to_string_lossy().as_ref()],
            |row| {
                Ok(ThumbnailStatus {
                    total_media: row.get(0)?,
                    ready: row.get::<_, Option<i64>>(1)?.unwrap_or(0),
                    failed: row.get::<_, Option<i64>>(2)?.unwrap_or(0),
                    cache_bytes: row.get::<_, Option<i64>>(3)?.unwrap_or(0),
                    ffmpeg_available: false,
                })
            },
        )
        .map_err(|error| format!("无法读取缩略图状态：{error}"))
}

pub fn thumbnail_previews(
    connection: &Connection,
    root: &Path,
    limit: usize,
) -> Result<Vec<ThumbnailPreview>, String> {
    let mut statement = connection
        .prepare(
            "
            SELECT m.id, m.media_kind, m.captured_at, t.cache_path
            FROM media m
            JOIN thumbnails t ON t.media_id = m.id
            WHERE m.library_root = ?1
              AND t.status = 'ready'
              AND t.source_modified_ns = m.modified_ns
              AND t.cache_path IS NOT NULL
            ORDER BY m.captured_at DESC, m.id DESC
            LIMIT ?2
            ",
        )
        .map_err(|error| format!("无法准备预览查询：{error}"))?;
    let rows = statement
        .query_map(params![root.to_string_lossy(), limit as i64], |row| {
            Ok(ThumbnailPreview {
                media_id: row.get(0)?,
                media_kind: row.get(1)?,
                captured_at: row.get(2)?,
                cache_path: row.get(3)?,
            })
        })
        .map_err(|error| format!("无法读取预览：{error}"))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("无法解析预览：{error}"))
}

pub fn clear_thumbnails(connection: &Connection, root: &Path) -> Result<usize, String> {
    connection
        .execute(
            "
            DELETE FROM thumbnails
            WHERE media_id IN (
                SELECT id FROM media WHERE library_root = ?1
            )
            ",
            [root.to_string_lossy().as_ref()],
        )
        .map_err(|error| format!("无法清除缩略图记录：{error}"))
}

pub fn timeline_months(connection: &Connection, root: &Path) -> Result<Vec<TimelineMonth>, String> {
    let mut statement = connection
        .prepare(
            "
            SELECT
                substr(captured_at, 1, 7) AS month_key,
                COUNT(*),
                SUM(CASE WHEN media_kind = 'photo' THEN 1 ELSE 0 END),
                SUM(CASE WHEN media_kind = 'video' THEN 1 ELSE 0 END),
                SUM(CASE WHEN latitude IS NOT NULL AND longitude IS NOT NULL THEN 1 ELSE 0 END)
            FROM media
            WHERE library_root = ?1
            GROUP BY month_key
            ORDER BY month_key DESC
            ",
        )
        .map_err(|error| format!("无法准备时间线月份查询：{error}"))?;
    let rows = statement
        .query_map([root.to_string_lossy().as_ref()], |row| {
            Ok(TimelineMonth {
                key: row.get(0)?,
                total: row.get(1)?,
                photos: row.get::<_, Option<i64>>(2)?.unwrap_or(0),
                videos: row.get::<_, Option<i64>>(3)?.unwrap_or(0),
                with_location: row.get::<_, Option<i64>>(4)?.unwrap_or(0),
            })
        })
        .map_err(|error| format!("无法读取时间线月份：{error}"))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("无法解析时间线月份：{error}"))
}

fn map_timeline_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<TimelineItem> {
    Ok(TimelineItem {
        id: row.get(0)?,
        path: row.get(1)?,
        relative_path: row.get(2)?,
        media_kind: row.get(3)?,
        extension: row.get(4)?,
        size_bytes: row.get(5)?,
        captured_at: row.get(6)?,
        captured_source: row.get(7)?,
        captured_precision: row.get(8)?,
        latitude: row.get(9)?,
        longitude: row.get(10)?,
        width: row.get(11)?,
        height: row.get(12)?,
        thumbnail_path: row.get(13)?,
    })
}

pub fn timeline_window(
    connection: &Connection,
    root: &Path,
    month: &str,
    offset: usize,
    limit: usize,
) -> Result<TimelineWindow, String> {
    if !is_month_key(month) {
        return Err("时间线月份格式无效".to_string());
    }
    let month_pattern = format!("{month}%");
    let total = connection
        .query_row(
            "
            SELECT COUNT(*) FROM media
            WHERE library_root = ?1 AND captured_at LIKE ?2
            ",
            params![root.to_string_lossy(), month_pattern],
            |row| row.get(0),
        )
        .map_err(|error| format!("无法统计时间线月份：{error}"))?;

    let mut statement = connection
        .prepare(
            "
            SELECT
                m.id, m.path, m.relative_path, m.media_kind, m.extension,
                m.size_bytes, m.captured_at, m.captured_source,
                m.captured_precision, m.latitude, m.longitude, m.width, m.height,
                CASE
                    WHEN t.status = 'ready' AND t.source_modified_ns = m.modified_ns
                    THEN t.cache_path
                    ELSE NULL
                END
            FROM media m
            LEFT JOIN thumbnails t ON t.media_id = m.id
            WHERE m.library_root = ?1 AND m.captured_at LIKE ?2
            ORDER BY m.captured_at DESC, m.id DESC
            LIMIT ?3 OFFSET ?4
            ",
        )
        .map_err(|error| format!("无法准备时间线窗口查询：{error}"))?;
    let rows = statement
        .query_map(
            params![
                root.to_string_lossy(),
                month_pattern,
                limit.clamp(1, 500) as i64,
                offset as i64
            ],
            map_timeline_item,
        )
        .map_err(|error| format!("无法读取时间线窗口：{error}"))?;
    let items = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("无法解析时间线窗口：{error}"))?;

    Ok(TimelineWindow {
        month: month.to_string(),
        total,
        offset,
        items,
    })
}

pub fn timeline_item(
    connection: &Connection,
    root: &Path,
    media_id: i64,
) -> Result<Option<TimelineItem>, String> {
    connection
        .query_row(
            "
            SELECT
                m.id, m.path, m.relative_path, m.media_kind, m.extension,
                m.size_bytes, m.captured_at, m.captured_source,
                m.captured_precision, m.latitude, m.longitude, m.width, m.height,
                CASE
                    WHEN t.status = 'ready' AND t.source_modified_ns = m.modified_ns
                    THEN t.cache_path
                    ELSE NULL
                END
            FROM media m
            LEFT JOIN thumbnails t ON t.media_id = m.id
            WHERE m.library_root = ?1 AND m.id = ?2
            ",
            params![root.to_string_lossy(), media_id],
            map_timeline_item,
        )
        .optional()
        .map_err(|error| format!("无法读取媒体详情：{error}"))
}

pub fn timeline_neighbor(
    connection: &Connection,
    root: &Path,
    captured_at: &str,
    media_id: i64,
    direction: &str,
) -> Result<Option<TimelineItem>, String> {
    let (comparison, ordering) = match direction {
        "newer" => (">", "ASC"),
        "older" => ("<", "DESC"),
        _ => return Err("媒体导航方向无效".to_string()),
    };
    let sql = format!(
        "
        SELECT
            m.id, m.path, m.relative_path, m.media_kind, m.extension,
            m.size_bytes, m.captured_at, m.captured_source,
            m.captured_precision, m.latitude, m.longitude, m.width, m.height,
            CASE
                WHEN t.status = 'ready' AND t.source_modified_ns = m.modified_ns
                THEN t.cache_path
                ELSE NULL
            END
        FROM media m
        LEFT JOIN thumbnails t ON t.media_id = m.id
        WHERE m.library_root = ?1
          AND (m.captured_at, m.id) {comparison} (?2, ?3)
        ORDER BY m.captured_at {ordering}, m.id {ordering}
        LIMIT 1
        "
    );
    connection
        .query_row(
            &sql,
            params![root.to_string_lossy(), captured_at, media_id],
            map_timeline_item,
        )
        .optional()
        .map_err(|error| format!("无法读取相邻媒体：{error}"))
}

fn is_month_key(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 7
        && bytes[0..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && value[5..7]
            .parse::<u8>()
            .is_ok_and(|month| (1..=12).contains(&month))
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
) -> Result<Option<(i64, i64, i64)>, String> {
    transaction
        .query_row(
            "SELECT size_bytes, modified_ns, metadata_version FROM media WHERE path = ?1",
            [path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
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
                metadata_error, metadata_version, last_seen_scan, indexed_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                ?13, ?14, ?15, ?16, ?17, ?18
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
                metadata_version = excluded.metadata_version,
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
                METADATA_VERSION,
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
                    needs_metadata_refresh: false,
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
    summary.needs_metadata_refresh = connection
        .query_row(
            "
            SELECT EXISTS(
                SELECT 1 FROM media
                WHERE library_root = ?1 AND metadata_version < ?2
            )
            ",
            params![root.as_ref(), METADATA_VERSION],
            |row| row.get(0),
        )
        .map_err(|error| format!("无法读取元数据索引版本：{error}"))?;

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        env, fs,
        time::{Instant, SystemTime, UNIX_EPOCH},
    };

    fn timeline_fixture() -> (PathBuf, PathBuf, Connection) {
        let unique = format!(
            "time-album-timeline-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        );
        let base = env::temp_dir().join(unique);
        let root = base.join("library");
        fs::create_dir_all(&root).expect("create fixture library");
        let database_path = base.join("data").join("timeline.sqlite3");
        let connection = open_at(&database_path, &root).expect("open fixture database");
        let root_text = root.to_string_lossy().to_string();
        connection
            .execute(
                "INSERT INTO scans (library_root, started_at, status) VALUES (?1, ?2, 'completed')",
                params![root_text, "2025-03-01T00:00:00"],
            )
            .expect("insert fixture scan");
        let scan_id = connection.last_insert_rowid();
        for (index, (relative, captured, kind)) in [
            ("2025/02/new.jpg", "2025-02-15T10:00:00", "photo"),
            ("2025/02/old.mp4", "2025-02-01T09:00:00", "video"),
            ("2025/01/first.jpg", "2025-01-20T08:00:00", "photo"),
        ]
        .iter()
        .enumerate()
        {
            let path = root.join(relative);
            connection
                .execute(
                    "
                    INSERT INTO media (
                        library_root, path, relative_path, media_kind, extension,
                        size_bytes, modified_ns, captured_at, captured_source,
                        captured_precision, last_seen_scan, indexed_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, 100, ?6, ?7, 'filename', 'second', ?8, ?9)
                    ",
                    params![
                        root_text,
                        path.to_string_lossy(),
                        relative,
                        kind,
                        if *kind == "photo" { "jpg" } else { "mp4" },
                        index as i64,
                        captured,
                        scan_id,
                        "2025-03-01T00:00:00"
                    ],
                )
                .expect("insert fixture media");
        }
        (base, root, connection)
    }

    #[test]
    fn validates_month_keys() {
        assert!(is_month_key("2025-01"));
        assert!(is_month_key("2096-12"));
        assert!(!is_month_key("2025-00"));
        assert!(!is_month_key("2025-13"));
        assert!(!is_month_key("2025-1"));
    }

    #[test]
    fn timeline_queries_are_grouped_windowed_and_navigable() {
        let (base, root, connection) = timeline_fixture();
        let months = timeline_months(&connection, &root).expect("query months");
        assert_eq!(months.len(), 2);
        assert_eq!(months[0].key, "2025-02");
        assert_eq!(months[0].total, 2);
        assert_eq!(months[0].photos, 1);
        assert_eq!(months[0].videos, 1);

        let first = timeline_window(&connection, &root, "2025-02", 0, 1)
            .expect("query first timeline window");
        assert_eq!(first.total, 2);
        assert_eq!(first.items.len(), 1);
        assert_eq!(first.items[0].relative_path, "2025/02/new.jpg");

        let selected = thumbnail_candidates_by_ids(&connection, &root, &[first.items[0].id])
            .expect("query selected thumbnail candidate");
        assert_eq!(selected.len(), 1);
        let cache_path = base.join("cache").join("first.jpg");
        record_thumbnail(
            &connection,
            first.items[0].id,
            0,
            Some(&cache_path),
            Some(512),
            Some(512),
            1_024,
            "ready",
            None,
            "2025-03-01T00:00:00",
        )
        .expect("record selected thumbnail");
        assert!(
            thumbnail_candidates_by_ids(&connection, &root, &[first.items[0].id])
                .expect("query completed selected thumbnail")
                .is_empty()
        );

        let older = timeline_neighbor(
            &connection,
            &root,
            &first.items[0].captured_at,
            first.items[0].id,
            "older",
        )
        .expect("query older neighbor")
        .expect("older neighbor exists");
        assert_eq!(older.relative_path, "2025/02/old.mp4");

        drop(connection);
        fs::remove_dir_all(base).expect("remove timeline fixture");
    }

    #[test]
    #[ignore = "requires TIME_ALBUM_REAL_DB"]
    fn real_timeline_query_when_explicitly_requested() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("project lives directly inside the media root")
            .to_path_buf();
        let root = safety::canonical_existing(&root).expect("canonicalize real library");
        let database_path =
            PathBuf::from(env::var("TIME_ALBUM_REAL_DB").expect("TIME_ALBUM_REAL_DB must be set"));
        let started = Instant::now();
        let connection = open_at(&database_path, &root).expect("open real database");
        let months = timeline_months(&connection, &root).expect("query real months");
        let total: i64 = months.iter().map(|month| month.total).sum();
        assert_eq!(total, 24_003);
        let first = timeline_window(&connection, &root, &months[0].key, 0, 80)
            .expect("query real timeline window");
        assert!(!first.items.is_empty());
        println!(
            "months={}, first_window={}, elapsed_ms={}",
            months.len(),
            first.items.len(),
            started.elapsed().as_millis()
        );
    }
}
