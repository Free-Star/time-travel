use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::safety;

const SETTINGS_FILE: &str = "settings.json";
const MEDIA_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "heif", "heic", "dng", "mp4", "mov", "3gp", "avi", "mkv", "vob",
];

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedSettings {
    #[serde(default)]
    library_roots: Vec<String>,
    #[serde(default)]
    active_library_root: Option<String>,
    // Migrates settings written by TimeTravel <= 0.0.2.
    #[serde(default)]
    library_root: Option<String>,
}

#[derive(Default)]
pub struct AppState {
    settings: Mutex<SavedSettings>,
}

impl AppState {
    pub fn replace_settings(&self, settings: SavedSettings) -> Result<(), String> {
        *self
            .settings
            .lock()
            .map_err(|_| "相册状态锁已损坏".to_string())? = settings;
        Ok(())
    }

    fn settings(&self) -> Result<SavedSettings, String> {
        self.settings
            .lock()
            .map(|value| value.clone())
            .map_err(|_| "相册状态锁已损坏".to_string())
    }

    pub fn root(&self) -> Result<Option<PathBuf>, String> {
        Ok(self.settings()?.active_library_root.map(PathBuf::from))
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySummary {
    pub root: String,
    pub display_name: String,
    pub top_level_folders: usize,
    pub top_level_media: usize,
    pub project_directory_excluded: bool,
    pub write_policy: &'static str,
    pub online: bool,
    pub active: bool,
}

pub fn configure(
    app: &AppHandle,
    state: &State<'_, AppState>,
    root: String,
) -> Result<LibrarySummary, String> {
    let canonical_root = validate_library_root(Path::new(&root))?;
    let root_text = canonical_root.to_string_lossy().to_string();
    let mut settings = state.settings()?;
    if !settings
        .library_roots
        .iter()
        .any(|item| same_path(item, &root_text))
    {
        settings.library_roots.push(root_text.clone());
    }
    settings.active_library_root = Some(root_text);
    settings.library_root = None;
    save_settings(app, &settings, &canonical_root)?;
    state.replace_settings(settings)?;
    summarize(&canonical_root, true)
}

pub fn activate(
    app: &AppHandle,
    state: &State<'_, AppState>,
    root: String,
) -> Result<LibrarySummary, String> {
    let canonical_root = validate_library_root(Path::new(&root))?;
    let mut settings = state.settings()?;
    if !settings
        .library_roots
        .iter()
        .any(|item| same_path(item, &root))
    {
        return Err("该目录尚未加入相册库".to_string());
    }
    settings.active_library_root = Some(canonical_root.to_string_lossy().to_string());
    save_settings(app, &settings, &canonical_root)?;
    state.replace_settings(settings)?;
    summarize(&canonical_root, true)
}

pub fn remove(
    app: &AppHandle,
    state: &State<'_, AppState>,
    root: String,
) -> Result<Option<LibrarySummary>, String> {
    let mut settings = state.settings()?;
    settings
        .library_roots
        .retain(|item| !same_path(item, &root));
    if settings
        .active_library_root
        .as_deref()
        .is_some_and(|item| same_path(item, &root))
    {
        settings.active_library_root = settings.library_roots.first().cloned();
    }
    let guard = settings
        .library_roots
        .iter()
        .map(PathBuf::from)
        .find(|path| path.is_dir())
        .unwrap_or_else(|| safety::workspace_dir().unwrap_or_else(|_| PathBuf::from(".")));
    save_settings(app, &settings, &guard)?;
    state.replace_settings(settings.clone())?;
    settings
        .active_library_root
        .map(|path| summarize(Path::new(&path), true))
        .transpose()
}

pub fn current(state: &State<'_, AppState>) -> Result<Option<LibrarySummary>, String> {
    state.root()?.map(|root| summarize(&root, true)).transpose()
}

pub fn all(state: &State<'_, AppState>) -> Result<Vec<LibrarySummary>, String> {
    let settings = state.settings()?;
    settings
        .library_roots
        .iter()
        .map(|root| {
            let active = settings
                .active_library_root
                .as_deref()
                .is_some_and(|value| same_path(value, root));
            summarize(Path::new(root), active)
        })
        .collect()
}

pub fn load_saved_settings(app: &AppHandle) -> Result<SavedSettings, String> {
    let path = settings_path(app)?;
    if !path.exists() {
        return Ok(SavedSettings::default());
    }
    let raw = fs::read_to_string(&path).map_err(|error| format!("无法读取应用设置：{error}"))?;
    let mut settings: SavedSettings =
        serde_json::from_str(&raw).map_err(|error| format!("应用设置格式错误：{error}"))?;
    if settings.library_roots.is_empty() {
        if let Some(root) = settings.library_root.take() {
            settings.library_roots.push(root.clone());
            settings.active_library_root = Some(root);
        }
    }
    if settings.active_library_root.is_none() {
        settings.active_library_root = settings.library_roots.first().cloned();
    }
    Ok(settings)
}

fn validate_library_root(root: &Path) -> Result<PathBuf, String> {
    let canonical_root = safety::canonical_existing(root)?;
    if !canonical_root.is_dir() {
        return Err("所选路径不是文件夹".to_string());
    }
    let workspace = safety::workspace_dir()?;
    if safety::is_same_or_descendant(&canonical_root, &workspace) {
        return Err("不能把开发工具目录本身选作相册".to_string());
    }
    fs::read_dir(&canonical_root).map_err(|error| format!("所选相册目录不可读取：{error}"))?;
    Ok(canonical_root)
}

fn summarize(root: &Path, active: bool) -> Result<LibrarySummary, String> {
    let online = root.is_dir() && fs::read_dir(root).is_ok();
    let workspace = safety::workspace_dir()?;
    let mut top_level_folders = 0;
    let mut top_level_media = 0;
    if online {
        for entry in fs::read_dir(root).map_err(|error| format!("无法读取相册目录：{error}"))?
        {
            let Ok(entry) = entry else { continue };
            let canonical = match safety::canonical_existing(&entry.path()) {
                Ok(path) => path,
                Err(_) => continue,
            };
            if safety::is_same_or_descendant(&canonical, &workspace) {
                continue;
            }
            if canonical.is_dir() {
                top_level_folders += 1;
            } else if is_supported_media(&canonical) {
                top_level_media += 1;
            }
        }
    }
    let display_name = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("相册")
        .to_string();
    Ok(LibrarySummary {
        root: root.to_string_lossy().to_string(),
        display_name,
        top_level_folders,
        top_level_media,
        project_directory_excluded: safety::is_same_or_descendant(&workspace, root),
        write_policy: "媒体目录绝对只读",
        online,
        active,
    })
}

fn same_path(left: &str, right: &str) -> bool {
    left.trim_end_matches(['\\', '/'])
        .eq_ignore_ascii_case(right.trim_end_matches(['\\', '/']))
}

fn is_supported_media(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| {
            MEDIA_EXTENSIONS
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
}

fn settings_path(_app: &AppHandle) -> Result<PathBuf, String> {
    crate::storage::file(SETTINGS_FILE)
}

fn save_settings(
    app: &AppHandle,
    settings: &SavedSettings,
    guard_root: &Path,
) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(settings)
        .map_err(|error| format!("无法生成应用设置：{error}"))?;
    safety::write_bytes_outside_library(&settings_path(app)?, &bytes, guard_root)
}
