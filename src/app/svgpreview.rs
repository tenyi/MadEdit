//! SVG 圖形預覽：以 resvg 點陣化目前 `.svg` 文件並在預覽面板顯示。
//!
//! 自成一體的模組：內容改變才重渲染，滾輪縮放，Esc 關閉；拔掉時刪除本檔、
//! `Tab` 上的 `svg_preview` 欄位與選單／命令各一項即可。

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Vec2};
use std::sync::Arc;

use super::i18n::Lang;

/// 預覽面板回傳的動作
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SvgAction {
    None,
    Close,
}

/// 一次點陣化結果（成功為影像，失敗為訊息）
type Rendered = Result<Arc<egui::ColorImage>, String>;

pub struct SvgPreview {
    /// 停靠位置；與 Markdown 預覽共用同一組 Placement
    pub placement: super::mdpreview::Placement,
    rendered: Option<Rendered>,
    /// The document version rendered for this tab.
    version: u64,
    dark: bool,
    /// 相對於適配大小的縮放（1.0 = 剛好填滿面板）
    zoom: f32,
    /// 剛切換停靠位置時重設為適配大小
    reset_zoom: bool,
    /// 本幀面板範圍（App 判斷 Ctrl+滾輪是否交給預覽）
    pub rect: Rect,
    error: Option<String>,
    /// Shares the font selected for diagram text.
    options: resvg::usvg::Options<'static>,
}

impl SvgPreview {
    pub fn new(font: Option<(String, fontique::Blob<u8>)>) -> Self {
        let options = super::mdpreview::svg_options(font);
        Self {
            placement: super::mdpreview::Placement::Tab,
            rendered: None,
            version: 0,
            dark: false,
            zoom: 1.0,
            reset_zoom: true,
            rect: Rect::NOTHING,
            error: None,
            options,
        }
    }

    pub fn docked(&self) -> bool {
        self.placement != super::mdpreview::Placement::Tab
    }

    pub fn set_placement(&mut self, placement: super::mdpreview::Placement) {
        if self.placement != placement {
            self.placement = placement;
            self.reset_zoom = true;
        }
    }

    pub fn refresh(&mut self, ctx: &egui::Context, text: impl FnOnce() -> String, version: u64, dark: bool) {
        if self.rendered.is_some() && self.version == version && self.dark == dark {
            return;
        }
        self.version = version;
        self.dark = dark;
        self.error = None;
        // 點陣化尺寸不得超過 GPU 紋理上限（多數裝置 2048/8192）：取 SVG 寬高比，
        // 長邊貼著上限縮放。之後的滾輪縮放只是顯示大小，不重渲染。
        let max_side = ctx.input(|i| i.max_texture_side).min(4096) as f32;
        let rendered = resvg::usvg::Tree::from_data(text().as_bytes(), &self.options)
            .map_err(|e| format!("{e}"))
            .and_then(|svg| {
                let size = svg.size();
                let longest = size.width().max(size.height()).max(1.0);
                let scale = (max_side / longest).min(1.0);
                let w = ((size.width() * scale).ceil() as u32).max(1).min(max_side as u32);
                let h = ((size.height() * scale).ceil() as u32).max(1).min(max_side as u32);
                let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h).ok_or_else(|| "invalid size".to_string())?;
                resvg::render(&svg, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
                let pixels = pixmap.take();
                Ok(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &pixels))
            });
        self.error = rendered.as_ref().err().cloned();
        self.rendered = Some(rendered.map(Arc::new));
        ctx.request_repaint();
    }

    /// 預覽面板；回傳 Esc 等關閉要求
    pub fn ui(&mut self, ui: &mut egui::Ui, lang: Lang) -> SvgAction {
        self.rect = ui.max_rect();
        if !self.docked() && ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
            return SvgAction::Close;
        }
        let mut close = false;
        let title = crate::app::i18n::translate(lang, "md.preview_title", "預覽");
        egui::Panel::top("svg-preview-toolbar").show(ui, |ui| {
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
            return SvgAction::Close;
        }
        let rest = ui.available_rect_before_wrap();
        let Some(rendered) = &self.rendered else {
            ui.painter().rect_filled(rest, 0, ui.visuals().extreme_bg_color);
            return SvgAction::None;
        };
        match rendered {
            Ok(image) => {
                let handle = ctx_tex(ui, image);
                // 適配面板（留邊），再乘上使用者縮放
                let size = Vec2::new(image.width() as f32, image.height() as f32);
                let fit = (rest.width() / size.x).min(rest.height() / size.y).min(1.0);
                let scale = fit.max(0.01) * self.zoom.max(0.05);
                let shown = size * scale;
                let center = rest.center();
                let rect = Rect::from_center_size(center, shown);
                let painter = ui.painter_at(rest);
                painter.rect_filled(rest, 0, ui.visuals().extreme_bg_color);
                painter.image(handle, rect, egui::Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            }
            Err(message) => {
                ui.painter().rect_filled(rest, 0, ui.visuals().extreme_bg_color);
                let text = crate::app::i18n::translate(lang, "svg.bad_file", "無法解析此 SVG 檔案").to_string();
                ui.painter().text(
                    rest.center(),
                    Align2::CENTER_CENTER,
                    format!("{text}\n{message}"),
                    FontId::monospace(13.0),
                    ui.visuals().error_fg_color,
                );
            }
        }
        // 滾輪縮放
        let zoom_delta = ui.ctx().input(|i| i.zoom_delta());
        if zoom_delta != 1.0 && ui.rect_contains_pointer(rest) {
            self.zoom = (self.zoom * zoom_delta).clamp(0.1, 8.0);
            ui.ctx().request_repaint();
        }
        SvgAction::None
    }
}

/// 取得（並快取）點陣化結果的紋理：同一 Id 的內容更新時 egui 自動上傳新影像
fn ctx_tex(ui: &egui::Ui, image: &Arc<egui::ColorImage>) -> egui::TextureId {
    ui.ctx()
        .load_texture_cached("svg-preview", egui::Id::unique(("svg-preview-tex", ())), Arc::clone(image), egui::TextureOptions::LINEAR)
        .id()
}

/// 以 resvg 點陣化 SVG（含字型庫選項）；測試與除錯用。主路徑在 `refresh`（含紋理上限）。
#[cfg(test)]
fn render_svg_with(bytes: &[u8], options: &resvg::usvg::Options<'static>) -> Result<egui::ColorImage, String> {
    let svg = resvg::usvg::Tree::from_data(bytes, options).map_err(|e| e.to_string())?;
    let size = svg.size();
    let cap = 4096.0_f32;
    let scale = (cap / size.width().max(size.height())).min(1.0).max(0.01);
    let w = ((size.width() * scale).ceil() as u32).max(1).min(cap as u32);
    let h = ((size.height() * scale).ceil() as u32).max(1).min(cap as u32);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h).ok_or_else(|| "invalid size".to_string())?;
    resvg::render(&svg, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    let pixels = pixmap.take();
    Ok(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &pixels))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><rect width="80" height="40" fill="#2266aa"/></svg>"##;
    const BAD: &str = "this is not svg";

    #[test]
    fn test_render_svg_success_and_failure() {
        let opts = resvg::usvg::Options::default();
        let img = render_svg_with(GOOD.as_bytes(), &opts).expect("valid svg renders");
        assert_eq!((img.width(), img.height()), (80, 40));
        let err = render_svg_with(BAD.as_bytes(), &opts).expect_err("invalid svg fails");
        assert!(!err.is_empty());
    }

    #[test]
    fn test_refresh_skips_unchanged_content() {
        let ctx = egui::Context::default();
        let mut preview = SvgPreview::new(None);
        preview.refresh(&ctx, || GOOD.into(), 1, false);
        let first = preview.rendered.clone();
        preview.refresh(&ctx, || panic!("unchanged document was read"), 1, false);
        assert!(preview.rendered.is_some());
        assert!(preview.error.is_none());
        // 相同內容：rendered 沿用（still Some），不會變成 None
        assert_eq!(preview.rendered, first);
        preview.refresh(&ctx, || BAD.into(), 2, false);
        assert!(preview.error.is_some(), "changed content re-renders and reports the error");
    }

    #[test]
    fn preview_without_a_fallback_font_does_not_scan_the_system() {
        assert!(SvgPreview::new(None).options.fontdb.is_empty());
    }
}
