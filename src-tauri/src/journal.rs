use std::{
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use chrono::Utc;
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use walkdir::WalkDir;

use crate::{database, safety};

const SETTINGS_FILE: &str = "journal-settings.json";

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct JournalSettings {
    journal_root: String,
    vault_root: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalSummary {
    pub journal_root: String,
    pub vault_root: String,
    pub display_name: String,
    pub total: i64,
    pub first_date: Option<String>,
    pub last_date: Option<String>,
    pub last_scan_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalScanReport {
    pub discovered: usize,
    pub indexed: usize,
    pub unchanged: usize,
    pub skipped: usize,
    pub removed: usize,
    pub summary: JournalSummary,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalMonth {
    pub key: String,
    pub total: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalEntry {
    pub id: i64,
    pub entry_date: String,
    pub title: String,
    pub content: String,
    pub path: String,
    pub relative_path: String,
    pub attachments: Vec<String>,
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    let _ = app;
    crate::storage::file(SETTINGS_FILE)
}

fn find_vault_root(journal_root: &Path) -> Result<PathBuf, String> {
    for ancestor in journal_root.ancestors() {
        if ancestor.join(".obsidian").is_dir() {
            return Ok(ancestor.to_path_buf());
        }
    }
    Err("所选目录不在有效的 Obsidian Vault 中（未找到 .obsidian）".to_string())
}

fn load_settings(app: &AppHandle) -> Result<Option<JournalSettings>, String> {
    let path = settings_path(app)?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path).map_err(|error| format!("无法读取日记设置：{error}"))?;
    serde_json::from_str(&raw)
        .map(Some)
        .map_err(|error| format!("日记设置格式错误：{error}"))
}

pub fn configure(app: &AppHandle, media_root: &Path, root: &str) -> Result<JournalSummary, String> {
    let journal_root = safety::canonical_existing(Path::new(root))?;
    if !journal_root.is_dir() {
        return Err("所选日记路径不是文件夹".to_string());
    }
    fs::read_dir(&journal_root).map_err(|error| format!("日记目录不可读取：{error}"))?;
    let vault_root = find_vault_root(&journal_root)?;
    let settings = JournalSettings {
        journal_root: journal_root.to_string_lossy().to_string(),
        vault_root: vault_root.to_string_lossy().to_string(),
    };
    let bytes = serde_json::to_vec_pretty(&settings)
        .map_err(|error| format!("无法生成日记设置：{error}"))?;
    let target = settings_path(app)?;
    safety::ensure_write_outside_library(&target, &journal_root)?;
    safety::write_bytes_outside_library(&target, &bytes, media_root)?;
    let connection = database::open(app, media_root)?;
    summary_from(&connection, &settings)
}

pub fn current(app: &AppHandle, media_root: &Path) -> Result<Option<JournalSummary>, String> {
    let Some(settings) = load_settings(app)? else {
        return Ok(None);
    };
    let journal_root = safety::canonical_existing(Path::new(&settings.journal_root))?;
    if !journal_root.is_dir() {
        return Ok(None);
    }
    let connection = database::open(app, media_root)?;
    summary_from(&connection, &settings).map(Some)
}

fn file_modified_ns(path: &Path) -> Result<i64, String> {
    let modified = fs::metadata(path)
        .and_then(|m| m.modified())
        .map_err(|error| format!("无法读取日记文件属性：{error}"))?;
    let duration = modified
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "日记文件时间早于 Unix 纪元".to_string())?;
    Ok(duration.as_nanos().min(i64::MAX as u128) as i64)
}

fn parse_entry(path: &Path, raw: &str) -> Option<(String, String, String)> {
    let filename_re = Regex::new(r"^(\d{4})(\d{2})(\d{2})").ok()?;
    let date_re = Regex::new(r#"(?m)^date:\s*["']?(\d{4})-(\d{2})-(\d{2})"#).ok()?;
    let title_re = Regex::new(r#"(?m)^title:\s*["']?([^\r\n"']+)"#).ok()?;
    let heading_re = Regex::new(r"(?m)^#\s+(.+)$").ok()?;
    let name = path.file_stem()?.to_string_lossy();
    let captures = date_re
        .captures(raw)
        .or_else(|| filename_re.captures(&name))?;
    let date = format!("{}-{}-{}", &captures[1], &captures[2], &captures[3]);
    chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d").ok()?;
    let title = title_re
        .captures(raw)
        .map(|c| c[1].trim().to_string())
        .or_else(|| heading_re.captures(raw).map(|c| c[1].trim().to_string()))
        .unwrap_or_else(|| name.to_string());
    let content = if raw.starts_with("---") {
        raw[3..]
            .find("\n---")
            .map(|end| raw[end + 7..].trim().to_string())
            .unwrap_or_else(|| raw.trim().to_string())
    } else {
        raw.trim().to_string()
    };
    Some((date, title, content))
}

pub fn scan(app: &AppHandle, media_root: &Path) -> Result<JournalScanReport, String> {
    let settings = load_settings(app)?.ok_or_else(|| "请先选择 Obsidian 日记目录".to_string())?;
    let journal_root = safety::canonical_existing(Path::new(&settings.journal_root))?;
    let mut connection = database::open(app, media_root)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("无法开始日记索引：{error}"))?;
    let scan_at = Utc::now().to_rfc3339();
    let mut discovered = 0usize;
    let mut indexed = 0usize;
    let mut unchanged = 0usize;
    let mut skipped = 0usize;
    let mut seen = Vec::new();

    for entry in WalkDir::new(&journal_root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if !entry.file_type().is_file()
            || !path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
        {
            continue;
        }
        discovered += 1;
        let canonical = safety::canonical_existing(path)?;
        let path_text = canonical.to_string_lossy().to_string();
        let size = fs::metadata(&canonical)
            .map_err(|error| format!("无法读取日记属性：{error}"))?
            .len() as i64;
        let modified_ns = file_modified_ns(&canonical)?;
        seen.push(path_text.clone());
        let existing: Option<(i64, i64)> = transaction
            .query_row(
                "SELECT size_bytes, modified_ns FROM journal_entries WHERE path = ?1",
                [&path_text],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| format!("无法检查日记索引：{error}"))?;
        if existing == Some((size, modified_ns)) {
            unchanged += 1;
            continue;
        }
        let raw = match fs::read_to_string(&canonical) {
            Ok(value) => value,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        let Some((entry_date, title, content)) = parse_entry(&canonical, &raw) else {
            skipped += 1;
            continue;
        };
        let relative = canonical
            .strip_prefix(&journal_root)
            .unwrap_or(&canonical)
            .to_string_lossy()
            .to_string();
        transaction.execute(
            "INSERT INTO journal_entries (journal_root, vault_root, path, relative_path, entry_date, title, content, size_bytes, modified_ns, indexed_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
             ON CONFLICT(path) DO UPDATE SET journal_root=excluded.journal_root, vault_root=excluded.vault_root,
             relative_path=excluded.relative_path, entry_date=excluded.entry_date, title=excluded.title,
             content=excluded.content, size_bytes=excluded.size_bytes, modified_ns=excluded.modified_ns, indexed_at=excluded.indexed_at",
            params![settings.journal_root, settings.vault_root, path_text, relative, entry_date, title, content, size, modified_ns, scan_at],
        ).map_err(|error| format!("无法写入日记索引：{error}"))?;
        indexed += 1;
    }
    let old_paths: Vec<String> = {
        let mut statement = transaction
            .prepare("SELECT path FROM journal_entries WHERE journal_root = ?1")
            .map_err(|error| format!("无法检查已删除日记：{error}"))?;
        let paths = statement
            .query_map([&settings.journal_root], |row| row.get(0))
            .map_err(|error| format!("无法读取日记路径：{error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("无法解析日记路径：{error}"))?;
        paths
    };
    let mut removed = 0;
    for path in old_paths {
        if !seen.contains(&path) {
            removed += transaction
                .execute("DELETE FROM journal_entries WHERE path=?1", [&path])
                .map_err(|error| format!("无法删除失效日记索引：{error}"))?;
        }
    }
    transaction.execute("INSERT INTO journal_sources (journal_root, vault_root, last_scan_at) VALUES (?1,?2,?3) ON CONFLICT(journal_root) DO UPDATE SET vault_root=excluded.vault_root,last_scan_at=excluded.last_scan_at", params![settings.journal_root, settings.vault_root, scan_at])
        .map_err(|error| format!("无法保存日记扫描状态：{error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("无法提交日记索引：{error}"))?;
    let connection = database::open(app, media_root)?;
    let summary = summary_from(&connection, &settings)?;
    Ok(JournalScanReport {
        discovered,
        indexed,
        unchanged,
        skipped,
        removed,
        summary,
    })
}

fn summary_from(
    connection: &Connection,
    settings: &JournalSettings,
) -> Result<JournalSummary, String> {
    let (total, first_date, last_date): (i64, Option<String>, Option<String>) = connection.query_row(
        "SELECT COUNT(*), MIN(entry_date), MAX(entry_date) FROM journal_entries WHERE journal_root=?1",
        [&settings.journal_root], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(|error| format!("无法统计日记：{error}"))?;
    let last_scan_at = connection
        .query_row(
            "SELECT last_scan_at FROM journal_sources WHERE journal_root=?1",
            [&settings.journal_root],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("无法读取日记扫描时间：{error}"))?;
    let display_name = Path::new(&settings.vault_root)
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("Obsidian")
        .to_string();
    Ok(JournalSummary {
        journal_root: settings.journal_root.clone(),
        vault_root: settings.vault_root.clone(),
        display_name,
        total,
        first_date,
        last_date,
        last_scan_at,
    })
}

pub fn months(app: &AppHandle, media_root: &Path) -> Result<Vec<JournalMonth>, String> {
    let settings = load_settings(app)?.ok_or_else(|| "请先选择日记目录".to_string())?;
    let connection = database::open(app, media_root)?;
    let mut statement = connection.prepare("SELECT substr(entry_date,1,7), COUNT(*) FROM journal_entries WHERE journal_root=?1 GROUP BY 1 ORDER BY 1 DESC")
        .map_err(|error| format!("无法准备日记月份：{error}"))?;
    let result = statement
        .query_map([settings.journal_root], |row| {
            Ok(JournalMonth {
                key: row.get(0)?,
                total: row.get(1)?,
            })
        })
        .map_err(|error| format!("无法读取日记月份：{error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("无法解析日记月份：{error}"));
    result
}

pub fn entries(
    app: &AppHandle,
    media_root: &Path,
    month: &str,
) -> Result<Vec<JournalEntry>, String> {
    if !Regex::new(r"^\d{4}-\d{2}$").unwrap().is_match(month) {
        return Err("日记月份格式无效".to_string());
    }
    let settings = load_settings(app)?.ok_or_else(|| "请先选择日记目录".to_string())?;
    let connection = database::open(app, media_root)?;
    let mut statement = connection.prepare("SELECT id,entry_date,title,content,path,relative_path FROM journal_entries WHERE journal_root=?1 AND entry_date LIKE ?2 ORDER BY entry_date DESC,id DESC")
        .map_err(|error| format!("无法准备日记列表：{error}"))?;
    let result = statement
        .query_map(params![settings.journal_root, format!("{month}%")], |row| {
            Ok(JournalEntry {
                id: row.get(0)?,
                entry_date: row.get(1)?,
                title: row.get(2)?,
                content: row.get(3)?,
                path: row.get(4)?,
                relative_path: row.get(5)?,
                attachments: Vec::new(),
            })
        })
        .map_err(|error| format!("无法读取日记：{error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("无法解析日记：{error}"));
    let mut result = result?;
    let vault_root = safety::canonical_existing(Path::new(&settings.vault_root))?;
    let embed_re = Regex::new(r"!\[\[([^\]|#]+)").map_err(|error| error.to_string())?;
    for entry in &mut result {
        for capture in embed_re.captures_iter(&entry.content) {
            let target = capture[1].trim().replace('/', "\\");
            let candidates = [
                vault_root.join(&target),
                vault_root.join("Attachment").join(&target),
            ];
            if let Some(path) = candidates
                .into_iter()
                .find_map(|path| safety::canonical_existing(&path).ok())
            {
                if safety::is_same_or_descendant(&path, &vault_root) && path.is_file() {
                    app.asset_protocol_scope()
                        .allow_file(&path)
                        .map_err(|error| format!("无法授权日记附件只读展示：{error}"))?;
                    entry.attachments.push(path.to_string_lossy().to_string());
                }
            }
        }
    }
    Ok(result)
}

pub fn entries_for_date(
    app: &AppHandle,
    media_root: &Path,
    date: &str,
) -> Result<Vec<JournalEntry>, String> {
    let month = date.get(0..7).ok_or_else(|| "日期格式无效".to_string())?;
    Ok(entries(app, media_root, month)?
        .into_iter()
        .filter(|entry| entry.entry_date == date)
        .collect())
}

pub fn open_in_obsidian(app: &AppHandle, path: &str) -> Result<(), String> {
    let settings = load_settings(app)?.ok_or_else(|| "请先选择日记目录".to_string())?;
    let journal_root = safety::canonical_existing(Path::new(&settings.journal_root))?;
    let note = safety::canonical_existing(Path::new(path))?;
    if !safety::is_same_or_descendant(&note, &journal_root) || !note.is_file() {
        return Err("拒绝打开：目标不在已配置的日记目录中".to_string());
    }
    let uri = format!(
        "obsidian://open?path={}",
        urlencoding::encode(&note.to_string_lossy())
    );
    std::process::Command::new("explorer.exe")
        .arg(uri)
        .spawn()
        .map_err(|error| format!("无法打开 Obsidian：{error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_entry;
    use std::path::Path;

    #[test]
    fn parses_diary_filename_without_frontmatter() {
        let entry = parse_entry(Path::new("20240222_周四.md"), "### 今天\n\n正文")
            .expect("parse diary filename");
        assert_eq!(entry.0, "2024-02-22");
        assert_eq!(entry.1, "20240222_周四");
        assert!(entry.2.contains("正文"));
    }

    #[test]
    fn frontmatter_date_and_title_take_priority() {
        let raw = "---\ndate: 2026-07-17\ntitle: 一天\n---\n### 正文";
        let entry = parse_entry(Path::new("note.md"), raw).expect("parse frontmatter");
        assert_eq!(entry.0, "2026-07-17");
        assert_eq!(entry.1, "一天");
        assert_eq!(entry.2, "### 正文");
    }
}
