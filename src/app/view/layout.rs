//! Character widths, wrapping, cached line layout and coordinate conversion.

use super::{
    CharWidths, Frame, HorizontalBar, Layout, LayoutEntry, LineLayout, LineWrap, ViewState,
    WrapKey, line_xs,
};
use crate::app::fonts::EDITOR_FAMILY;
use crate::core::content_hash;
use crate::core::document::Document;
use crate::core::editor::{EditorState, TextPosition, char_visual_columns, tab_size};
use crate::core::model::EditMode;
use crate::core::settings::{Settings, WrapMode};
use eframe::egui::{self, FontFamily, FontId, Pos2, Rect, Vec2};
use std::collections::HashSet;

impl CharWidths {
    /// 確認字型設定並量測尚未快取的字元
    pub fn prepare(
        &mut self,
        ctx: &egui::Context,
        font: &FontId,
        family: &str,
        chars: impl Iterator<Item = char>,
    ) {
        let key = (font.size.to_bits(), family.to_string(), self.generation);
        if key != self.key {
            self.key = key;
            self.map.get_mut().clear();
            self.cell_w = 0.0;
        }
        self.measure = Some((ctx.clone(), font.clone()));
        let map = self.map.get_mut();
        let missing: HashSet<char> = chars.filter(|c| !map.contains_key(c)).collect();
        if missing.is_empty() && self.cell_w > 0.0 {
            return;
        }
        ctx.fonts_mut(|f| {
            if self.cell_w <= 0.0 {
                self.cell_w = f.glyph_width(font, '0').max(1.0);
                self.space_w = f.glyph_width(font, ' ').max(1.0);
                self.text_h = f.row_height(font);
            }
            for c in missing {
                let w = if c == '\t' {
                    0.0
                } else {
                    f.glyph_width(font, c)
                };
                map.insert(c, w);
            }
        });
    }

    /// Forgets every measured width. Call it once the new fonts are active (the frame after
    /// `Context::set_fonts`): widths measured in the frame of the change came from the old font.
    pub fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    pub fn get(&self, c: char) -> f32 {
        if let Some(w) = self.map.borrow().get(&c) {
            return *w;
        }
        let Some((ctx, font)) = &self.measure else {
            return self.cell_w * char_visual_columns(c) as f32;
        };
        // Measure now instead of guessing: the guess (cell width) is what made proportional
        // fonts overlap in a freshly opened file until the layout caches were cleared.
        let w = if c == '\t' {
            0.0
        } else {
            ctx.fonts_mut(|f| f.glyph_width(font, c))
        };
        self.map.borrow_mut().insert(c, w);
        w
    }
}

/// 水平可見的字元範圍 `[起, 迄)`，左右各留邊界以減少捲動時重解析
///
/// `xs` 是整行的每字元 x 座標（單調遞增），因此可用二分搜尋取得視窗邊界。
pub(super) fn visible_chars(
    xs: &[f32],
    row_start: usize,
    row_end: usize,
    scroll_x: f32,
    width: f32,
    rtl: bool,
) -> (usize, usize) {
    const MARGIN: f32 = 64.0;
    // 右到左模式下行首貼齊右緣，可見範圍對應的 x 區間會鏡射
    let (left, right) = if rtl {
        (scroll_x - width - MARGIN, scroll_x + MARGIN)
    } else {
        (scroll_x - MARGIN, scroll_x + width + MARGIN)
    };
    let last = row_end.min(xs.len().saturating_sub(1));
    let first = row_start.min(last);
    // 視覺列以列首字元為 x 原點（自動換行時同一行的每一列都從 0 起算）
    let base = xs[first];
    let lo = xs.partition_point(|x| *x < left + base).clamp(first, last);
    let hi = xs.partition_point(|x| *x <= right + base).clamp(lo, last);
    (lo, hi)
}
/// 依寬度切分一行（盡量在空白後斷行），回傳每個視覺列的起始字元
fn wrap_line(xs: &[f32], chars: &[char], max_w: f32) -> Vec<usize> {
    let mut starts = vec![0usize];
    let n = chars.len();
    let mut row_start = 0usize;
    let mut last_space: Option<usize> = None;
    let mut i = 0;
    while i < n {
        if xs[i + 1] - xs[row_start] > max_w && i > row_start {
            let brk = match last_space {
                Some(s) if s > row_start && s <= i => s,
                _ => i,
            };
            starts.push(brk);
            row_start = brk;
            last_space = None;
            i = brk;
            continue;
        }
        if chars[i] == ' ' || chars[i] == '\t' {
            last_space = Some(i + 1);
        }
        i += 1;
    }
    starts
}
impl Layout {
    pub fn new(settings: &Settings, mode: EditMode) -> Self {
        let column = mode == EditMode::Column;
        Self {
            font: FontId::new(settings.font_size, FontFamily::Name(EDITOR_FAMILY.into())),
            // 列塊模式需要字元格網，強制等寬
            fixed: settings.fixed_width || column,
            wrap: if column {
                WrapMode::NoWrap
            } else {
                settings.wrap_mode
            },
            wrap_width: 0.0,
            row_h: 0.0,
            rtl: settings.right_to_left,
        }
    }
}

impl ViewState {
    /// 重建換行快取。設定、寬度或字型改變時整份重算；內容改變時只重算雜湊不同的行
    pub(super) fn update_wrap(&mut self, doc: &Document, lay: &Layout, widths: &CharWidths) {
        if lay.wrap == WrapMode::NoWrap {
            self.wrap.key = None;
            return;
        }
        let key = WrapKey {
            doc_id: doc.id,
            width_bits: lay.wrap_width.to_bits(),
            fixed: lay.fixed,
            font: widths.key.clone(),
            tab: tab_size(),
        };
        if self.wrap.key.as_ref() != Some(&key) {
            self.wrap.per_line.clear();
            self.wrap.checked_version = None;
            self.wrap.key = Some(key);
        }
        let lines = &doc.editor.lines;
        if self.wrap.per_line.len() != lines.len() {
            self.wrap.per_line.resize(lines.len(), None);
            self.wrap.checked_version = None;
        }
        if self.wrap.checked_version == Some(doc.version) {
            return;
        }
        let max_w = lay.wrap_width.max(widths.cell_w * 4.0);
        for (slot, line) in self.wrap.per_line.iter_mut().zip(lines.iter()) {
            let hash = content_hash(line);
            if slot.as_ref().is_some_and(|w| w.hash == hash) {
                continue;
            }
            let chars: Vec<char> = line.chars().collect();
            let xs = line_xs(line, lay.fixed, widths);
            // wrap_line 的第一個起始是 0，breaks 只存後續斷行點（串接時補回行首 0）
            let breaks = wrap_line(&xs, &chars, max_w)
                .into_iter()
                .skip(1)
                .map(|i| i as u32)
                .collect();
            *slot = Some(LineWrap { hash, breaks });
        }
        self.wrap.checked_version = Some(doc.version);
        // 串接各行的視覺列索引
        self.wrap.rows.clear();
        self.wrap.line_first_row.clear();
        for (li, w) in self.wrap.per_line.iter().enumerate() {
            self.wrap.line_first_row.push(self.wrap.rows.len() as u32);
            let Some(w) = w else { continue };
            self.wrap.rows.push((li as u32, 0));
            for &b in &w.breaks {
                self.wrap.rows.push((li as u32, b));
            }
        }
    }

    /// 一行在自動換行時的斷行位置（字元索引，不含 0）；未自動換行時為空
    pub fn wrap_breaks(&self, line: usize) -> Vec<usize> {
        if !self.wrapped() {
            return Vec::new();
        }
        let Some(&first) = self.wrap.line_first_row.get(line) else {
            return Vec::new();
        };
        self.wrap.rows[first as usize + 1..]
            .iter()
            .take_while(|(l, _)| *l as usize == line)
            .map(|(_, s)| *s as usize)
            .collect()
    }

    pub(super) fn wrapped(&self) -> bool {
        self.wrap.key.is_some()
    }

    pub fn row_count(&self, ed: &EditorState) -> usize {
        if self.wrapped() {
            self.wrap.rows.len()
        } else {
            ed.lines.len()
        }
    }

    /// 視覺列 → (行號, 起始字元, 結束字元)
    pub(super) fn row_info(&self, ed: &EditorState, row: usize) -> (usize, usize, usize) {
        if !self.wrapped() {
            return (row, 0, ed.line_char_count(row));
        }
        let (line, start) = self.wrap.rows[row];
        let end = match self.wrap.rows.get(row + 1) {
            Some(&(l, s)) if l == line => s as usize,
            _ => ed.line_char_count(line as usize),
        };
        (line as usize, start as usize, end)
    }

    /// 目前最上方可見的邏輯行與行內進度（行號從 0 起算；進度依自動換行的視覺列計算）。
    pub fn top_line(&self, ed: &EditorState) -> (usize, f32) {
        let rows = self.row_count(ed);
        if rows == 0 {
            return (0, 0.0);
        }
        let row = self.top_row.min(rows - 1);
        let line = self.row_info(ed, row).0;
        if !self.wrapped() {
            return (line, 0.0);
        }
        // 同一個邏輯行可佔多個視覺列；傳遞其進度，預覽才不會在長行內停住。
        let first = self.wrap.line_first_row[line] as usize;
        let end = self
            .wrap
            .line_first_row
            .get(line + 1)
            .map_or(rows, |r| *r as usize);
        (line, (row - first) as f32 / (end - first).max(1) as f32)
    }

    /// 預覽反向同步只改變可視範圍，保留游標與選取；換行時取最接近的視覺列。
    pub fn scroll_to_source(&mut self, ed: &EditorState, line: usize, fraction: f32) {
        let line = line.min(ed.lines.len().saturating_sub(1));
        self.top_row = if self.wrapped() {
            let first = self.wrap.line_first_row[line] as usize;
            let end = self
                .wrap
                .line_first_row
                .get(line + 1)
                .map_or(self.row_count(ed), |r| *r as usize);
            (first + (fraction.clamp(0.0, 1.0) * (end - first) as f32).round() as usize)
                .min(end.saturating_sub(1))
        } else {
            line
        };
    }

    /// 文字位置所在的視覺列
    pub(super) fn row_of(&self, pos: TextPosition) -> usize {
        if !self.wrapped() {
            return pos.line;
        }
        let Some(&first) = self.wrap.line_first_row.get(pos.line) else {
            return self.wrap.rows.len().saturating_sub(1);
        };
        let mut r = first as usize;
        while r + 1 < self.wrap.rows.len()
            && self.wrap.rows[r + 1].0 as usize == pos.line
            && self.wrap.rows[r + 1].1 as usize <= pos.col
        {
            r += 1;
        }
        r
    }

    /// 每幀開始：丟掉上一幀沒用到的行版面，更新文件版本與版面參數
    ///
    /// 超長行的版面（字元陣列 + 每字元 x 座標）建一次就要數毫秒，因此跨幀沿用：
    /// 只有真的被改到的行（內容雜湊不同）才重建。
    pub fn begin_frame(&mut self, version: u64, fixed: bool, widths: &CharWidths) {
        let mut frame = self.frame.borrow_mut();
        frame.lines.retain(|_, e| e.used);
        for e in frame.lines.values_mut() {
            e.used = false;
        }
        let params = (fixed, widths.key.clone(), tab_size());
        if frame.params.as_ref() != Some(&params) {
            frame.lines.clear();
            frame.params = Some(params);
        }
        frame.version = version;
    }

    /// 取得一行本幀的版面資料（字元陣列與每字元 x 座標），同一幀內共用
    pub(super) fn line_layout(
        &self,
        ed: &EditorState,
        line: usize,
        fixed: bool,
        widths: &CharWidths,
    ) -> LineLayout {
        let mut frame = self.frame.borrow_mut();
        let version = frame.version;
        if let Some(e) = frame.lines.get_mut(&line) {
            // 版本未變＝內容沒被編輯；版本變了則以內容雜湊確認這一行是否真的改了
            if e.version == version || e.hash == content_hash(&ed.lines[line]) {
                e.version = version;
                e.used = true;
                return std::rc::Rc::clone(&e.layout);
            }
        }
        let hash = content_hash(&ed.lines[line]);
        let layout: LineLayout = std::rc::Rc::new((
            ed.lines[line].chars().collect(),
            line_xs(&ed.lines[line], fixed, widths),
        ));
        frame.lines.insert(
            line,
            LayoutEntry {
                version,
                hash,
                layout: std::rc::Rc::clone(&layout),
                used: true,
            },
        );
        layout
    }

    /// 清除行版面快取（內容編輯後必須呼叫）
    pub fn clear_frame_layout(&self) {
        self.frame.borrow_mut().lines.clear();
    }

    /// 視覺列所屬的邏輯行
    pub(super) fn row_line(&self, ed: &EditorState, row: usize) -> usize {
        if !self.wrapped() {
            row.min(ed.lines.len().saturating_sub(1))
        } else {
            self.wrap.rows[row].0 as usize
        }
    }

    /// 視覺列在行內的起始字元
    pub(super) fn row_start(&self, _ed: &EditorState, row: usize) -> usize {
        if !self.wrapped() {
            0
        } else {
            self.wrap.rows[row].1 as usize
        }
    }

    /// 某視覺列在指定 x（相對列首）的字元位置
    pub(super) fn pos_at_x(
        &self,
        ed: &EditorState,
        row: usize,
        x: f32,
        lay: &Layout,
        widths: &CharWidths,
    ) -> TextPosition {
        let row = row.min(self.row_count(ed).saturating_sub(1));
        let line = self.row_line(ed, row);
        let start = self.row_start(ed, row);
        let end = match self.wrap.rows.get(row + 1) {
            Some(&(l, s)) if self.wrapped() && l as usize == line => s as usize,
            _ => ed.line_char_count(line),
        };
        let xs = &self.line_layout(ed, line, lay.fixed, widths).1;
        let x0 = xs[start];
        for i in start..end {
            if x < (xs[i] + xs[i + 1]) / 2.0 - x0 {
                return TextPosition::new(line, i);
            }
        }
        TextPosition::new(line, end)
    }

    /// 游標位置的 x（相對所在視覺列的列首）
    pub(super) fn x_of(
        &self,
        ed: &EditorState,
        pos: TextPosition,
        lay: &Layout,
        widths: &CharWidths,
    ) -> f32 {
        let row = self.row_of(pos);
        let line = self.row_line(ed, row);
        let start = self.row_start(ed, row);
        let xs = &self.line_layout(ed, line, lay.fixed, widths).1;
        xs[pos.col.min(xs.len() - 1)] - xs[start]
    }

    /// 捲動使游標可見（打字機模式則讓游標保持在中央）
    pub(super) fn scroll_to_caret(
        &mut self,
        ed: &EditorState,
        lay: &Layout,
        widths: &CharWidths,
        text_w: f32,
        typewriter: bool,
    ) {
        let head = ed.selection.head;
        let row = self.row_of(head);
        let vis = self.visible_rows.max(1);
        if typewriter {
            self.top_row = row.saturating_sub(vis / 2);
        } else if row < self.top_row {
            self.top_row = row;
        } else if row >= self.top_row + vis {
            self.top_row = row + 1 - vis;
        }
        if self.wrapped() {
            self.scroll_x = 0.0;
            return;
        }
        let x = if ed.selection.is_block() {
            ed.selection
                .head_vcol
                .map(|v| v as f32 * widths.cell_w)
                .unwrap_or_else(|| self.x_of(ed, head, lay, widths))
        } else {
            self.x_of(ed, head, lay, widths)
        };
        let margin = widths.cell_w * 4.0;
        if x < self.scroll_x + margin {
            self.scroll_x = (x - margin).max(0.0);
        } else if x > self.scroll_x + text_w - margin {
            self.scroll_x = x - text_w + margin;
        }
    }
}

pub(super) fn horizontal_bar(view: &ViewState, frame: &Frame) -> Option<HorizontalBar> {
    if view.wrap.key.is_some() {
        return None;
    }
    let text_rect = frame.text_rect;
    let total_w = view.content_width.max(view.scroll_x + text_rect.width());
    let track = frame.hbar.shrink2(Vec2::new(2.0, 2.0));
    let thumb_w = (track.width() * text_rect.width() / total_w).clamp(20.0, track.width());
    let max_scroll = (total_w - text_rect.width()).max(0.0);
    let frac = if max_scroll <= 0.0 {
        0.0
    } else {
        view.scroll_x / max_scroll
    };
    let thumb = Rect::from_min_size(
        Pos2::new(track.left() + (track.width() - thumb_w) * frac, track.top()),
        Vec2::new(thumb_w, track.height()),
    );
    Some(HorizontalBar {
        track,
        thumb,
        max_scroll,
    })
}
