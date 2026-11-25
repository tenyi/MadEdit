//! 點陣圖片預覽：把 `.bmp`／`.jpg`／`.jpeg`（以及 image crate 支援的 png/gif/webp）
//! 從磁碟解碼後顯示在預覽分頁或並排面板。
//!
//! 圖片在 MadEdit 以十六進位模式開啟，文件文字不是可靠的圖片來源，因此預覽直接讀取
//! 磁碟上的位元組；以「長度 + 修改時間」當版本戳，只有檔案變動才重新解碼。
//! 超過 GPU 紋理上限的巨型圖先縮到上限再顯示（之後的縮放只動顯示大小）。

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Vec2};
use std::path::Path;
use std::sync::Arc;

use super::i18n::Lang;

/// 預覽面板回傳的動作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageAction {
    None,
    Close,
}

/// 一次解碼結果（成功為影像，失敗為訊息）。
type Rendered = Result<Arc<egui::ColorImage>, String>;

pub struct ImagePreview {
    /// 停靠位置；與 Markdown 預覽共用同一組 `Placement`
    pub placement: super::mdpreview::Placement,
    rendered: Option<Rendered>,
    /// 檔案版本戳（長度 + 修改時間），檔案沒變就沿上次解碼結果
    stamp: u64,
    /// 相對於適配大小的縮放；1.0 = 剛好填滿面板
    zoom: f32,
    /// 切換停靠位置時重設縮放
    reset_zoom: bool,
    /// 本面板矩形（App 判斷 Ctrl+滾輪是否交給預覽）
    pub rect: Rect,
    error: Option<String>,
}

impl Default for ImagePreview {
    fn default() -> Self {
        Self {
            placement: super::mdpreview::Placement::Tab,
            rendered: None,
            stamp: u64::MAX,
            zoom: 1.0,
            reset_zoom: true,
            rect: Rect::NOTHING,
            error: None,
        }
    }
}

impl ImagePreview {
    pub fn docked(&self) -> bool {
        self.placement != super::mdpreview::Placement::Tab
    }

    pub fn set_placement(&mut self, placement: super::mdpreview::Placement) {
        if self.placement != placement {
            self.placement = placement;
            self.reset_zoom = true;
        }
    }

    /// 讀檔並解碼；`stamp` 不同才重讀（檔案簽名：長度 + 修改時間）。
    pub fn refresh(&mut self, ctx: &egui::Context, path: &Path, stamp: u64) {
        if self.rendered.is_some() && self.stamp == stamp {
            return;
        }
        self.stamp = stamp;
        self.error = None;
        let max_side = ctx.input(|i| i.max_texture_side).min(4096) as u32;
        let rendered = std::fs::read(path)
            .map_err(|e| format!("{e}"))
            .and_then(|bytes| {
                let img = image::load_from_memory(&bytes).map_err(|e| format!("{e}"))?;
                // 超過紋理上限先等比縮小（只做一次；縮放按鈕只改顯示大小）
                let img = if img.width() > max_side || img.height() > max_side {
                    let scale = max_side as f32 / img.width().max(img.height()) as f32;
                    let w = ((img.width() as f32 * scale).round() as u32).max(1);
                    let h = ((img.height() as f32 * scale).round() as u32).max(1);
                    image::DynamicImage::ImageRgba8(image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle))
                } else {
                    img
                };
                let rgba = img.to_rgba8();
                Ok(egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()))
            });
        self.error = rendered.as_ref().err().cloned();
        self.rendered = Some(rendered.map(Arc::new));
        ctx.request_repaint();
    }

    /// 預覽面板；回傳 Esc 等關閉要求
    pub fn ui(&mut self, ui: &mut egui::Ui, lang: Lang) -> ImageAction {
        self.rect = ui.max_rect();
        if !self.docked() && ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
            return ImageAction::Close;
        }
        let mut close = false;
        let title = crate::app::i18n::translate(lang, "md.preview_title", "預覽");
        egui::Panel::top("img-preview-toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(title).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    for (label, zoom, key, zh) in [
                        ("1:1", 0.0_f32, "md.zoom_reset", "還原 100%"),
                        ("＋", 0.2, "md.zoom_in", "放大（Ctrl+滾輪）"),
                        ("－", -0.2, "md.zoom_out", "縮小（Ctrl+滾輪）"),
                    ] {
                        let name = super::i18n::translate(lang, key, zh);
                        if super::form::icon_button(ui, egui::Button::new(label), name, name).clicked() {
                            self.zoom = if zoom == 0.0 { 1.0 } else { (self.zoom + zoom).clamp(0.1, 8.0) };
                        }
                    }
                    let close_label = crate::app::i18n::translate(lang, "btn.close", "關閉");
                    if ui.button(close_label).clicked() {
                        close = true;
                    }
                });
            });
        });
        if close {
            return ImageAction::Close;
        }
        let rest = ui.available_rect_before_wrap();
        let Some(rendered) = &self.rendered else {
            ui.painter().rect_filled(rest, 0, ui.visuals().extreme_bg_color);
            return ImageAction::None;
        };
        match rendered {
            Ok(image) => {
                let handle = ctx_tex(ui, image);
                let size = Vec2::new(image.width() as f32, image.height() as f32);
                let fit = (rest.width() / size.x).min(rest.height() / size.y).min(1.0);
                let scale = fit.max(0.01) * self.zoom.max(0.05);
                let shown = size * scale;
                let rect = Rect::from_center_size(rest.center(), shown);
                let painter = ui.painter_at(rest);
                painter.rect_filled(rest, 0, ui.visuals().extreme_bg_color);
                painter.image(handle, rect, egui::Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            }
            Err(message) => {
                ui.painter().rect_filled(rest, 0, ui.visuals().extreme_bg_color);
                let text = crate::app::i18n::translate(lang, "img.bad_file", "無法解碼此圖片").to_string();
                ui.painter().text(
                    rest.center(),
                    Align2::CENTER_CENTER,
                    format!("{text}\n{message}"),
                    FontId::monospace(13.0),
                    ui.visuals().error_fg_color,
                );
            }
        }
        let zoom_delta = ui.ctx().input(|i| i.zoom_delta());
        if zoom_delta != 1.0 && ui.rect_contains_pointer(rest) {
            self.zoom = (self.zoom * zoom_delta).clamp(0.1, 8.0);
            ui.ctx().request_repaint();
        }
        ImageAction::None
    }
}

/// 取得（並快取）解碼結果的紋理：同一 Id 的內容更新時 egui 自動上傳新影像
fn ctx_tex(ui: &egui::Ui, image: &Arc<egui::ColorImage>) -> egui::TextureId {
    ui.ctx()
        .load_texture_cached("img-preview", egui::Id::unique(("img-preview-tex", ())), Arc::clone(image), egui::TextureOptions::LINEAR)
        .id()
}

#[cfg(test)]
mod tests {
    /// 以內嵌的最小 PNG（1×1 紅色像素）驗證解碼路徑。
    const TINY_PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00,
        0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49,
        0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D, 0xB0,
        0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn decodes_png_bytes_and_reports_bad_data() {
        let img = image::load_from_memory(TINY_PNG).expect("內嵌 PNG 必須能解碼");
        assert_eq!((img.width(), img.height()), (1, 1));
        assert!(image::load_from_memory(b"not an image").is_err());
    }
}
