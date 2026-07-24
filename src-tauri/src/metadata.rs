use std::{ffi::OsStr, io::BufReader, path::Path, sync::OnceLock, time::SystemTime};

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone};
use exif::{In, Reader, Tag, Value};
use regex::Regex;

use crate::safety;

#[derive(Debug, Default)]
pub struct ExtractedMetadata {
    pub captured_at: String,
    pub captured_source: String,
    pub captured_precision: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub error: Option<String>,
}

pub fn extract(
    path: &Path,
    relative_path: &Path,
    library_root: &Path,
    is_photo: bool,
    modified: SystemTime,
) -> ExtractedMetadata {
    let mut result = ExtractedMetadata::default();

    if is_photo {
        match read_exif(path, library_root) {
            Ok(Some(exif)) => {
                if let Some(captured) = exif_datetime(&exif) {
                    result.captured_at = captured;
                    result.captured_source = "exif".to_string();
                    result.captured_precision = "second".to_string();
                }
                let (latitude, longitude) = exif_gps(&exif);
                result.latitude = latitude;
                result.longitude = longitude;
                result.width = exif_dimension(&exif, Tag::PixelXDimension)
                    .or_else(|| exif_dimension(&exif, Tag::ImageWidth));
                result.height = exif_dimension(&exif, Tag::PixelYDimension)
                    .or_else(|| exif_dimension(&exif, Tag::ImageLength));
            }
            Ok(None) => {}
            Err(error) => result.error = Some(error),
        }
    }

    if result.captured_at.is_empty() {
        if let Some((captured, precision)) = filename_datetime(path.file_stem()) {
            result.captured_at = captured;
            result.captured_source = "filename".to_string();
            result.captured_precision = precision.to_string();
        } else if let Some(captured) = folder_datetime(relative_path) {
            result.captured_at = captured;
            result.captured_source = "folder".to_string();
            result.captured_precision = "month".to_string();
        } else {
            let modified: DateTime<Local> = modified.into();
            result.captured_at = modified.to_rfc3339();
            result.captured_source = "modified".to_string();
            result.captured_precision = "second".to_string();
        }
    }

    result
}

fn read_exif(path: &Path, library_root: &Path) -> Result<Option<exif::Exif>, String> {
    let file = safety::open_media_readonly(path, library_root)?;
    let mut reader = BufReader::new(file);
    match Reader::new().read_from_container(&mut reader) {
        Ok(exif) => Ok(Some(exif)),
        Err(exif::Error::NotFound(_)) => Ok(None),
        Err(error) => Err(format!("EXIF 读取失败：{error}")),
    }
}

fn exif_datetime(exif: &exif::Exif) -> Option<String> {
    [Tag::DateTimeOriginal, Tag::DateTimeDigitized, Tag::DateTime]
        .iter()
        .find_map(|tag| {
            exif.get_field(*tag, In::PRIMARY)
                .and_then(|field| ascii_value(&field.value))
                .and_then(|raw| {
                    NaiveDateTime::parse_from_str(raw.trim(), "%Y:%m:%d %H:%M:%S")
                        .ok()
                        .map(|value| value.format("%Y-%m-%dT%H:%M:%S").to_string())
                })
        })
}

fn exif_gps(exif: &exif::Exif) -> (Option<f64>, Option<f64>) {
    let latitude = exif
        .get_field(Tag::GPSLatitude, In::PRIMARY)
        .and_then(|field| dms_value(&field.value));
    let longitude = exif
        .get_field(Tag::GPSLongitude, In::PRIMARY)
        .and_then(|field| dms_value(&field.value));
    let latitude_ref = exif
        .get_field(Tag::GPSLatitudeRef, In::PRIMARY)
        .and_then(|field| ascii_value(&field.value));
    let longitude_ref = exif
        .get_field(Tag::GPSLongitudeRef, In::PRIMARY)
        .and_then(|field| ascii_value(&field.value));

    (
        latitude.map(|value| {
            if latitude_ref
                .as_deref()
                .is_some_and(|reference| reference.eq_ignore_ascii_case("S"))
            {
                -value
            } else {
                value
            }
        }),
        longitude.map(|value| {
            if longitude_ref
                .as_deref()
                .is_some_and(|reference| reference.eq_ignore_ascii_case("W"))
            {
                -value
            } else {
                value
            }
        }),
    )
}

fn dms_value(value: &Value) -> Option<f64> {
    let values = match value {
        Value::Rational(values) if values.len() >= 3 => values,
        _ => return None,
    };
    Some(values[0].to_f64() + values[1].to_f64() / 60.0 + values[2].to_f64() / 3600.0)
}

fn ascii_value(value: &Value) -> Option<String> {
    match value {
        Value::Ascii(values) => values.first().map(|bytes| {
            String::from_utf8_lossy(bytes)
                .trim_end_matches('\0')
                .trim()
                .to_string()
        }),
        _ => None,
    }
}

fn exif_dimension(exif: &exif::Exif, tag: Tag) -> Option<i64> {
    exif.get_field(tag, In::PRIMARY)
        .and_then(|field| field.value.get_uint(0))
        .map(i64::from)
}

fn filename_datetime(stem: Option<&OsStr>) -> Option<(String, &'static str)> {
    let stem = stem?.to_string_lossy();
    static FULL: OnceLock<Regex> = OnceLock::new();
    static DATE: OnceLock<Regex> = OnceLock::new();
    let full = FULL.get_or_init(|| {
        Regex::new(r"(?i)(?:^|[^0-9])(20\d{2})[-_]?(\d{2})[-_]?(\d{2})[-_T]?(\d{2})[-_:]?(\d{2})[-_:]?(\d{2})(?:[^0-9]|$)")
            .expect("valid full date regex")
    });
    let date = DATE.get_or_init(|| {
        Regex::new(r"(?i)(?:^|[^0-9])(20\d{2})[-_]?(\d{2})[-_]?(\d{2})(?:[^0-9]|$)")
            .expect("valid date regex")
    });

    if let Some(captures) = full.captures(&stem) {
        let value = NaiveDate::from_ymd_opt(
            captures[1].parse().ok()?,
            captures[2].parse().ok()?,
            captures[3].parse().ok()?,
        )?
        .and_hms_opt(
            captures[4].parse().ok()?,
            captures[5].parse().ok()?,
            captures[6].parse().ok()?,
        )?;
        return Some((value.format("%Y-%m-%dT%H:%M:%S").to_string(), "second"));
    }

    let captures = date.captures(&stem)?;
    let value = NaiveDate::from_ymd_opt(
        captures[1].parse().ok()?,
        captures[2].parse().ok()?,
        captures[3].parse().ok()?,
    )?
    .and_hms_opt(12, 0, 0)?;
    Some((value.format("%Y-%m-%dT%H:%M:%S").to_string(), "day"))
}

fn folder_datetime(relative_path: &Path) -> Option<String> {
    static YEAR: OnceLock<Regex> = OnceLock::new();
    static MONTH: OnceLock<Regex> = OnceLock::new();
    let year_pattern = YEAR.get_or_init(|| Regex::new(r"^(20\d{2})年$").expect("valid year regex"));
    let month_pattern =
        MONTH.get_or_init(|| Regex::new(r"^(\d{1,2})月$").expect("valid month regex"));

    let mut year = None;
    let mut month = None;
    for component in relative_path.components() {
        let value = component.as_os_str().to_string_lossy();
        if year.is_none() {
            year = year_pattern
                .captures(&value)
                .and_then(|captures| captures[1].parse::<i32>().ok());
        } else if month.is_none() {
            month = month_pattern
                .captures(&value)
                .and_then(|captures| captures[1].parse::<u32>().ok());
        }
    }

    let date = NaiveDate::from_ymd_opt(year?, month.unwrap_or(1), 1)?.and_hms_opt(12, 0, 0)?;
    Local
        .from_local_datetime(&date)
        .single()
        .map(|value| value.to_rfc3339())
        .or_else(|| Some(date.format("%Y-%m-%dT%H:%M:%S").to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_camera_filename() {
        let parsed = filename_datetime(Some(OsStr::new("IMG_20240731_121955")))
            .expect("camera filename parses");
        assert_eq!(parsed.0, "2024-07-31T12:19:55");
        assert_eq!(parsed.1, "second");
    }

    #[test]
    fn parses_screenshot_filename() {
        let parsed = filename_datetime(Some(OsStr::new("Screenshot_2026-01-14-16-30-26-657_app")))
            .expect("screenshot filename parses");
        assert_eq!(parsed.0, "2026-01-14T16:30:26");
    }

    #[test]
    fn ignores_date_like_substrings_inside_long_identifiers() {
        assert!(filename_datetime(Some(OsStr::new("weread_image_22209603086420"))).is_none());
    }

    #[test]
    fn uses_archive_folders_as_month_precision() {
        let parsed =
            folder_datetime(Path::new("2023年/6月/example.jpg")).expect("folder date parses");
        assert!(parsed.starts_with("2023-06-01T12:00:00"));
    }
}
