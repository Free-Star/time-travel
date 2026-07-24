use std::{
    collections::hash_map::DefaultHasher,
    env, fs,
    hash::{Hash, Hasher},
    io::BufReader,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use chrono::Local;
use image::{codecs::jpeg::JpegEncoder, ImageReader};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::{
    database::{self, ThumbnailPreview, ThumbnailStatus},
    safety,
};

const THUMBNAIL_EDGE: u32 = 512;

#[derive(Default)]
pub struct ThumbnailControl {
    active_cancel: Mutex<Option<Arc<AtomicBool>>>,
}

impl ThumbnailControl {
    pub fn begin(&self) -> Result<Arc<AtomicBool>, String> {
        let mut guard = self
            .active_cancel
            .lock()
            .map_err(|_| "缩略图任务状态锁已损坏".to_string())?;
        if guard.is_some() {
            return Err("缩略图任务已在运行".to_string());
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
pub struct ThumbnailProgress {
    pub status: String,
    pub total: usize,
    pub processed: usize,
    pub ready: usize,
    pub failed: usize,
    pub current_path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailReport {
    pub status: String,
    pub processed: usize,
    pub ready: usize,
    pub failed: usize,
    pub thumbnail_status: ThumbnailStatus,
    pub previews: Vec<ThumbnailPreview>,
}

pub fn generate(
    app: &AppHandle,
    library_root: &Path,
    cancel: &AtomicBool,
    limit: usize,
) -> Result<ThumbnailReport, String> {
    let root = safety::canonical_existing(library_root)?;
    let cache = cache_dir(app, &root)?;
    let database_path = database::database_path(app)?;
    generate_core(
        Some(app),
        &root,
        &cache,
        &database_path,
        cancel,
        limit,
        None,
    )
}

pub fn generate_selected(
    app: &AppHandle,
    library_root: &Path,
    cancel: &AtomicBool,
    media_ids: &[i64],
) -> Result<ThumbnailReport, String> {
    if media_ids.is_empty() || media_ids.len() > 100 {
        return Err("可见预览批次必须包含 1 到 100 个媒体".to_string());
    }
    let root = safety::canonical_existing(library_root)?;
    let cache = cache_dir(app, &root)?;
    let database_path = database::database_path(app)?;
    generate_core(
        Some(app),
        &root,
        &cache,
        &database_path,
        cancel,
        media_ids.len(),
        Some(media_ids),
    )
}

fn generate_core(
    app: Option<&AppHandle>,
    library_root: &Path,
    cache: &Path,
    database_path: &Path,
    cancel: &AtomicBool,
    limit: usize,
    media_ids: Option<&[i64]>,
) -> Result<ThumbnailReport, String> {
    let root = safety::canonical_existing(library_root)?;
    safety::create_directory_outside_library(cache, &root)?;
    let connection = database::open_at(database_path, &root)?;
    let candidates = if let Some(media_ids) = media_ids {
        database::thumbnail_candidates_by_ids(&connection, &root, media_ids)?
    } else {
        database::thumbnail_candidates(&connection, &root, limit.clamp(1, 1000))?
    };
    let ffmpeg = find_ffmpeg();
    let mut progress = ThumbnailProgress {
        status: "generating".to_string(),
        total: candidates.len(),
        ..ThumbnailProgress::default()
    };

    for candidate in candidates {
        if cancel.load(Ordering::Relaxed) {
            progress.status = "cancelled".to_string();
            emit_progress(app, &progress);
            return report_from_connection(&connection, &root, progress);
        }

        progress.current_path = candidate.path.clone();
        let media_path = Path::new(&candidate.path);
        let output_directory = cache.join(format!("{:02x}", candidate.media_id.rem_euclid(256)));
        let output = output_directory.join(format!(
            "{}-{}.jpg",
            candidate.media_id, candidate.modified_ns
        ));
        safety::create_directory_outside_library(&output_directory, &root)?;

        let generated = if candidate.media_kind == "photo" {
            generate_image(media_path, &output, &root)
        } else if let Some(ffmpeg) = ffmpeg.as_deref() {
            generate_video(ffmpeg, media_path, &output, &root, cache)
        } else {
            Err("未找到 FFmpeg，无法生成视频封面".to_string())
        };

        match generated {
            Ok((width, height, bytes)) => {
                database::record_thumbnail(
                    &connection,
                    candidate.media_id,
                    candidate.modified_ns,
                    Some(&output),
                    Some(width),
                    Some(height),
                    bytes,
                    "ready",
                    None,
                    &Local::now().to_rfc3339(),
                )?;
                progress.ready += 1;
            }
            Err(error) => {
                database::record_thumbnail(
                    &connection,
                    candidate.media_id,
                    candidate.modified_ns,
                    None,
                    None,
                    None,
                    0,
                    "failed",
                    Some(&error),
                    &Local::now().to_rfc3339(),
                )?;
                progress.failed += 1;
            }
        }

        progress.processed += 1;
        emit_progress(app, &progress);
    }

    progress.status = "completed".to_string();
    progress.current_path.clear();
    emit_progress(app, &progress);
    report_from_connection(&connection, &root, progress)
}

pub fn status(app: &AppHandle, root: &Path) -> Result<ThumbnailStatus, String> {
    let connection = database::open(app, root)?;
    let mut status = database::thumbnail_status(&connection, root)?;
    status.ffmpeg_available = find_ffmpeg().is_some();
    Ok(status)
}

pub fn previews(
    app: &AppHandle,
    root: &Path,
    limit: usize,
) -> Result<Vec<ThumbnailPreview>, String> {
    let connection = database::open(app, root)?;
    let previews = database::thumbnail_previews(&connection, root, limit.clamp(1, 100))?;
    Ok(previews
        .into_iter()
        .filter(|preview| Path::new(&preview.cache_path).is_file())
        .collect())
}

pub fn clear(app: &AppHandle, root: &Path) -> Result<ThumbnailStatus, String> {
    let cache = cache_dir(app, root)?;
    safety::remove_directory_outside_library(&cache, root)?;
    let connection = database::open(app, root)?;
    database::clear_thumbnails(&connection, root)?;
    status(app, root)
}

fn report_from_connection(
    connection: &rusqlite::Connection,
    root: &Path,
    progress: ThumbnailProgress,
) -> Result<ThumbnailReport, String> {
    let mut thumbnail_status = database::thumbnail_status(connection, root)?;
    thumbnail_status.ffmpeg_available = find_ffmpeg().is_some();
    let previews = database::thumbnail_previews(connection, root, 12)?
        .into_iter()
        .filter(|preview| Path::new(&preview.cache_path).is_file())
        .collect();
    Ok(ThumbnailReport {
        status: progress.status,
        processed: progress.processed,
        ready: progress.ready,
        failed: progress.failed,
        thumbnail_status,
        previews,
    })
}

fn cache_dir(app: &AppHandle, root: &Path) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定应用缓存目录：{error}"))?;
    let mut hasher = DefaultHasher::new();
    root.to_string_lossy().to_lowercase().hash(&mut hasher);
    Ok(base
        .join("thumbnails")
        .join(format!("{:016x}", hasher.finish())))
}

fn generate_image(
    media_path: &Path,
    output: &Path,
    root: &Path,
) -> Result<(u32, u32, i64), String> {
    let input = safety::open_media_readonly(media_path, root)?;
    let reader = ImageReader::new(BufReader::new(input))
        .with_guessed_format()
        .map_err(|error| format!("无法识别图片格式：{error}"))?;
    let image = reader
        .decode()
        .map_err(|error| format!("无法解码图片：{error}"))?;
    let thumbnail = image.thumbnail(THUMBNAIL_EDGE, THUMBNAIL_EDGE);
    let width = thumbnail.width();
    let height = thumbnail.height();
    let output_file = safety::create_file_outside_library(output, root)?;
    JpegEncoder::new_with_quality(output_file, 82)
        .encode_image(&thumbnail)
        .map_err(|error| format!("无法编码缩略图：{error}"))?;
    let bytes = output
        .metadata()
        .map(|metadata| i64::try_from(metadata.len()).unwrap_or(i64::MAX))
        .map_err(|error| format!("无法读取缩略图大小：{error}"))?;
    Ok((width, height, bytes))
}

fn generate_video(
    ffmpeg: &Path,
    media_path: &Path,
    output: &Path,
    root: &Path,
    working_directory: &Path,
) -> Result<(u32, u32, i64), String> {
    drop(safety::open_media_readonly(media_path, root)?);
    safety::ensure_write_outside_library(output, root)?;

    let attempts = ["1", "0"];
    let mut last_error = String::new();
    for seek in attempts {
        let result = Command::new(ffmpeg)
            .current_dir(working_directory)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-ss",
                seek,
            ])
            .arg("-i")
            .arg(media_path)
            .args([
                "-frames:v",
                "1",
                "-vf",
                "scale=512:512:force_original_aspect_ratio=decrease",
                "-q:v",
                "4",
                "-y",
            ])
            .arg(output)
            .output()
            .map_err(|error| format!("无法启动 FFmpeg：{error}"))?;

        if result.status.success() && output.is_file() {
            let (width, height) = image::image_dimensions(output)
                .map_err(|error| format!("视频封面无效：{error}"))?;
            let bytes = output
                .metadata()
                .map(|metadata| i64::try_from(metadata.len()).unwrap_or(i64::MAX))
                .map_err(|error| format!("无法读取视频封面大小：{error}"))?;
            return Ok((width, height, bytes));
        }
        last_error = String::from_utf8_lossy(&result.stderr).trim().to_string();
    }

    Err(if last_error.is_empty() {
        "FFmpeg 未能生成视频封面".to_string()
    } else {
        format!("FFmpeg 生成封面失败：{last_error}")
    })
}

fn find_ffmpeg() -> Option<PathBuf> {
    if let Some(path) = env::var_os("TIME_ALBUM_FFMPEG").map(PathBuf::from) {
        if path.is_file() {
            return Some(path);
        }
    }

    let executable = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    if let Some(path) = env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| env::split_paths(&paths).collect::<Vec<_>>())
        .map(|directory| directory.join(executable))
        .find(|path| path.is_file())
    {
        return Some(path);
    }

    if cfg!(windows) {
        let packages = env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)?
            .join("Microsoft")
            .join("WinGet")
            .join("Packages");
        for package in fs::read_dir(packages).ok()?.flatten() {
            let name = package.file_name().to_string_lossy().to_string();
            if !name.starts_with("Gyan.FFmpeg") {
                continue;
            }
            for build in fs::read_dir(package.path()).ok()?.flatten() {
                let candidate = build.path().join("bin").join("ffmpeg.exe");
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

fn emit_progress(app: Option<&AppHandle>, progress: &ThumbnailProgress) {
    if let Some(app) = app {
        let _ = app.emit("thumbnail-progress", progress);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, Rgb, RgbImage};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_paths(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let unique = format!(
            "time-album-thumbnail-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        );
        let base = env::temp_dir().join(unique);
        let library = base.join("library");
        let cache = base.join("cache");
        fs::create_dir_all(&library).expect("create test library");
        fs::create_dir_all(&cache).expect("create test cache");
        (base, library, cache)
    }

    #[test]
    fn ffmpeg_is_discoverable_when_installed() {
        assert!(find_ffmpeg().is_some());
    }

    #[test]
    fn thumbnail_control_cancels_an_active_job() {
        let control = ThumbnailControl::default();
        let token = control.begin().expect("thumbnail job begins");
        assert!(control.cancel());
        assert!(token.load(Ordering::Relaxed));
        control.finish();
    }

    #[test]
    fn image_thumbnail_does_not_modify_source() {
        let (base, library, cache) = test_paths("image");
        let source = library.join("source.png");
        let output = cache.join("thumb.jpg");
        let image = RgbImage::from_pixel(900, 600, Rgb([60, 110, 170]));
        DynamicImage::ImageRgb8(image)
            .save(&source)
            .expect("save test image");
        let before_bytes = fs::read(&source).expect("read source before");
        let before_modified = fs::metadata(&source)
            .expect("source metadata before")
            .modified()
            .expect("source modified time before");

        let (width, height, bytes) =
            generate_image(&source, &output, &library).expect("generate image thumbnail");
        assert_eq!((width, height), (512, 341));
        assert!(bytes > 0);
        assert_eq!(fs::read(&source).expect("read source after"), before_bytes);
        assert_eq!(
            fs::metadata(&source)
                .expect("source metadata after")
                .modified()
                .expect("source modified time after"),
            before_modified
        );

        fs::remove_dir_all(base).expect("remove test paths");
    }

    #[test]
    fn video_cover_does_not_modify_source() {
        let ffmpeg = find_ffmpeg().expect("ffmpeg is installed");
        let (base, library, cache) = test_paths("video");
        let source = library.join("source.mp4");
        let output = cache.join("cover.jpg");
        let created = Command::new(&ffmpeg)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=640x360:d=2",
                "-c:v",
                "mpeg4",
                "-y",
            ])
            .arg(&source)
            .status()
            .expect("run ffmpeg test fixture");
        assert!(created.success());
        let before_bytes = fs::read(&source).expect("read video before");
        let before_modified = fs::metadata(&source)
            .expect("video metadata before")
            .modified()
            .expect("video modified time before");

        let (width, height, bytes) = generate_video(&ffmpeg, &source, &output, &library, &cache)
            .expect("generate video cover");
        assert_eq!((width, height), (512, 288));
        assert!(bytes > 0);
        assert_eq!(fs::read(&source).expect("read video after"), before_bytes);
        assert_eq!(
            fs::metadata(&source)
                .expect("video metadata after")
                .modified()
                .expect("video modified time after"),
            before_modified
        );

        fs::remove_dir_all(base).expect("remove test paths");
    }

    #[test]
    #[ignore = "requires real-library thumbnail environment variables"]
    fn generates_real_thumbnails_when_explicitly_requested() {
        let library =
            env::var("TIME_ALBUM_REAL_LIBRARY").expect("TIME_ALBUM_REAL_LIBRARY must be set");
        let database = env::var("TIME_ALBUM_REAL_DB").expect("TIME_ALBUM_REAL_DB must be set");
        let cache = env::var("TIME_ALBUM_REAL_THUMBNAIL_CACHE")
            .expect("TIME_ALBUM_REAL_THUMBNAIL_CACHE must be set");
        let report = generate_core(
            None,
            Path::new(&library),
            Path::new(&cache),
            Path::new(&database),
            &AtomicBool::new(false),
            100,
            None,
        )
        .expect("real thumbnail batch succeeds");
        assert_eq!(report.status, "completed");
        assert_eq!(report.processed, 100);
        assert!(report.ready > 0);
        println!("{report:#?}");
    }
}
