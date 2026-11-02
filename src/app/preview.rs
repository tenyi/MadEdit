//! App 內的預覽列印視窗：以獨立原生視窗（egui 多視窗）依 A4 紙張自行分頁繪製列印內容。
//!
//! 實際列印仍交給系統瀏覽器（見 `print.rs`），預覽版面依相同的紙張、邊界與字級估算，
//! 與瀏覽器實際分頁可能略有差異。

use std::ops::Range;
use std::sync::Arc;

use eframe::egui::{self, Color32, FontFamily, FontId, Galley, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use eframe::egui::text::{LayoutJob, TextFormat};

use crate::app::fonts::EDITOR_FAMILY;
use crate::app::form;
use crate::app::i18n::{self, Lang};
use crate::app::print::{PrintContent, SpanKind, PAGE_NO_MARK, PAGE_TOTAL_MARK};
use crate::core::editor::{char_visual_columns, tab_columns_at};

/// A4 紙張寬度（CSS 像素，96 DPI；與 HTML 列印一致）
const PAGE_W: f32 = 793.7;
/// A4 紙張高度
const PAGE_H: f32 = 1122.5;
/// 四周邊界 15mm（與 HTML 的 `@page { margin: 15mm }` 一致）
const MARGIN: f32 = 56.7;
/// 頁首頁尾字級
const HF_FONT: f32 = 10.0;
/// 頁首頁尾與內文的間距
const HF_GAP: f32 = 6.0;
/// 可選的縮放比例
const ZOOMS: &[f32] = &[0.5, 0.75, 1.0, 1.25, 1.5, 2.0];

/// 頁面上的一個區塊
#[derive(Debug, Clone, PartialEq)]
enum Block {
    /// 十六進位位移標頭列
    HexHeader,
    /// 某邏輯行中的連續數個換行後列（行過長時可跨頁）
    Rows { line: usize, rows: Range<usize> },
}

/// 預覽視窗回報給 App 的動作
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PreviewAction {
    /// 維持開啟
    None,
    /// 使用者按下「列印」
    Print,
    /// 關閉預覽視窗
    Close,
}

/// 預覽列印狀態
pub struct PrintPreview {
    /// 預覽中的文件識別碼（列印時據此找回文件）
    pub doc_id: u64,
    /// 文件顯示名稱（視窗標題用）
    pub title: String,
    /// 列印內容快照
    content: PrintContent,
    /// 內文字級（CSS 像素）
    font_size: f32,
    /// 目前縮放比例
    zoom: f32,
    /// 目前顯示的頁（從 0 起算）
    page: usize,
    /// 分頁結果：每頁的區塊清單
    pages: Vec<Vec<Block>>,
    /// 分頁時採用的縮放比例（縮放改變時需重新分頁）
    paged_zoom: Option<f32>,
}

impl PrintPreview {
    pub fn new(doc_id: u64, title: String, content: PrintContent, font_size: f32) -> Self {
        Self { doc_id, title, content, font_size, zoom: 1.0, page: 0, pages: Vec::new(), paged_zoom: None }
    }

    /// 總頁數（尚未分頁時視為 1）
    pub fn page_count(&self) -> usize {
        self.pages.len().max(1)
    }

    /// 頁首區高度（未縮放）
    fn header_h(&self) -> f32 {
        if self.content.header.is_some() { HF_FONT * 1.4 + HF_GAP } else { 0.0 }
    }

    /// 頁尾區高度（未縮放）
    fn footer_h(&self) -> f32 {
        if self.content.footer.is_some() { HF_FONT * 1.4 + HF_GAP } else { 0.0 }
    }

    /// 內文區寬度（未縮放）
    fn body_w() -> f32 {
        PAGE_W - 2.0 * MARGIN
    }

    /// 內文區高度（未縮放）
    fn body_h(&self) -> f32 {
        PAGE_H - 2.0 * MARGIN - self.header_h() - self.footer_h()
    }

    /// 內文字型（已套用縮放）
    fn body_font(&self) -> FontId {
        FontId::new(self.font_size * self.zoom, FontFamily::Name(EDITOR_FAMILY.into()))
    }

    /// 建立某一行的排版工作：Tab 依視覺欄展開為空白，超過內文寬度時任意處換行（同 HTML 的 break-all）
    fn line_job(&self, idx: usize) -> LayoutJob {
        let font = self.body_font();
        let mut job = LayoutJob::default();
        job.wrap.max_width = Self::body_w() * self.zoom;
        job.wrap.break_anywhere = true;
        // 目前所在的視覺欄（用於 Tab 對齊）
        let mut vcol = 0usize;
        for sp in &self.content.lines[idx].spans {
            let color = match sp.kind {
                SpanKind::Text(Some((r, g, b))) => Color32::from_rgb(r, g, b),
                SpanKind::Text(None) => Color32::BLACK,
                SpanKind::LineNumber => Color32::from_rgb(0x99, 0x99, 0x99),
                SpanKind::Bookmark => Color32::from_rgb(0x3c, 0x78, 0xe6),
                SpanKind::Control => Color32::from_rgb(0xaa, 0xaa, 0xcc),
            };
            let mut text = String::with_capacity(sp.text.len());
            for ch in sp.text.chars() {
                if ch == '\t' {
                    let n = tab_columns_at(vcol);
                    text.extend(std::iter::repeat_n(' ', n));
                    vcol += n;
                } else {
                    text.push(ch);
                    vcol += char_visual_columns(ch);
                }
            }
            job.append(&text, 0.0, TextFormat::simple(font.clone(), color));
        }
        if job.sections.is_empty() {
            // 空行也要佔一列高度
            job.append(" ", 0.0, TextFormat::simple(font, Color32::BLACK));
        }
        job
    }

    /// 十六進位位移標頭的排版工作
    fn hex_header_job(&self) -> LayoutJob {
        let text = self.content.hex_header.clone().unwrap_or_default();
        let mut job = LayoutJob::simple_singleline(text, self.body_font(), Color32::from_rgb(0x33, 0x66, 0xcc));
        job.wrap.max_width = Self::body_w() * self.zoom;
        job.wrap.break_anywhere = true;
        job
    }

    /// 依目前縮放重新分頁（以實際排版後的列高計算）
    fn paginate(&mut self, painter: &egui::Painter) {
        let body_h = self.body_h() * self.zoom;
        let hex_h = if self.content.hex_header.is_some() { painter.layout_job(self.hex_header_job()).size().y } else { 0.0 };
        let mut pages: Vec<Vec<Block>> = Vec::new();
        // 目前頁已使用的高度
        let mut y = 0.0;
        // 目前頁是否已有內文列（避免單列高於整頁時無限換頁）
        let mut has_rows = false;
        // 開新頁：依設定在頁頂放置位移標頭
        let new_page = |pages: &mut Vec<Vec<Block>>, y: &mut f32, has_rows: &mut bool| {
            let mut page = Vec::new();
            *y = 0.0;
            if self.content.hex_header.is_some() && (pages.is_empty() || self.content.hex_header_every_page) {
                page.push(Block::HexHeader);
                *y = hex_h;
            }
            pages.push(page);
            *has_rows = false;
        };
        new_page(&mut pages, &mut y, &mut has_rows);
        for idx in 0..self.content.lines.len() {
            let galley = painter.layout_job(self.line_job(idx));
            let n = galley.rows.len();
            let mut r = 0;
            while r < n {
                let start = r;
                let y0 = galley.rows[start].rect().min.y;
                while r < n && y + (galley.rows[r].rect().max.y - y0) <= body_h {
                    r += 1;
                }
                if r == start {
                    if has_rows {
                        // 本頁放不下，換頁後重試
                        new_page(&mut pages, &mut y, &mut has_rows);
                        continue;
                    }
                    // 單列就高於整頁：強制放入
                    r = start + 1;
                }
                y += galley.rows[r - 1].rect().max.y - y0;
                has_rows = true;
                if let Some(page) = pages.last_mut() {
                    page.push(Block::Rows { line: idx, rows: start..r });
                }
                if r < n {
                    new_page(&mut pages, &mut y, &mut has_rows);
                }
            }
        }
        self.pages = pages;
        self.page = self.page.min(self.pages.len().saturating_sub(1));
        self.paged_zoom = Some(self.zoom);
    }

    /// 繪製頁首或頁尾的三欄文字
    fn paint_header_footer(&self, painter: &egui::Painter, cols: &[String; 3], left: f32, right: f32, top: f32) {
        let font = FontId::new(HF_FONT * self.zoom, FontFamily::Proportional);
        let color = Color32::from_rgb(0x55, 0x55, 0x55);
        let page_no = (self.page + 1).to_string();
        let total = self.page_count().to_string();
        for (i, s) in cols.iter().enumerate() {
            let text = s.replace(PAGE_NO_MARK, &page_no).replace(PAGE_TOTAL_MARK, &total);
            if text.is_empty() {
                continue;
            }
            let galley = painter.layout_no_wrap(text, font.clone(), color);
            let x = match i {
                0 => left,
                1 => (left + right - galley.size().x) / 2.0,
                _ => right - galley.size().x,
            };
            painter.galley(Pos2::new(x, top), galley, color);
        }
    }

    /// 繪製目前頁（`rect` 為已縮放的整張紙範圍）
    fn paint_page(&self, painter: &egui::Painter, rect: Rect) {
        let z = self.zoom;
        // 紙張陰影與白底
        painter.rect_filled(rect.translate(Vec2::splat(3.0)), 0.0, Color32::from_black_alpha(60));
        painter.rect_filled(rect, 0.0, Color32::WHITE);
        painter.rect_stroke(rect, 0.0, Stroke::new(1.0, Color32::from_gray(160)), StrokeKind::Outside);

        let left = rect.left() + MARGIN * z;
        let right = rect.right() - MARGIN * z;
        if let Some(cols) = &self.content.header {
            self.paint_header_footer(painter, cols, left, right, rect.top() + MARGIN * z);
        }
        if let Some(cols) = &self.content.footer {
            let top = rect.bottom() - MARGIN * z - HF_FONT * 1.4 * z;
            self.paint_header_footer(painter, cols, left, right, top);
        }

        // 內文
        let mut y = rect.top() + (MARGIN + self.header_h()) * z;
        let Some(blocks) = self.pages.get(self.page) else { return };
        for block in blocks {
            match block {
                Block::HexHeader => {
                    let galley = painter.layout_job(self.hex_header_job());
                    let h = galley.size().y;
                    painter.galley(Pos2::new(left, y), galley, Color32::BLACK);
                    y += h;
                }
                Block::Rows { line, rows } => {
                    // 排版結果由 epaint 快取，逐幀重排成本很低
                    let galley: Arc<Galley> = painter.layout_job(self.line_job(*line));
                    let (Some(first), Some(last)) = (galley.rows.get(rows.start), galley.rows.get(rows.end.saturating_sub(1))) else {
                        continue;
                    };
                    let y0 = first.rect().min.y;
                    let h = last.rect().max.y - y0;
                    // 只顯示本頁負責的列：以裁切範圍遮掉其他列
                    let clip = Rect::from_min_max(Pos2::new(rect.left(), y), Pos2::new(rect.right(), y + h)).intersect(painter.clip_rect());
                    painter.with_clip_rect(clip).galley(Pos2::new(left, y - y0), galley, Color32::BLACK);
                    y += h;
                }
            }
        }
    }

    /// 繪製預覽視窗內容，回傳使用者要求的動作
    pub fn ui(&mut self, ui: &mut egui::Ui, lang: Lang) -> PreviewAction {
        let mut action = PreviewAction::None;
        if ui.ctx().input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape)) {
            return PreviewAction::Close;
        }
        if self.paged_zoom != Some(self.zoom) {
            let painter = ui.painter().clone();
            self.paginate(&painter);
        }
        let total = self.page_count();

        // 鍵盤翻頁
        ui.ctx().input(|i| {
            if i.key_pressed(egui::Key::PageDown) || i.key_pressed(egui::Key::ArrowRight) {
                self.page = (self.page + 1).min(total - 1);
            }
            if i.key_pressed(egui::Key::PageUp) || i.key_pressed(egui::Key::ArrowLeft) {
                self.page = self.page.saturating_sub(1);
            }
            if i.key_pressed(egui::Key::Home) {
                self.page = 0;
            }
            if i.key_pressed(egui::Key::End) {
                self.page = total - 1;
            }
        });

        let t = |key: &str, zh: &'static str| i18n::translate(lang, key, zh);
        let (first, last) = (t("pv.first", "第一頁"), t("pv.last", "最後一頁"));
        egui::Panel::top("print-preview-toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button(t("pv.print", "列印...")).clicked() {
                    action = PreviewAction::Print;
                }
                ui.separator();
                if ui.add_enabled_ui(self.page > 0, |ui| form::icon_button(ui, egui::Button::new("⏮"), first, first)).inner.clicked() {
                    self.page = 0;
                }
                if ui.add_enabled(self.page > 0, egui::Button::new(format!("◀ {}", t("pv.prev", "上一頁")))).clicked() {
                    self.page -= 1;
                }
                ui.label(i18n::fill(t("pv.page", "第 {n} / {q} 頁"), Some(self.page + 1), Some(&total.to_string())));
                if ui.add_enabled(self.page + 1 < total, egui::Button::new(format!("{} ▶", t("pv.next", "下一頁")))).clicked() {
                    self.page += 1;
                }
                if ui.add_enabled_ui(self.page + 1 < total, |ui| form::icon_button(ui, egui::Button::new("⏭"), last, last)).inner.clicked() {
                    self.page = total - 1;
                }
                ui.separator();
                ui.label(t("pv.zoom", "縮放："));
                egui::ComboBox::from_id_salt("print-preview-zoom").selected_text(format!("{:.0}%", self.zoom * 100.0)).show_ui(ui, |ui| {
                    for &z in ZOOMS {
                        ui.selectable_value(&mut self.zoom, z, format!("{:.0}%", z * 100.0));
                    }
                });
                ui.separator();
                if ui.button(t("btn.close", "關閉")).clicked() {
                    action = PreviewAction::Close;
                }
            });
        });

        let desk = if ui.visuals().dark_mode { Color32::from_gray(22) } else { Color32::from_gray(214) };
        egui::CentralPanel::default().frame(egui::Frame::new().fill(desk)).show(ui, |ui| {
            egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
                let page_size = Vec2::new(PAGE_W, PAGE_H) * self.zoom;
                // 四周留白，紙張水平置中
                let pad = 16.0;
                let avail_w = ui.available_width();
                let (rect, _) = ui.allocate_exact_size(Vec2::new(avail_w.max(page_size.x + 2.0 * pad), page_size.y + 2.0 * pad), Sense::hover());
                let page_rect = Rect::from_min_size(Pos2::new(rect.center().x - page_size.x / 2.0, rect.top() + pad), page_size);
                self.paint_page(ui.painter(), page_rect);
            });
        });
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::print::build_content;
    use crate::core::document::Document;
    use crate::core::settings::PrintOptions;

    /// 以無視窗的 egui Context 進行分頁
    fn paginate(text: &str, opts: &PrintOptions) -> PrintPreview {
        let mut doc = Document::default();
        doc.editor.set_text(text);
        let mut pv = PrintPreview::new(doc.id, doc.display_name(), build_content(&doc, opts), 12.0);
        let ctx = egui::Context::default();
        // 測試環境沒有自訂字型，改用內建等寬字型家族名稱註冊 editor 家族
        let mut defs = egui::FontDefinitions::default();
        let mono = defs.families[&FontFamily::Monospace].clone();
        defs.families.insert(FontFamily::Name(EDITOR_FAMILY.into()), mono);
        ctx.set_fonts(defs);
        ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
        ctx.run_ui(egui::RawInput::default(), |ui| {
            let painter = ui.painter().clone();
            pv.paginate(&painter);
        })
        .textures_delta
        .clear();
        pv
    }

    #[test]
    fn test_short_document_is_single_page() {
        let pv = paginate("a\nb\nc", &PrintOptions::default());
        assert_eq!(pv.page_count(), 1);
        assert_eq!(pv.pages[0].len(), 3);
    }

    #[test]
    fn test_long_document_splits_pages_without_losing_lines() {
        let text: Vec<String> = (0..500).map(|i| format!("line {i}")).collect();
        let pv = paginate(&text.join("\n"), &PrintOptions::default());
        assert!(pv.page_count() > 1);
        // 每一行都恰好出現一次（短行不會被拆開）
        let lines: Vec<usize> = pv.pages.iter().flatten().filter_map(|b| if let Block::Rows { line, .. } = b { Some(*line) } else { None }).collect();
        assert_eq!(lines, (0..500).collect::<Vec<_>>());
    }

    #[test]
    fn test_very_long_line_wraps_across_pages() {
        let text = "x".repeat(200_000);
        let pv = paginate(&text, &PrintOptions { line_numbers: false, ..Default::default() });
        assert!(pv.page_count() > 1);
        // 各頁的列範圍首尾相接
        let mut next = 0;
        for b in pv.pages.iter().flatten() {
            if let Block::Rows { rows, .. } = b {
                assert_eq!(rows.start, next);
                next = rows.end;
            }
        }
    }
}
