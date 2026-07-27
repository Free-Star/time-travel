use std::{
    fs,
    path::{Path, PathBuf},
};

use tauri::{AppHandle, Manager};

/// All mutable application-owned files live beside the executable so a
/// current-user installation is self-contained and can be removed cleanly.
pub fn data_dir() -> Result<PathBuf, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("无法确定 TimeTravel 安装目录：{error}"))?;
    executable
        .parent()
        .map(|directory| directory.join("data"))
        .ok_or_else(|| "TimeTravel 可执行文件没有父目录".to_string())
}

pub fn file(name: &str) -> Result<PathBuf, String> {
    Ok(data_dir()?.join(name))
}

/// Move application-owned data from the pre self-contained layout. Source
/// media and Obsidian files are never part of this migration.
pub fn migrate_legacy_layout(app: &AppHandle) -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Ok(());
    }
    let destination = data_dir()?;
    fs::create_dir_all(&destination)
        .map_err(|error| format!("无法创建安装目录数据文件夹：{error}"))?;

    let legacy_data = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法确定旧应用数据目录：{error}"))?;
    let legacy_config = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("无法确定旧配置目录：{error}"))?;
    let legacy_cache = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定旧缓存目录：{error}"))?;

    for name in [
        "settings.json",
        "journal-settings.json",
        "time-album.sqlite3",
    ] {
        let source = if legacy_config.join(name).is_file() {
            legacy_config.join(name)
        } else {
            legacy_data.join(name)
        };
        move_if_needed(&source, &destination.join(name))?;
    }
    move_if_needed(
        &legacy_cache.join("thumbnails"),
        &destination.join("thumbnails"),
    )
    .and_then(|_| {
        move_if_needed(
            &legacy_cache.join("EBWebView"),
            &destination.join("webview"),
        )
    })
}

fn move_if_needed(source: &Path, destination: &Path) -> Result<(), String> {
    if destination.exists() || !source.exists() {
        return Ok(());
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("无法准备迁移目录：{error}"))?;
    }
    if fs::rename(source, destination).is_ok() {
        return Ok(());
    }
    copy_recursively(source, destination)?;
    if source.is_dir() {
        fs::remove_dir_all(source)
    } else {
        fs::remove_file(source)
    }
    .map_err(|error| {
        format!(
            "工作文件已迁移，但无法清理旧位置（{}）：{error}",
            source.display()
        )
    })
}

fn copy_recursively(source: &Path, destination: &Path) -> Result<(), String> {
    if source.is_file() {
        fs::copy(source, destination)
            .map(|_| ())
            .map_err(|error| format!("无法迁移旧版工作文件（{}）：{error}", source.display()))?;
        return Ok(());
    }
    fs::create_dir_all(destination).map_err(|error| format!("无法创建迁移目标：{error}"))?;
    for entry in fs::read_dir(source).map_err(|error| format!("无法读取旧工作目录：{error}"))?
    {
        let entry = entry.map_err(|error| format!("无法读取旧工作文件：{error}"))?;
        copy_recursively(&entry.path(), &destination.join(entry.file_name()))?;
    }
    Ok(())
}
