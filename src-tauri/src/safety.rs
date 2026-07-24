use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

/// Resolve an existing path to its canonical form.
///
/// Every media path passes through this function before it can be opened.
pub fn canonical_existing(path: &Path) -> Result<PathBuf, String> {
    fs::canonicalize(path).map_err(|error| format!("无法访问路径 {}：{error}", path.display()))
}

pub fn workspace_dir() -> Result<PathBuf, String> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest
        .parent()
        .ok_or_else(|| "无法确定开发工具目录".to_string())?;
    canonical_existing(workspace)
}

/// Return true when `candidate` is `root` itself or one of its descendants.
///
/// Both paths must already be absolute/canonical at runtime.
pub fn is_same_or_descendant(candidate: &Path, root: &Path) -> bool {
    candidate == root || candidate.starts_with(root)
}

/// Open a media file with the standard library's read-only `File::open`.
///
/// There is deliberately no write-capable media handle in the application.
#[allow(dead_code)]
pub fn open_media_readonly(path: &Path, library_root: &Path) -> Result<File, String> {
    let root = canonical_existing(library_root)?;
    let media = canonical_existing(path)?;

    if !is_same_or_descendant(&media, &root) {
        return Err("拒绝读取：文件不在已选择的相册目录中".to_string());
    }
    if !media.is_file() {
        return Err("拒绝读取：目标不是媒体文件".to_string());
    }

    File::open(&media).map_err(|error| format!("无法以只读方式打开媒体：{error}"))
}

/// Resolve a future write target by canonicalizing its nearest existing parent.
///
/// This catches `..` segments and directory junctions before the write happens.
fn resolve_future_target(target: &Path) -> Result<PathBuf, String> {
    let mut existing = target;
    let mut missing_parts = Vec::new();

    while !existing.exists() {
        let name = existing
            .file_name()
            .ok_or_else(|| format!("无效的写入路径：{}", target.display()))?;
        missing_parts.push(name.to_os_string());
        existing = existing
            .parent()
            .ok_or_else(|| format!("无效的写入路径：{}", target.display()))?;
    }

    let mut resolved = canonical_existing(existing)?;
    for part in missing_parts.iter().rev() {
        resolved.push(part);
    }
    Ok(resolved)
}

/// Reject every application write that resolves inside the selected library.
pub fn ensure_write_outside_library(target: &Path, library_root: &Path) -> Result<(), String> {
    let root = canonical_existing(library_root)?;
    let resolved_target = resolve_future_target(target)?;

    if is_same_or_descendant(&resolved_target, &root) {
        return Err(format!(
            "安全策略拒绝写入相册目录：{}",
            resolved_target.display()
        ));
    }
    Ok(())
}

/// The single application-level write entry point.
///
/// Future database, cache and settings code must use this function (or a
/// similarly guarded database connection) rather than writing beside media.
pub fn write_bytes_outside_library(
    target: &Path,
    bytes: &[u8],
    library_root: &Path,
) -> Result<(), String> {
    ensure_write_outside_library(target, library_root)?;

    let parent = target
        .parent()
        .ok_or_else(|| format!("写入路径没有父目录：{}", target.display()))?;
    fs::create_dir_all(parent).map_err(|error| format!("无法创建应用配置目录：{error}"))?;

    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(target)
        .map_err(|error| format!("无法写入应用配置：{error}"))?;
    file.write_all(bytes)
        .map_err(|error| format!("无法完成应用配置写入：{error}"))
}

pub fn create_directory_outside_library(
    directory: &Path,
    library_root: &Path,
) -> Result<(), String> {
    ensure_write_outside_library(directory, library_root)?;
    fs::create_dir_all(directory).map_err(|error| format!("无法创建应用数据目录：{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_descendants_component_by_component() {
        let root = Path::new("E:/photos");
        assert!(is_same_or_descendant(
            Path::new("E:/photos/2024/a.jpg"),
            root
        ));
        assert!(!is_same_or_descendant(
            Path::new("E:/photos-backup/a.jpg"),
            root
        ));
    }

    #[test]
    fn rejects_a_real_target_inside_the_library() {
        let test_root = std::env::temp_dir().join("time-album-safety-library");
        let target = test_root.join("2024").join("photo.jpg");
        fs::create_dir_all(target.parent().expect("target has parent"))
            .expect("create test directories");

        let result = ensure_write_outside_library(&target, &test_root);
        assert!(result.is_err());

        fs::remove_dir_all(test_root).expect("remove test directories");
    }

    #[test]
    fn allows_a_sibling_cache_target() {
        let temp = std::env::temp_dir().join("time-album-safety-siblings");
        let library = temp.join("library");
        let cache = temp.join("cache").join("settings.json");
        fs::create_dir_all(&library).expect("create library");
        fs::create_dir_all(cache.parent().expect("cache has parent")).expect("create cache");

        let result = ensure_write_outside_library(&cache, &library);
        assert!(result.is_ok());

        fs::remove_dir_all(temp).expect("remove test directories");
    }
}
