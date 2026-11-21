//! Windows 檔案系統圖示：向 Shell（SHGetFileInfoW）取得真實系統圖示，
//! 轉成 RGBA 貼圖並以副檔名／路徑為鍵快取，供資源管理器樹與列表繪製。
//!
//! - 資料夾以完整路徑查詢（特殊資料夾會拿到對應的系統圖示）。
//! - 檔案以「副檔名 + SHGFI_USEFILEATTRIBUTES」查詢，不必檔案真的存在；
//!   相同副檔名共用同一張貼圖。
//! - 取 LARGEICON（32px）貼圖、繪製在 16pt 槽位，高 DPI 下依然清晰。
use std::collections::HashMap;

use egui::TextureHandle;
use windows::core::PCWSTR;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
    BITMAPINFOHEADER, DIB_RGB_COLORS,
};
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
use windows::Win32::UI::Shell::{SHGetFileInfoW, SHGFI_FLAGS, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_USEFILEATTRIBUTES, SHFILEINFOW};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, ICONINFO};

/// 非 Windows 平台的空實作（呼叫端回退到向量圖示）。
#[cfg(not(windows))]
#[derive(Default)]
pub(crate) struct IconCache;

#[cfg(not(windows))]
impl IconCache {
    pub(crate) fn get(&mut self, _ctx: &egui::Context, _path: &str, _is_dir: bool) -> Option<TextureHandle> {
        None
    }
}

#[cfg(windows)]

/// 圖示貼圖快取：鍵為 "d|路徑"（資料夾）或 "f|副檔名"（檔案）。
#[cfg(windows)]
#[derive(Default)]
pub(crate) struct IconCache {
    textures: HashMap<String, Option<TextureHandle>>,
}

#[cfg(windows)]
impl IconCache {
    /// 取得某路徑／類型的系統圖示貼圖；取得失敗時回傳 None（呼叫端畫向量備援圖示）。
    pub(crate) fn get(&mut self, ctx: &egui::Context, path: &str, is_dir: bool) -> Option<TextureHandle> {
        let key = if is_dir {
            format!("d|{}", path.to_lowercase())
        } else {
            let ext = std::path::Path::new(path)
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            format!("f|{ext}")
        };
        if let Some(tex) = self.textures.get(&key) {
            return tex.clone();
        }
        let tex = self.load(ctx, &key, path, is_dir);
        self.textures.insert(key, tex.clone());
        tex
    }

    fn load(&mut self, ctx: &egui::Context, key: &str, path: &str, is_dir: bool) -> Option<TextureHandle> {
        unsafe {
            // 檔案用假路徑「C:\icon.<ext>」＋ USEFILEATTRIBUTES：副檔名即可對應系統圖示。
            let query = if is_dir {
                path.to_string()
            } else {
                let ext = std::path::Path::new(path)
                    .extension()
                    .map(|e| e.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if ext.is_empty() { "C:\\icon".to_string() } else { format!(r"C:\icon.{ext}") }
            };
            let mut wide: Vec<u16> = query.encode_utf16().collect();
            wide.push(0);
            let mut info = SHFILEINFOW::default();
            let flags = SHGFI_ICON | SHGFI_LARGEICON | if is_dir { SHGFI_FLAGS(0) } else { SHGFI_USEFILEATTRIBUTES };
            let ok = SHGetFileInfoW(
                PCWSTR(wide.as_ptr()),
                if is_dir { windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(0) } else { FILE_ATTRIBUTE_NORMAL },
                Some(&mut info as *mut _ as _),
                std::mem::size_of::<SHFILEINFOW>() as u32,
                flags,
            );
            if ok == 0 || info.hIcon.is_invalid() {
                return None;
            }
            let pixels = hicon_to_rgba(info.hIcon);
            let _ = DestroyIcon(info.hIcon);
            let (w, h, rgba) = pixels?;
            let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
            Some(ctx.load_texture(format!("shell-icon-{key}"), image, egui::TextureOptions::LINEAR))
        }
    }
}

/// HICON → (寬, 高, RGBA)。透明度：32 位圖示直接用 alpha；若全為 0（舊式圖示）則視為不透明。
#[cfg(windows)]
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn hicon_to_rgba(hicon: windows::Win32::UI::WindowsAndMessaging::HICON) -> Option<(usize, usize, Vec<u8>)> {
    let mut info = ICONINFO::default();
    GetIconInfo(hicon, &mut info).ok()?;
    let mut bm = BITMAP::default();
    let got = GetObjectW(
        info.hbmColor.into(),
        std::mem::size_of::<BITMAP>() as i32,
        Some(&mut bm as *mut _ as *mut _),
    );
    if got == 0 || bm.bmWidth <= 0 || bm.bmHeight <= 0 {
        let _ = DeleteObject(info.hbmColor.into());
        let _ = DeleteObject(info.hbmMask.into());
        return None;
    }
    let (w, h) = (bm.bmWidth as usize, bm.bmHeight as usize);
    let hdc = CreateCompatibleDC(None);
    let mut bmi = BITMAPINFO::default();
    bmi.bmiHeader = BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: bm.bmWidth,
        biHeight: -bm.bmHeight, // 負值 = 由上而下
        biPlanes: 1,
        biBitCount: 32,
        biCompression: DIB_RGB_COLORS.0,
        ..Default::default()
    };
    let mut buf = vec![0u8; w * h * 4];
    GetDIBits(hdc, info.hbmColor.into(), 0, h as u32, Some(buf.as_mut_ptr() as *mut _), &mut bmi, DIB_RGB_COLORS);
    let _ = DeleteDC(hdc);
    let _ = DeleteObject(info.hbmColor.into());
    let _ = DeleteObject(info.hbmMask.into());
    if buf.chunks_exact_mut(4).all(|px| px[3] == 0) {
        for px in buf.chunks_exact_mut(4) {
            px[3] = 255;
        }
    }
    Some((w, h, buf))
}
