// Windows 發行版不顯示主控台視窗
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod core;
mod single_instance;
#[cfg(test)]
mod test_support;
#[cfg(target_os = "macos")]
mod macos;

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;

fn main() -> eframe::Result {
    #[cfg(target_os = "macos")]
    macos::install();
    // 命令列：MadEdit [檔案...] [--screenshot <輸出.png>]（截圖供自動化測試使用）
    let mut files = Vec::new();
    let mut screenshot = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(a) = args.next() {
        if a == "--screenshot" {
            screenshot = args.next().map(PathBuf::from);
        } else {
            files.push(PathBuf::from(a));
        }
    }

    // 截圖測試獨立執行，避免把測試檔案送進使用者正在編輯的視窗。
    let instance = if screenshot.is_none() {
        let directory = core::settings::Settings::path()
            .and_then(|p| p.parent().map(PathBuf::from))
            .ok_or_else(|| eframe::Error::AppCreation(Box::new(std::io::Error::other("找不到 MadEdit 設定目錄"))))?;
        match single_instance::Instance::start(&directory, files.clone())
            .map_err(|e| eframe::Error::AppCreation(Box::new(e)))? {
            Some(instance) => Some(instance),
            None => return Ok(()),
        }
    } else {
        None
    };

    let size = core::settings::Settings::load().window_size.unwrap_or((1200.0, 780.0));
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("MadEdit")
        .with_inner_size([size.0.max(400.0), size.1.max(300.0)])
        .with_min_inner_size([400.0, 300.0])
        .with_drag_and_drop(true);
    // Windows：去掉系統標題欄，由 egui 繪製（圖示 + 選單列 + 視窗按鈕），
    // 縮放邊框與拖動由 window_subclass（src/app/titlebar.rs）以 WM_NCHITTEST 補回。
    #[cfg(windows)]
    {
        viewport = viewport.with_decorations(false);
    }
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../assets/madedit-icon.png")) {
        viewport = viewport.with_icon(icon);
    }
    let mut options = eframe::NativeOptions { viewport, ..Default::default() };
    // egui 預設向顯示卡要求固定的資源上限（OpenGL 後端用 WebGL2 標準值），
    // 舊版 Mesa 軟體繪製（如 Ubuntu 20.04 的 llvmpipe）達不到時會直接啟動失敗。
    // 這裡把要求值壓到顯示卡實際支援的範圍內；編輯器只畫 2D 介面，用不到那麼多。
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_options.wgpu_setup {
        let base = setup.device_descriptor.clone();
        setup.device_descriptor = Arc::new(move |adapter| {
            let mut desc = base(adapter);
            desc.required_limits = desc.required_limits.or_worse_values_from(&adapter.limits());
            desc
        });
    }
    eframe::run_native("MadEdit", options, Box::new(|cc| {
        let mut app = app::App::new(cc, files, screenshot);
        if let Some(instance) = instance {
            instance.attach_context(&cc.egui_ctx);
            app.instance = Some(instance);
        }
        Ok(Box::new(app))
    }))
}

#[cfg(test)]
mod tests {
    /// 內嵌的視窗圖示（原版調色盤格式 PNG）必須能被 eframe 解碼
    #[test]
    fn test_window_icon_decodes() {
        let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/madedit-icon.png")).unwrap();
        assert_eq!((icon.width, icon.height), (48, 48));
    }
}
