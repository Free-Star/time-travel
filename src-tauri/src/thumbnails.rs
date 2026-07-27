use std::{
    collections::hash_map::DefaultHasher,
    env, fs,
    hash::{Hash, Hasher},
    io::{BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
};

use chrono::Local;
use image::{codecs::jpeg::JpegEncoder, ImageReader};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::{
    database::{self, ThumbnailPreview, ThumbnailStatus},
    safety,
};

// Timeline/map cards never render near 512 physical pixels. A 320px cache cuts
// decode, encode, disk and WebView upload cost while the viewer still opens the
// original media for full-resolution inspection.
const THUMBNAIL_EDGE: u32 = 320;

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

    pub fn try_begin(&self) -> Result<Option<Arc<AtomicBool>>, String> {
        let mut guard = self
            .active_cancel
            .lock()
            .map_err(|_| "缩略图任务状态锁已损坏".to_string())?;
        if guard.is_some() {
            return Ok(None);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *guard = Some(cancel.clone());
        Ok(Some(cancel))
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

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailResult {
    pub media_id: i64,
    pub status: String,
    pub cache_path: Option<String>,
    pub generator: Option<String>,
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

#[derive(Debug)]
struct GeneratedThumbnail {
    width: u32,
    height: u32,
    bytes: i64,
    generator: &'static str,
}

#[derive(Debug)]
struct CompletedCandidate {
    candidate: database::ThumbnailCandidate,
    output: PathBuf,
    generated: Result<GeneratedThumbnail, String>,
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
        false,
    )
}

pub fn generate_selected(
    app: &AppHandle,
    library_root: &Path,
    cancel: &AtomicBool,
    media_ids: &[i64],
    retry_failed: bool,
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
        retry_failed,
    )
}

#[allow(clippy::too_many_arguments)]
fn generate_core(
    app: Option<&AppHandle>,
    library_root: &Path,
    cache: &Path,
    database_path: &Path,
    cancel: &AtomicBool,
    limit: usize,
    media_ids: Option<&[i64]>,
    retry_failed: bool,
) -> Result<ThumbnailReport, String> {
    let root = safety::canonical_existing(library_root)?;
    safety::create_directory_outside_library(cache, &root)?;
    let connection = database::open_at(database_path, &root)?;
    let candidates = if let Some(media_ids) = media_ids {
        database::thumbnail_candidates_by_ids(&connection, &root, media_ids, retry_failed)?
    } else {
        database::thumbnail_candidates(&connection, &root, limit.clamp(1, 1000))?
    };
    let ffmpeg = find_ffmpeg();
    let mut progress = ThumbnailProgress {
        status: "generating".to_string(),
        total: candidates.len(),
        ..ThumbnailProgress::default()
    };

    let mut photo_batches: Vec<Vec<database::ThumbnailCandidate>> =
        (0..3).map(|_| Vec::new()).collect();
    let mut video_batch = Vec::new();
    let mut photo_index = 0usize;
    for candidate in candidates {
        if candidate.media_kind == "photo" {
            let batch_index = photo_index % 3;
            photo_batches[batch_index].push(candidate);
            photo_index += 1;
        } else {
            video_batch.push(candidate);
        }
    }

    thread::scope(|scope| -> Result<(), String> {
        let (sender, receiver) = mpsc::channel::<CompletedCandidate>();
        for batch in photo_batches.into_iter().filter(|batch| !batch.is_empty()) {
            let sender = sender.clone();
            let root = root.as_path();
            let ffmpeg = ffmpeg.as_deref();
            scope.spawn(move || {
                for candidate in batch {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let completed = process_candidate(candidate, root, cache, ffmpeg);
                    if sender.send(completed).is_err() {
                        break;
                    }
                }
            });
        }
        if !video_batch.is_empty() {
            let sender = sender.clone();
            let root = root.as_path();
            let ffmpeg = ffmpeg.as_deref();
            scope.spawn(move || {
                for candidate in video_batch {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let completed = process_candidate(candidate, root, cache, ffmpeg);
                    if sender.send(completed).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sender);

        for completed in receiver {
            record_completed(&connection, app, &mut progress, completed)?;
        }
        Ok(())
    })?;

    progress.status = if cancel.load(Ordering::Relaxed) {
        "cancelled".to_string()
    } else {
        "completed".to_string()
    };
    progress.current_path.clear();
    emit_progress(app, &progress);
    report_from_connection(&connection, &root, progress)
}

fn process_candidate(
    candidate: database::ThumbnailCandidate,
    root: &Path,
    cache: &Path,
    ffmpeg: Option<&Path>,
) -> CompletedCandidate {
    let output_directory = cache.join(format!("{:02x}", candidate.media_id.rem_euclid(256)));
    let output = output_directory.join(format!(
        "{}-{}.jpg",
        candidate.media_id, candidate.modified_ns
    ));
    let generated = safety::create_directory_outside_library(&output_directory, root)
        .and_then(|_| generate_media_thumbnail(&candidate, &output, root, cache, ffmpeg));
    CompletedCandidate {
        candidate,
        output,
        generated,
    }
}

fn record_completed(
    connection: &rusqlite::Connection,
    app: Option<&AppHandle>,
    progress: &mut ThumbnailProgress,
    completed: CompletedCandidate,
) -> Result<(), String> {
    progress.current_path = completed.candidate.path.clone();
    match completed.generated {
        Ok(generated) => {
            database::record_thumbnail(
                connection,
                completed.candidate.media_id,
                completed.candidate.modified_ns,
                Some(&completed.output),
                Some(generated.width),
                Some(generated.height),
                generated.bytes,
                "ready",
                None,
                &Local::now().to_rfc3339(),
            )?;
            progress.ready += 1;
            emit_result(
                app,
                &ThumbnailResult {
                    media_id: completed.candidate.media_id,
                    status: "ready".to_string(),
                    cache_path: Some(completed.output.to_string_lossy().to_string()),
                    generator: Some(generated.generator.to_string()),
                },
            );
        }
        Err(error) => {
            database::record_thumbnail(
                connection,
                completed.candidate.media_id,
                completed.candidate.modified_ns,
                None,
                None,
                None,
                0,
                "failed",
                Some(&error),
                &Local::now().to_rfc3339(),
            )?;
            progress.failed += 1;
            emit_result(
                app,
                &ThumbnailResult {
                    media_id: completed.candidate.media_id,
                    status: "failed".to_string(),
                    cache_path: None,
                    generator: None,
                },
            );
        }
    }
    progress.processed += 1;
    emit_progress(app, progress);
    Ok(())
}

fn generate_media_thumbnail(
    candidate: &database::ThumbnailCandidate,
    output: &Path,
    root: &Path,
    cache: &Path,
    ffmpeg: Option<&Path>,
) -> Result<GeneratedThumbnail, String> {
    let media_path = Path::new(&candidate.path);
    drop(safety::open_media_readonly(media_path, root)?);

    #[cfg(target_os = "windows")]
    if let Ok(image) = crate::windows_thumbnail::load(media_path, THUMBNAIL_EDGE, true) {
        return encode_thumbnail(image, output, root, "windows-cache");
    }

    if candidate.media_kind == "photo" {
        if matches!(
            media_path
                .extension()
                .and_then(|value| value.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref(),
            Some("jpg" | "jpeg")
        ) {
            if let Ok(image) = load_embedded_jpeg_thumbnail(media_path, root) {
                if image.width().max(image.height()) >= 240 {
                    return encode_thumbnail(image, output, root, "embedded-jpeg");
                }
            }
        }

        #[cfg(target_os = "windows")]
        if let Ok(image) = crate::windows_thumbnail::load(media_path, THUMBNAIL_EDGE, false) {
            return encode_thumbnail(image, output, root, "windows-shell");
        }

        generate_image(media_path, output, root)
    } else {
        #[cfg(target_os = "windows")]
        if let Ok(image) = crate::windows_thumbnail::load(media_path, THUMBNAIL_EDGE, false) {
            return encode_thumbnail(image, output, root, "windows-shell");
        }

        ffmpeg
            .ok_or_else(|| "未找到 FFmpeg，无法生成视频封面".to_string())
            .and_then(|path| generate_video(path, media_path, output, root, cache))
    }
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
) -> Result<GeneratedThumbnail, String> {
    let input = safety::open_media_readonly(media_path, root)?;
    let reader = ImageReader::new(BufReader::new(input))
        .with_guessed_format()
        .map_err(|error| format!("无法识别图片格式：{error}"))?;
    let image = reader
        .decode()
        .map_err(|error| format!("无法解码图片：{error}"))?;
    encode_thumbnail(image, output, root, "image-decoder")
}

fn encode_thumbnail(
    image: image::DynamicImage,
    output: &Path,
    root: &Path,
    generator: &'static str,
) -> Result<GeneratedThumbnail, String> {
    let thumbnail = image.thumbnail(THUMBNAIL_EDGE, THUMBNAIL_EDGE);
    let width = thumbnail.width();
    let height = thumbnail.height();
    let output_file = safety::create_file_outside_library(output, root)?;
    JpegEncoder::new_with_quality(output_file, 76)
        .encode_image(&thumbnail)
        .map_err(|error| format!("无法编码缩略图：{error}"))?;
    let bytes = output
        .metadata()
        .map(|metadata| i64::try_from(metadata.len()).unwrap_or(i64::MAX))
        .map_err(|error| format!("无法读取缩略图大小：{error}"))?;
    Ok(GeneratedThumbnail {
        width,
        height,
        bytes,
        generator,
    })
}

fn generate_video(
    ffmpeg: &Path,
    media_path: &Path,
    output: &Path,
    root: &Path,
    working_directory: &Path,
) -> Result<GeneratedThumbnail, String> {
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
                "scale=320:320:force_original_aspect_ratio=decrease",
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
            return Ok(GeneratedThumbnail {
                width,
                height,
                bytes,
                generator: "ffmpeg",
            });
        }
        last_error = String::from_utf8_lossy(&result.stderr).trim().to_string();
    }

    Err(if last_error.is_empty() {
        "FFmpeg 未能生成视频封面".to_string()
    } else {
        format!("FFmpeg 生成封面失败：{last_error}")
    })
}

fn load_embedded_jpeg_thumbnail(
    media_path: &Path,
    root: &Path,
) -> Result<image::DynamicImage, String> {
    let input = safety::open_media_readonly(media_path, root)?;
    let jpeg = extract_embedded_jpeg(BufReader::new(input))?;
    image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg)
        .map_err(|error| format!("无法解码内嵌 JPEG 预览：{error}"))
}

fn extract_embedded_jpeg<R: Read>(mut reader: R) -> Result<Vec<u8>, String> {
    let mut signature = [0u8; 2];
    reader
        .read_exact(&mut signature)
        .map_err(|error| format!("无法读取 JPEG 文件头：{error}"))?;
    if signature != [0xff, 0xd8] {
        return Err("图片不是 JPEG 格式".to_string());
    }

    loop {
        let mut marker_prefix = [0u8; 1];
        reader
            .read_exact(&mut marker_prefix)
            .map_err(|error| format!("无法读取 JPEG 标记：{error}"))?;
        while marker_prefix[0] != 0xff {
            reader
                .read_exact(&mut marker_prefix)
                .map_err(|error| format!("无法定位 JPEG 标记：{error}"))?;
        }
        let mut marker = [0u8; 1];
        reader
            .read_exact(&mut marker)
            .map_err(|error| format!("无法读取 JPEG 标记类型：{error}"))?;
        while marker[0] == 0xff {
            reader
                .read_exact(&mut marker)
                .map_err(|error| format!("无法读取 JPEG 填充标记：{error}"))?;
        }
        if matches!(marker[0], 0xd9 | 0xda) {
            break;
        }
        if marker[0] == 0x01 || (0xd0..=0xd7).contains(&marker[0]) {
            continue;
        }

        let mut length_bytes = [0u8; 2];
        reader
            .read_exact(&mut length_bytes)
            .map_err(|error| format!("无法读取 JPEG 区段长度：{error}"))?;
        let length = u16::from_be_bytes(length_bytes) as usize;
        if length < 2 {
            return Err("JPEG 区段长度无效".to_string());
        }
        let mut segment = vec![0u8; length - 2];
        reader
            .read_exact(&mut segment)
            .map_err(|error| format!("无法读取 JPEG 区段：{error}"))?;
        if marker[0] == 0xe1 && segment.starts_with(b"Exif\0\0") {
            if let Some(thumbnail) = exif_jpeg_from_tiff(&segment[6..]) {
                return Ok(thumbnail.to_vec());
            }
        }
    }
    Err("JPEG 不包含可用的内嵌预览".to_string())
}

fn exif_jpeg_from_tiff(tiff: &[u8]) -> Option<&[u8]> {
    if tiff.len() < 8 {
        return None;
    }
    let little_endian = match &tiff[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let read_u16 = |offset: usize| -> Option<u16> {
        let bytes: [u8; 2] = tiff.get(offset..offset + 2)?.try_into().ok()?;
        Some(if little_endian {
            u16::from_le_bytes(bytes)
        } else {
            u16::from_be_bytes(bytes)
        })
    };
    let read_u32 = |offset: usize| -> Option<u32> {
        let bytes: [u8; 4] = tiff.get(offset..offset + 4)?.try_into().ok()?;
        Some(if little_endian {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        })
    };
    if read_u16(2)? != 42 {
        return None;
    }

    let ifd0 = read_u32(4)? as usize;
    let ifd0_entries = read_u16(ifd0)? as usize;
    let ifd1_pointer = ifd0
        .checked_add(2)?
        .checked_add(ifd0_entries.checked_mul(12)?)?;
    let ifd1 = read_u32(ifd1_pointer)? as usize;
    if ifd1 == 0 {
        return None;
    }

    let entries = read_u16(ifd1)? as usize;
    let mut jpeg_offset = None;
    let mut jpeg_length = None;
    for index in 0..entries {
        let entry = ifd1.checked_add(2)?.checked_add(index.checked_mul(12)?)?;
        let tag = read_u16(entry)?;
        let field_type = read_u16(entry + 2)?;
        let count = read_u32(entry + 4)?;
        if field_type != 4 || count != 1 {
            continue;
        }
        match tag {
            0x0201 => jpeg_offset = Some(read_u32(entry + 8)? as usize),
            0x0202 => jpeg_length = Some(read_u32(entry + 8)? as usize),
            _ => {}
        }
    }

    let start = jpeg_offset?;
    let end = start.checked_add(jpeg_length?)?;
    let jpeg = tiff.get(start..end)?;
    jpeg.starts_with(&[0xff, 0xd8]).then_some(jpeg)
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

fn emit_result(app: Option<&AppHandle>, result: &ThumbnailResult) {
    if let Some(app) = app {
        let _ = app.emit("thumbnail-result", result);
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
        assert!(control
            .try_begin()
            .expect("busy thumbnail state remains readable")
            .is_none());
        assert!(control.cancel());
        assert!(token.load(Ordering::Relaxed));
        control.finish();
        assert!(control
            .try_begin()
            .expect("automatic thumbnail job begins after finish")
            .is_some());
        control.finish();
    }

    #[test]
    fn reads_embedded_jpeg_from_exif_ifd() {
        let thumbnail = [0xff, 0xd8, 0xff, 0xd9];
        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"II");
        tiff.extend_from_slice(&42u16.to_le_bytes());
        tiff.extend_from_slice(&8u32.to_le_bytes());
        tiff.extend_from_slice(&0u16.to_le_bytes());
        tiff.extend_from_slice(&14u32.to_le_bytes());
        tiff.extend_from_slice(&2u16.to_le_bytes());
        tiff.extend_from_slice(&0x0201u16.to_le_bytes());
        tiff.extend_from_slice(&4u16.to_le_bytes());
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&44u32.to_le_bytes());
        tiff.extend_from_slice(&0x0202u16.to_le_bytes());
        tiff.extend_from_slice(&4u16.to_le_bytes());
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&(thumbnail.len() as u32).to_le_bytes());
        tiff.extend_from_slice(&0u32.to_le_bytes());
        tiff.extend_from_slice(&thumbnail);

        assert_eq!(exif_jpeg_from_tiff(&tiff), Some(thumbnail.as_slice()));
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

        let generated =
            generate_image(&source, &output, &library).expect("generate image thumbnail");
        assert_eq!((generated.width, generated.height), (320, 213));
        assert!(generated.bytes > 0);
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

        let generated = generate_video(&ffmpeg, &source, &output, &library, &cache)
            .expect("generate video cover");
        assert_eq!((generated.width, generated.height), (320, 180));
        assert!(generated.bytes > 0);
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
            false,
        )
        .expect("real thumbnail batch succeeds");
        assert_eq!(report.status, "completed");
        assert_eq!(report.processed, 100);
        assert!(report.ready > 0);
        println!("{report:#?}");
    }
}
