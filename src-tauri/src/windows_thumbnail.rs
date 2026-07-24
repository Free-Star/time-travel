use std::{ffi::c_void, mem::size_of, path::Path};

use image::{DynamicImage, RgbImage};
use windows::{
    core::HSTRING,
    Win32::{
        Foundation::SIZE,
        Graphics::Gdi::{
            DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO,
            BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
        },
        System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED},
        UI::Shell::{
            IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK,
            SIIGBF_INCACHEONLY, SIIGBF_THUMBNAILONLY,
        },
    },
};

struct ComApartment;

impl ComApartment {
    fn initialize() -> Result<Self, String> {
        let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        result
            .ok()
            .map(|_| Self)
            .map_err(|error| format!("无法初始化 Windows 缩略图线程：{error}"))
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

struct OwnedBitmap(HBITMAP);

impl Drop for OwnedBitmap {
    fn drop(&mut self) {
        if !self.0 .0.is_null() {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(self.0 .0));
            }
        }
    }
}

pub fn load(path: &Path, edge: u32, cache_only: bool) -> Result<DynamicImage, String> {
    let _apartment = ComApartment::initialize()?;
    let path = path.to_string_lossy();
    let shell_path = path.strip_prefix(r"\\?\").unwrap_or(&path);
    let wide_path = HSTRING::from(shell_path);
    let factory: IShellItemImageFactory = unsafe { SHCreateItemFromParsingName(&wide_path, None) }
        .map_err(|error| format!("Windows 无法识别媒体路径：{error}"))?;

    let mut flags = SIIGBF_THUMBNAILONLY | SIIGBF_BIGGERSIZEOK;
    if cache_only {
        flags |= SIIGBF_INCACHEONLY;
    }
    let bitmap = unsafe {
        factory.GetImage(
            SIZE {
                cx: edge as i32,
                cy: edge as i32,
            },
            flags,
        )
    }
    .map(OwnedBitmap)
    .map_err(|error| format!("Windows 缩略图不可用：{error}"))?;

    bitmap_to_image(bitmap.0)
}

fn bitmap_to_image(handle: HBITMAP) -> Result<DynamicImage, String> {
    let mut bitmap = BITMAP::default();
    let object_bytes = unsafe {
        GetObjectW(
            HGDIOBJ(handle.0),
            size_of::<BITMAP>() as i32,
            Some((&mut bitmap as *mut BITMAP).cast::<c_void>()),
        )
    };
    if object_bytes == 0 || bitmap.bmWidth <= 0 || bitmap.bmHeight == 0 {
        return Err("Windows 返回了无效的缩略图位图".to_string());
    }

    let width = bitmap.bmWidth as u32;
    let height = bitmap.bmHeight.unsigned_abs();
    let pixel_count = width
        .checked_mul(height)
        .and_then(|value| value.checked_mul(4))
        .ok_or_else(|| "Windows 缩略图尺寸过大".to_string())?;
    let mut bgra = vec![0u8; pixel_count as usize];
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            biHeight: -(height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..BITMAPINFOHEADER::default()
        },
        ..BITMAPINFO::default()
    };

    let device_context = unsafe { GetDC(None) };
    if device_context.0.is_null() {
        return Err("无法读取 Windows 缩略图像素".to_string());
    }
    let copied = unsafe {
        GetDIBits(
            device_context,
            handle,
            0,
            height,
            Some(bgra.as_mut_ptr().cast::<c_void>()),
            &mut info,
            DIB_RGB_COLORS,
        )
    };
    unsafe {
        let _ = ReleaseDC(None, device_context);
    }
    if copied != height as i32 {
        return Err("Windows 缩略图像素读取不完整".to_string());
    }

    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    for pixel in bgra.chunks_exact(4) {
        rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
    }
    RgbImage::from_raw(width, height, rgb)
        .map(DynamicImage::ImageRgb8)
        .ok_or_else(|| "Windows 缩略图像素格式无效".to_string())
}
