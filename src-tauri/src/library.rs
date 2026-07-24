use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::safety;

const SETTINGS_FILE: &str = "settings.json";
const MEDIA_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "heif", "heic", "dng", "mp4", "mov", "3gp", "avi", "mkv", "vob",
];

#[derive(Default)]
pub struct AppState {
    library_root: Mutex<Option<PathBuf>>,
}

impl AppState {
    pub fn set_root(&self, root: PathBuf) -> Result<(), String> {
        let mut guard = self
            .library_root
            .lock()
            .map_err(|_| "相册状态锁已损坏".to_string())?;
        *guard = Some(root);
        Ok(())
    }

    fn root(&self) -> Result<Option<PathBuf>, String> {
        self.library_root
            .lock()
            .map(|guard| guard.clone())
            .map_err(|_| "相册状态锁已损坏".to_string())
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySummary {
    root: String,
    display_name: String,
    top_level_folders: usize,
    top_level_media: usize,
    project_directory_excluded: bool,
    write_policy: &'static str,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SavedSettings {
    library_root: String,
}

pub fn configure(
    app: &AppHandle,
    state: &State<'_, AppState>,
    root: String,
) -> Result<LibrarySummary, String> {
    let canonical_root = validate_library_root(Path::new(&root))?;
    save_root(app, &canonical_root)?;
    state.set_root(canonical_root.clone())?;
    summarize(&canonical_root)
}

pub fn current(state: &State<'_, AppState>) -> Result<Option<LibrarySummary>, String> {
    state.root()?.map(|root| summarize(&root)).transpose()
}

pub fn load_saved_root(app: &AppHandle) -> Result<Option<PathBuf>, String> {
    let path = settings_path(app)?;
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(&path).map_err(|error| format!("无法读取应用设置：{error}"))?;
    let settings: SavedSettings =
        serde_json::from_str(&raw).map_err(|error| format!("应用设置格式错误：{error}"))?;

    validate_library_root(Path::new(&settings.library_root)).map(Some)
}

fn validate_library_root(root: &Path) -> Result<PathBuf, String> {
    let canonical_root = safety::canonical_existing(root)?;
    if !canonical_root.is_dir() {
        return Err("所选路径不是文件夹".to_string());
    }

    let workspace = workspace_dir()?;
    if safety::is_same_or_descendant(&canonical_root, &workspace) {
        return Err("不能把开发工具目录本身选作相册".to_string());
    }

    fs::read_dir(&canonical_root).map_err(|error| format!("所选相册目录不可读取：{error}"))?;
    Ok(canonical_root)
}

fn summarize(root: &Path) -> Result<LibrarySummary, String> {
    let workspace = workspace_dir()?;
    let mut top_level_folders = 0;
    let mut top_level_media = 0;

    for entry in fs::read_dir(root).map_err(|error| format!("无法读取相册目录：{error}"))?
    {
        let entry = entry.map_err(|error| format!("无法读取目录项：{error}"))?;
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
    })
}

fn is_supported_media(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            MEDIA_EXTENSIONS
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
        .unwrap_or(false)
}

fn workspace_dir() -> Result<PathBuf, String> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest
        .parent()
        .ok_or_else(|| "无法确定开发工具目录".to_string())?;
    safety::canonical_existing(workspace)
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|directory| directory.join(SETTINGS_FILE))
        .map_err(|error| format!("无法确定应用配置目录：{error}"))
}

fn save_root(app: &AppHandle, root: &Path) -> Result<(), String> {
    let settings = SavedSettings {
        library_root: root.to_string_lossy().to_string(),
    };
    let bytes = serde_json::to_vec_pretty(&settings)
        .map_err(|error| format!("无法生成应用设置：{error}"))?;
    let target = settings_path(app)?;
    safety::write_bytes_outside_library(&target, &bytes, root)
}
