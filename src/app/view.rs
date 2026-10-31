//! 自繪文字編輯區：排版（等寬格網或比例字寬、自動換行）、繪製、滑鼠鍵盤與輸入法
//!
//! 文字內容與游標的唯一真值在 `Document.editor`；本模組只保存捲動位置、換行快取等檢視狀態。

use std::collections::{HashMap, HashSet};

use eframe::egui::{
    self, Align2, Color32, CornerRadius, Event, FontFamily, FontId, Key, Modifiers, PointerButton, Pos2, Rect,
    Sense, Stroke, StrokeKind, Vec2,
};
use serde::{Deserialize, Serialize};

use super::{i18n, menus};
use crate::app::commands::C;
use crate::app::fonts::EDITOR_FAMILY;
use crate::core::content_hash;
use crate::core::document::{Document, EditKind};
use crate::core::editor::{
    char_col_to_visual_col, char_visual_columns, tab_columns_at, tab_size, visual_col_to_char_col,
    EditorState, Selection, TextPosition,
};
use crate::core::model::EditMode;
use crate::core::settings::{Settings, WrapMode};
use crate::core::spell::SpellChecker;
use crate::core::syntax::{self, HighlightSpan};
use crate::core::textops;

// ── 字寬快取 ────────────────────────────────────────────────────

/// Width cache key: (font size bits, family, font generation). The generation is bumped by
/// [`CharWidths::invalidate`] when the font data itself is replaced under the same name.
type WidthKey = (u32, String, u32);

/// 編輯區字型的逐字字寬快取（字型或字級改變時清空）
#[derive(Default)]
pub struct CharWidths {
    key: WidthKey,
    /// Interior mutability so that `get` can measure a character it has not seen yet:
    /// layout code only holds `&CharWidths`, and a width guessed from the cell size would be
    /// cached in the line layout and wrap caches (keyed by line content) and never corrected.
    map: std::cell::RefCell<HashMap<char, f32>>,
    /// Context and font used by `get` to measure on demand; `None` in unit tests that fill
    /// the widths by hand, where unknown characters fall back to the cell grid.
    measure: Option<(egui::Context, FontId)>,
    /// Bumped by `invalidate`; part of the key so every cache keyed on it is rebuilt.
    generation: u32,
    pub cell_w: f32,
    pub space_w: f32,
    pub text_h: f32,
}

impl CharWidths {
    /// 確認字型設定並量測尚未快取的字元
    pub fn prepare(&mut self, ctx: &egui::Context, font: &FontId, family: &str, chars: impl Iterator<Item = char>) {
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
                let w = if c == '\t' { 0.0 } else { f.glyph_width(font, c) };
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
        let w = if c == '\t' { 0.0 } else { ctx.fonts_mut(|f| f.glyph_width(font, c)) };
        self.map.borrow_mut().insert(c, w);
        w
    }
}

/// 一行文字每個字元的起始 x（相對行首），長度為字元數 + 1（最後一項為行寬）
pub fn line_xs(line: &str, fixed: bool, widths: &CharWidths) -> Vec<f32> {
    let mut xs = Vec::with_capacity(line.len() + 1);
    let mut x = 0.0f32;
    let mut vcol = 0usize;
    for c in line.chars() {
        xs.push(x);
        if fixed {
            vcol += if c == '\t' { tab_columns_at(vcol) } else { char_visual_columns(c) };
            x = vcol as f32 * widths.cell_w;
        } else if c == '\t' {
            let tab_w = tab_size() as f32 * widths.space_w;
            x = ((x / tab_w).floor() + 1.0) * tab_w;
        } else {
            x += widths.get(c);
        }
    }
    xs.push(x);
    xs
}

/// 水平可見的字元範圍 `[起, 迄)`，左右各留邊界以減少捲動時重解析
///
/// `xs` 是整行的每字元 x 座標（單調遞增），因此可用二分搜尋取得視窗邊界。
fn visible_chars(xs: &[f32], row_start: usize, row_end: usize, scroll_x: f32, width: f32, rtl: bool) -> (usize, usize) {
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

// ── 自動換行快取 ────────────────────────────────────────────────

#[derive(Clone, PartialEq)]
struct WrapKey {
    doc_id: u64,
    width_bits: u32,
    fixed: bool,
    font: WidthKey,
    tab: usize,
}

/// 單一邏輯行的換行結果（以內容雜湊判斷是否需要重算）
#[derive(Clone)]
struct LineWrap {
    hash: u64,
    /// 換行位置（字元索引，不含 0）
    breaks: Vec<u32>,
}

/// 自動換行後的視覺列：(行號, 起始字元)
#[derive(Default)]
pub struct WrapCache {
    key: Option<WrapKey>,
    /// 已完成各行雜湊檢查的文件版本；版本相同時直接沿用快取
    checked_version: Option<u64>,
    /// 與 `doc.editor.lines` 對齊的各行換行結果；編輯時只重算內容變動的行
    per_line: Vec<Option<LineWrap>>,
    rows: Vec<(u32, u32)>,
    line_first_row: Vec<u32>,
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

// ── 檢視狀態 ────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    Select,
    Block,
    Lines { anchor_line: usize },
    VScroll { grab: f32 },
    HScroll { grab: f32 },
}

/// 右鍵選單中的拼字建議
struct SpellContext {
    line: usize,
    start: usize,
    end: usize,
    word: String,
    suggestions: Vec<String>,
}

/// 對應括號快取的鍵與值
type BraceCache = ((TextPosition, u64), Option<(TextPosition, TextPosition)>);

/// 一行在本幀共用的版面資料：字元陣列與每字元起始 x
type LineLayout = std::rc::Rc<(Vec<char>, Vec<f32>)>;

/// 每幀各邏輯行的版面快取；超長行只量測一次，繪製與滑鼠／游標計算共用
#[derive(Default)]
struct FrameLayout {
    /// 本幀的文件版本（由 [`ViewState::begin_frame`] 設定）
    version: u64,
    /// 版面參數（等寬格網、字型、縮排）：改變時整份清空
    params: Option<(bool, WidthKey, usize)>,
    lines: HashMap<usize, LayoutEntry>,
}

/// 一行本幀的版面，附內容版本與雜湊以便跨幀沿用
struct LayoutEntry {
    version: u64,
    hash: u64,
    layout: LineLayout,
    /// 本幀是否用到；上一幀沒用到就丟棄（記憶體只保留看得到的行）
    used: bool,
}

pub struct ViewState {
    /// 最上方顯示的視覺列
    pub top_row: usize,
    pub scroll_x: f32,
    last_caret: Option<(TextPosition, u64, Option<usize>)>,
    wrap: WrapCache,
    /// 輸入法組字中的文字
    pub preedit: Option<String>,
    drag: Option<Drag>,
    /// 已知最寬的行寬（水平捲軸範圍）
    content_width: f32,
    /// 上下移動時維持的水平位置
    preferred_x: Option<f32>,
    /// 雙擊標示的字詞
    pub highlight_word: Option<String>,
    /// 可見列數（翻頁用）
    pub visible_rows: usize,
    /// 強制捲動到游標（例如搜尋後）
    pub ensure_visible: bool,
    /// 對應括號快取：(游標位置, 文件版本) → 括號配對
    brace_cache: Option<BraceCache>,
    spell_cache: HashMap<u64, Vec<(usize, usize)>>,
    spell_ctx: Option<SpellContext>,
    /// 本幀各行版面快取（繪製、滑鼠與游標計算共用）
    frame: std::cell::RefCell<FrameLayout>,
    pub text_rect: Rect,
    blink_start: f64,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            top_row: 0,
            scroll_x: 0.0,
            last_caret: None,
            wrap: WrapCache::default(),
            preedit: None,
            drag: None,
            content_width: 0.0,
            preferred_x: None,
            highlight_word: None,
            visible_rows: 30,
            ensure_visible: true,
            brace_cache: None,
            spell_cache: HashMap::new(),
            spell_ctx: None,
            frame: std::cell::RefCell::default(),
            text_rect: Rect::NOTHING,
            blink_start: 0.0,
        }
    }
}

/// 版面參數（每幀依設定計算）
pub struct Layout {
    pub font: FontId,
    pub fixed: bool,
    pub wrap: WrapMode,
    pub wrap_width: f32,
    pub row_h: f32,
    pub rtl: bool,
}

impl Layout {
    pub fn new(settings: &Settings, mode: EditMode) -> Self {
        let column = mode == EditMode::Column;
        Self {
            font: FontId::new(settings.font_size, FontFamily::Name(EDITOR_FAMILY.into())),
            // 列塊模式需要字元格網，強制等寬
            fixed: settings.fixed_width || column,
            wrap: if column { WrapMode::NoWrap } else { settings.wrap_mode },
            wrap_width: 0.0,
            row_h: 0.0,
            rtl: settings.right_to_left,
        }
    }
}

impl ViewState {
    /// 重建換行快取。設定、寬度或字型改變時整份重算；內容改變時只重算雜湊不同的行
    fn update_wrap(&mut self, doc: &Document, lay: &Layout, widths: &CharWidths) {
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
        let Some(&first) = self.wrap.line_first_row.get(line) else { return Vec::new() };
        self.wrap.rows[first as usize + 1..]
            .iter()
            .take_while(|(l, _)| *l as usize == line)
            .map(|(_, s)| *s as usize)
            .collect()
    }

    fn wrapped(&self) -> bool {
        self.wrap.key.is_some()
    }

    pub fn row_count(&self, ed: &EditorState) -> usize {
        if self.wrapped() { self.wrap.rows.len() } else { ed.lines.len() }
    }

    /// 視覺列 → (行號, 起始字元, 結束字元)
    fn row_info(&self, ed: &EditorState, row: usize) -> (usize, usize, usize) {
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
        if !self.wrapped() { return (line, 0.0); }
        // 同一個邏輯行可佔多個視覺列；傳遞其進度，預覽才不會在長行內停住。
        let first = self.wrap.line_first_row[line] as usize;
        let end = self.wrap.line_first_row.get(line + 1).map_or(rows, |r| *r as usize);
        (line, (row - first) as f32 / (end - first).max(1) as f32)
    }

    /// 預覽反向同步只改變可視範圍，保留游標與選取；換行時取最接近的視覺列。
    pub fn scroll_to_source(&mut self, ed: &EditorState, line: usize, fraction: f32) {
        let line = line.min(ed.lines.len().saturating_sub(1));
        self.top_row = if self.wrapped() {
            let first = self.wrap.line_first_row[line] as usize;
            let end = self.wrap.line_first_row.get(line + 1).map_or(self.row_count(ed), |r| *r as usize);
            (first + (fraction.clamp(0.0, 1.0) * (end - first) as f32).round() as usize).min(end.saturating_sub(1))
        } else { line };
    }

    /// 文字位置所在的視覺列
    fn row_of(&self, pos: TextPosition) -> usize {
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
    fn line_layout(&self, ed: &EditorState, line: usize, fixed: bool, widths: &CharWidths) -> LineLayout {
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
        let layout: LineLayout = std::rc::Rc::new((ed.lines[line].chars().collect(), line_xs(&ed.lines[line], fixed, widths)));
        frame.lines.insert(line, LayoutEntry { version, hash, layout: std::rc::Rc::clone(&layout), used: true });
        layout
    }

    /// 清除行版面快取（內容編輯後必須呼叫）
    pub fn clear_frame_layout(&self) {
        self.frame.borrow_mut().lines.clear();
    }

    /// 視覺列所屬的邏輯行
    fn row_line(&self, ed: &EditorState, row: usize) -> usize {
        if !self.wrapped() {
            row.min(ed.lines.len().saturating_sub(1))
        } else {
            self.wrap.rows[row].0 as usize
        }
    }

    /// 視覺列在行內的起始字元
    fn row_start(&self, _ed: &EditorState, row: usize) -> usize {
        if !self.wrapped() { 0 } else { self.wrap.rows[row].1 as usize }
    }

    /// 某視覺列在指定 x（相對列首）的字元位置
    fn pos_at_x(&self, ed: &EditorState, row: usize, x: f32, lay: &Layout, widths: &CharWidths) -> TextPosition {
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
    fn x_of(&self, ed: &EditorState, pos: TextPosition, lay: &Layout, widths: &CharWidths) -> f32 {
        let row = self.row_of(pos);
        let line = self.row_line(ed, row);
        let start = self.row_start(ed, row);
        let xs = &self.line_layout(ed, line, lay.fixed, widths).1;
        xs[pos.col.min(xs.len() - 1)] - xs[start]
    }

    /// 捲動使游標可見（打字機模式則讓游標保持在中央）
    fn scroll_to_caret(&mut self, ed: &EditorState, lay: &Layout, widths: &CharWidths, text_w: f32, typewriter: bool) {
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
            ed.selection.head_vcol.map(|v| v as f32 * widths.cell_w).unwrap_or_else(|| self.x_of(ed, head, lay, widths))
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

// ── 編輯動作（亦供巨集錄製與播放） ───────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EdAction {
    Insert(String),
    Newline { indent: bool },
    Tab,
    BackTab,
    Backspace,
    Delete,
    DeleteWord { forward: bool },
    /// macOS 的 ⌘Delete：刪除至行首／行尾，既有選取優先刪除。
    DeleteLineEdge { forward: bool },
    Left { extend: bool, word: bool },
    Right { extend: bool, word: bool },
    Up { extend: bool },
    Down { extend: bool },
    PageUp { extend: bool },
    PageDown { extend: bool },
    Home { extend: bool },
    LineStart { extend: bool },
    End { extend: bool },
    DocStart { extend: bool },
    DocEnd { extend: bool },
    /// Alt+Shift+方向鍵：延伸矩形選取
    Block { dx: i32, dy: i32 },
    ScrollLines(i32),
    ToggleInsert,
    ClearSelection,
}

/// 檢視產生、需由 App 處理的請求
#[derive(Default)]
pub struct ViewOutput {
    pub commands: Vec<C>,
    /// 寫入系統剪貼簿
    pub copy: Option<String>,
    /// 由系統剪貼簿貼上（中鍵貼上）
    pub paste: bool,
    /// 加入個人字典
    pub add_word: Option<String>,
    /// 錄製巨集用：本幀執行的編輯動作
    pub actions: Vec<EdAction>,
    /// 編輯區取得焦點
    pub focused: bool,
}

/// 自動補全的成對字元
fn closing_pair(c: char) -> Option<char> {
    match c {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        '"' => Some('"'),
        '\'' => Some('\''),
        _ => None,
    }
}

/// 執行一個編輯動作
pub fn apply_action(doc: &mut Document, view: &mut ViewState, act: &EdAction, settings: &Settings, widths: &CharWidths, lay: &Layout) {
    if doc.read_only && matches!(act, EdAction::Insert(_) | EdAction::Newline { .. } | EdAction::Tab | EdAction::BackTab | EdAction::Backspace | EdAction::Delete | EdAction::DeleteWord { .. } | EdAction::DeleteLineEdge { .. }) {
        return;
    }
    let sel_mode = doc.selecting_mode;
    let ext = |e: bool| e || sel_mode;
    let is_vertical = matches!(act, EdAction::Up { .. } | EdAction::Down { .. } | EdAction::PageUp { .. } | EdAction::PageDown { .. });
    if !is_vertical {
        view.preferred_x = None;
    }
    match act {
        EdAction::Insert(text) => insert_typed(doc, text, settings),
        EdAction::Newline { indent } => {
            let hint = doc.selection_hint();
            let auto = *indent && settings.auto_indent;
            doc.edit(EditKind::Other, hint, |ed| {
                let block = ed.selection.is_block();
                if block {
                    ed.selection = Selection::caret(ed.selection.head);
                }
                let head = ed.selection.start();
                let lead: String = if auto {
                    ed.lines[head.line].chars().take(head.col).take_while(|c| *c == ' ' || *c == '\t').collect()
                } else {
                    String::new()
                };
                ed.insert_text(&format!("\n{lead}"));
            });
        }
        EdAction::Tab => {
            let sel = doc.editor.selection;
            if !sel.is_block() && sel.start().line != sel.end().line {
                indent_selection(doc, settings, true);
            } else {
                let text = if settings.insert_spaces_for_tab {
                    let head = doc.editor.selection.head;
                    let line = &doc.editor.lines[head.line];
                    let vcol = char_col_to_visual_col(line, head.col);
                    " ".repeat(tab_columns_at(vcol))
                } else {
                    "\t".to_string()
                };
                let hint = doc.selection_hint();
                doc.edit(EditKind::Typing, hint, |ed| ed.insert_text(&text));
            }
        }
        EdAction::BackTab => indent_selection(doc, settings, false),
        EdAction::Backspace => {
            let hint = doc.selection_hint();
            doc.edit(EditKind::Typing, hint, |ed| ed.delete_backspace());
        }
        EdAction::Delete => {
            let hint = doc.selection_hint();
            doc.edit(EditKind::Other, hint, |ed| ed.delete_forward());
        }
        EdAction::DeleteWord { forward } => {
            let hint = doc.selection_hint();
            doc.edit(EditKind::Other, hint, |ed| ed.delete_word(*forward));
        }
        EdAction::DeleteLineEdge { forward } => {
            let hint = doc.selection_hint();
            doc.edit(EditKind::Other, hint, |ed| {
                if ed.selection.is_empty() {
                    if *forward { ed.move_end(true); } else { ed.move_home(true); }
                }
                ed.delete_selection();
            });
        }
        EdAction::Left { extend, word } => {
            if *word {
                doc.editor.move_word(false, ext(*extend))
            } else if !ext(*extend) && !doc.editor.selection.is_empty() && !doc.editor.selection.is_block() {
                let s = doc.editor.selection.start();
                doc.editor.move_caret_to(s, false);
            } else {
                doc.editor.move_left(ext(*extend))
            }
        }
        EdAction::Right { extend, word } => {
            if *word {
                doc.editor.move_word(true, ext(*extend))
            } else if !ext(*extend) && !doc.editor.selection.is_empty() && !doc.editor.selection.is_block() {
                let e = doc.editor.selection.end();
                doc.editor.move_caret_to(e, false);
            } else {
                doc.editor.move_right(ext(*extend))
            }
        }
        EdAction::Up { extend } => move_rows(doc, view, -1, ext(*extend), widths, lay),
        EdAction::Down { extend } => move_rows(doc, view, 1, ext(*extend), widths, lay),
        EdAction::PageUp { extend } => {
            let n = view.visible_rows.max(2) as i64 - 1;
            view.top_row = view.top_row.saturating_sub(n as usize);
            move_rows(doc, view, -n, ext(*extend), widths, lay)
        }
        EdAction::PageDown { extend } => {
            let n = view.visible_rows.max(2) as i64 - 1;
            let max_top = view.row_count(&doc.editor).saturating_sub(1);
            view.top_row = (view.top_row + n as usize).min(max_top);
            move_rows(doc, view, n, ext(*extend), widths, lay)
        }
        EdAction::Home { extend } => doc.editor.move_home_smart(ext(*extend)),
        EdAction::LineStart { extend } => doc.editor.move_home(ext(*extend)),
        EdAction::End { extend } => doc.editor.move_end(ext(*extend)),
        EdAction::DocStart { extend } => doc.editor.move_doc_edge(false, ext(*extend)),
        EdAction::DocEnd { extend } => doc.editor.move_doc_edge(true, ext(*extend)),
        EdAction::Block { dx, dy } => {
            let ed = &mut doc.editor;
            if !ed.selection.is_block() {
                let head = ed.selection.head;
                let v = char_col_to_visual_col(&ed.lines[head.line], head.col);
                ed.selection = Selection::block(head, head, v, v);
            }
            let head = ed.selection.head;
            let head_v = ed.selection.head_vcol.unwrap_or(0);
            let new_line = (head.line as i64 + *dy as i64).clamp(0, ed.lines.len() as i64 - 1) as usize;
            let new_v = (head_v as i64 + *dx as i64).max(0) as usize;
            let col = visual_col_to_char_col(&ed.lines[new_line], new_v);
            ed.update_block_selection(TextPosition::new(new_line, col), new_v);
        }
        EdAction::ScrollLines(n) => {
            let max_top = view.row_count(&doc.editor).saturating_sub(1);
            view.top_row = (view.top_row as i64 + *n as i64).clamp(0, max_top as i64) as usize;
            return;
        }
        EdAction::ToggleInsert => {
            doc.editor.toggle_overwrite_mode();
        }
        EdAction::ClearSelection => {
            let h = doc.editor.selection.head;
            doc.editor.selection = Selection::caret(h);
            doc.selecting_mode = false;
        }
    }
    view.ensure_visible = true;
}

/// 輸入文字（處理自動補全成對字元）
fn insert_typed(doc: &mut Document, text: &str, settings: &Settings) {
    let mut chars = text.chars();
    let single = match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    };
    let hint = doc.selection_hint();
    let ed = &doc.editor;
    let sel = ed.selection;
    if settings.auto_complete_pair && !sel.is_block() && !ed.overwrite_mode
        && let Some(c) = single {
            // 輸入的右括號正好是游標右側的字：直接跳過
            let next = ed.lines[sel.head.line].chars().nth(sel.head.col);
            if sel.is_empty() && next == Some(c) && matches!(c, ')' | ']' | '}' | '"' | '\'') {
                doc.editor.move_right(false);
                return;
            }
            if let Some(close) = closing_pair(c) {
                if !sel.is_empty() && settings.wrap_selection_with_pair {
                    let inner = ed.selected_text();
                    doc.edit(EditKind::Other, hint, |ed| {
                        ed.replace_selection(&format!("{c}{inner}{close}"));
                        let s = ed.selection.start();
                        let e = ed.selection.end();
                        ed.select_range(TextPosition::new(s.line, s.col + 1), TextPosition::new(e.line, e.col.saturating_sub(1)));
                    });
                    return;
                }
                doc.edit(EditKind::Typing, hint, |ed| {
                    ed.insert_text(&format!("{c}{close}"));
                    ed.move_left(false);
                });
                return;
            }
        }
    doc.edit(EditKind::Typing, hint, |ed| ed.insert_text(text));
}

/// 選取範圍所涵蓋的行（選取結束於行首時不含該行）
pub fn selected_line_range(ed: &EditorState) -> (usize, usize) {
    let s = ed.selection.start();
    let e = ed.selection.end();
    let last = if e.line > s.line && e.col == 0 && !ed.selection.is_block() { e.line - 1 } else { e.line };
    (s.line, last)
}

/// 增加或減少選取行的縮排
pub fn indent_selection(doc: &mut Document, settings: &Settings, increase: bool) {
    let (a, b) = selected_line_range(&doc.editor);
    let unit = if settings.insert_spaces_for_tab { " ".repeat(settings.indent_size.max(1)) } else { "\t".to_string() };
    let size = settings.indent_size.max(1);
    let had_sel = !doc.editor.selection.is_empty();
    doc.edit(EditKind::Other, a..b + 1, |ed| {
        for l in a..=b {
            ed.lines[l] = if increase { textops::indent_line(&ed.lines[l], &unit) } else { textops::unindent_line(&ed.lines[l], size) };
        }
        if had_sel || a != b {
            ed.select_range(TextPosition::new(a, 0), TextPosition::new(b, ed.line_char_count(b)));
        } else {
            let h = ed.selection.head;
            ed.set_caret(h.line, h.col);
        }
    });
}

/// 以視覺列為單位上下移動游標（自動換行時移動到相鄰的視覺列）
fn move_rows(doc: &mut Document, view: &mut ViewState, delta: i64, extend: bool, widths: &CharWidths, lay: &Layout) {
    let ed = &mut doc.editor;
    if ed.selection.is_block() && extend {
        return;
    }
    let head = ed.selection.head;
    let row = view.row_of(head);
    let x = view.preferred_x.unwrap_or_else(|| view.x_of(ed, head, lay, widths));
    let total = view.row_count(ed) as i64;
    let target = (row as i64 + delta).clamp(0, total - 1) as usize;
    let pos = if target as i64 == row as i64 + delta || delta.abs() > 1 {
        view.pos_at_x(ed, target, x, lay, widths)
    } else if delta < 0 {
        TextPosition::new(0, 0)
    } else {
        let last = ed.lines.len() - 1;
        TextPosition::new(last, ed.line_char_count(last))
    };
    ed.move_caret_to(pos, extend);
    view.preferred_x = Some(x);
}

// ── 鍵盤事件轉換 ────────────────────────────────────────────────

fn key_to_action(key: Key, m: Modifiers) -> Option<EdAction> {
    let mac = cfg!(target_os = "macos");
    // macOS：Option 移動字詞、⌘ 移至行首尾或文件頭尾；其他平台：Ctrl 移動字詞
    let word = if mac { m.alt } else { m.ctrl };
    let line_edge = mac && m.command;
    let extend = m.shift;
    let block = m.alt && m.shift && (!mac || m.command);
    Some(match key {
        Key::ArrowLeft if block => EdAction::Block { dx: -1, dy: 0 },
        Key::ArrowRight if block => EdAction::Block { dx: 1, dy: 0 },
        Key::ArrowUp if block => EdAction::Block { dx: 0, dy: -1 },
        Key::ArrowDown if block => EdAction::Block { dx: 0, dy: 1 },
        Key::ArrowLeft if line_edge => EdAction::LineStart { extend },
        Key::ArrowRight if line_edge => EdAction::End { extend },
        Key::ArrowUp if line_edge => EdAction::DocStart { extend },
        Key::ArrowDown if line_edge => EdAction::DocEnd { extend },
        Key::ArrowLeft => EdAction::Left { extend, word },
        Key::ArrowRight => EdAction::Right { extend, word },
        Key::ArrowUp if m.command && !m.alt => EdAction::ScrollLines(-1),
        Key::ArrowDown if m.command && !m.alt => EdAction::ScrollLines(1),
        Key::ArrowUp if !m.alt => EdAction::Up { extend },
        Key::ArrowDown if !m.alt => EdAction::Down { extend },
        Key::PageUp => EdAction::PageUp { extend },
        Key::PageDown => EdAction::PageDown { extend },
        Key::Home if mac || m.command => EdAction::DocStart { extend },
        Key::End if mac || m.command => EdAction::DocEnd { extend },
        Key::Home => EdAction::Home { extend },
        Key::End => EdAction::End { extend },
        Key::Backspace if line_edge => EdAction::DeleteLineEdge { forward: false },
        Key::Delete if line_edge => EdAction::DeleteLineEdge { forward: true },
        Key::Backspace if word => EdAction::DeleteWord { forward: false },
        Key::Delete if word => EdAction::DeleteWord { forward: true },
        Key::Backspace if !m.command => EdAction::Backspace,
        Key::Delete if !m.command && !m.shift => EdAction::Delete,
        Key::Enter if !m.command && !m.alt => EdAction::Newline { indent: !m.shift },
        Key::Tab if m.shift && !m.command => EdAction::BackTab,
        Key::Tab if !m.command && !m.alt => EdAction::Tab,
        Key::Insert if m.is_none() => EdAction::ToggleInsert,
        Key::Escape => EdAction::ClearSelection,
        _ => return None,
    })
}

// ── 繪製與互動 ──────────────────────────────────────────────────

pub struct ViewEnv<'a> {
    pub settings: &'a Settings,
    pub widths: &'a mut CharWidths,
    pub spell: Option<&'a SpellChecker>,
    pub theme_name: &'a str,
    pub is_dark: bool,
    pub editor_family: &'a str,
}

fn rgb((r, g, b): (u8, u8, u8)) -> Color32 {
    Color32::from_rgb(r, g, b)
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct GlyphStyle {
    color: Color32,
    bold: bool,
    italic: bool,
}

/// Keep glyph advances unchanged: the caret and wrapping share unstyled widths.
fn paint_syntax_text(painter: &egui::Painter, pos: Pos2, text: String, font: FontId, style: GlyphStyle) {
    if !style.bold && !style.italic {
        painter.text(pos, Align2::LEFT_TOP, text, font, style.color);
        return;
    }
    let format = egui::TextFormat { font_id: font, color: style.color, italics: style.italic, ..Default::default() };
    let galley = painter.layout_job(egui::text::LayoutJob::simple_format(text, format));
    if style.bold {
        // Synthetic weight avoids loading a second font or changing grid metrics.
        let offset = Vec2::new(0.5 / painter.ctx().pixels_per_point(), 0.0);
        painter.galley(pos + offset, galley.clone(), style.color);
    }
    painter.galley(pos, galley, style.color);
}

/// 編輯區的固定 Id（App 用來判斷焦點與要求焦點）
pub fn editor_id(doc_id: u64) -> egui::Id {
    egui::Id::unique(("madedit-editor", doc_id))
}

/// 顯示文字編輯區
pub fn show(ui: &mut egui::Ui, doc: &mut Document, view: &mut ViewState, env: &mut ViewEnv, out: &mut ViewOutput) {
    let settings = env.settings;
    let mut lay = Layout::new(settings, doc.mode);
    let ctx = ui.ctx().clone();

    // 量測可見範圍的字元（含行號與控制字元符號）；取樣有上限，避免超長行每幀掃整份文件
    let first_guess = view.top_row.min(doc.editor.lines.len().saturating_sub(1));
    {
        let mut budget = 4096usize;
        let mut sample: Vec<char> = Vec::new();
        for line in doc.editor.lines.iter().skip(first_guess.saturating_sub(5)).take(200) {
            if budget == 0 {
                break;
            }
            let take = budget.min(512);
            sample.extend(line.chars().take(take));
            sample.push(' ');
            budget = budget.saturating_sub(take + 1);
        }
        let extra = "0123456789·»↵ ".chars();
        let pre = view.preedit.clone().unwrap_or_default();
        env.widths.prepare(&ctx, &lay.font, env.editor_family, sample.into_iter().chain(extra).chain(pre.chars()));
    }
    let widths: &CharWidths = env.widths;
    lay.row_h = (widths.text_h * settings.line_spacing.clamp(100, 250) as f32 / 100.0).round().max(1.0);

    let colors = syntax::theme_colors(env.theme_name, env.is_dark);
    let bg = rgb(colors.background);
    let fg = rgb(colors.foreground);
    let visuals = ui.visuals().clone();
    let sel_color = colors
        .selection
        .map(|c| rgb(c).gamma_multiply(0.85))
        .unwrap_or(visuals.selection.bg_fill.gamma_multiply(0.6));
    let faint = fg.gamma_multiply(0.35);
    // 主題的目前行底色常與註解色相同（如 base16-ocean.dark），降低不透明度以免文字被吃掉
    let active_line_color = colors
        .line_highlight
        .map(|c| rgb(c).gamma_multiply(0.3))
        .unwrap_or(if env.is_dark { Color32::from_white_alpha(14) } else { Color32::from_black_alpha(12) });

    // ── 版面配置：尺標、行號欄、文字區、捲軸 ──
    let full = ui.available_rect_before_wrap();
    let sb = 14.0;
    let ruler_h = if settings.show_ruler { (widths.text_h * 0.8).max(14.0) } else { 0.0 };
    let line_count = doc.editor.lines.len();
    let digits = line_count.to_string().len().max(3);
    let num_w = if settings.show_line_numbers { digits as f32 * widths.cell_w + 10.0 } else { 0.0 };
    let mark_w = if settings.show_bookmarks { 14.0 } else { 0.0 };
    let gutter_w = num_w + mark_w + 4.0;
    let text_rect = Rect::from_min_max(
        Pos2::new(full.left() + gutter_w, full.top() + ruler_h),
        Pos2::new(full.right() - sb, full.bottom() - sb),
    );
    if text_rect.width() < 10.0 || text_rect.height() < 10.0 {
        return;
    }
    view.text_rect = text_rect;
    lay.wrap_width = match lay.wrap {
        WrapMode::NoWrap => f32::INFINITY,
        WrapMode::ByWindow => text_rect.width() - widths.cell_w,
        WrapMode::ByColumn => settings.wrap_column.max(1) as f32 * widths.cell_w,
    };
    view.update_wrap(doc, &lay, widths);
    view.begin_frame(doc.version, lay.fixed, widths);
    view.visible_rows = ((text_rect.height() / lay.row_h).floor() as usize).max(1);
    doc.editor.viewport.visible_line_count = view.visible_rows;

    let id = editor_id(doc.id);
    let response = ui.interact(text_rect, id, Sense::click_and_drag());
    let gutter_rect = Rect::from_min_max(Pos2::new(full.left(), text_rect.top()), Pos2::new(text_rect.left(), text_rect.bottom()));
    let gutter_resp = ui.interact(gutter_rect, id.with("gutter"), Sense::click_and_drag());
    let vbar = Rect::from_min_max(Pos2::new(text_rect.right(), full.top()), Pos2::new(full.right(), text_rect.bottom()));
    let hbar = Rect::from_min_max(Pos2::new(full.left(), text_rect.bottom()), Pos2::new(text_rect.right(), full.bottom()));
    let vbar_resp = ui.interact(vbar, id.with("vbar"), Sense::click_and_drag());
    let hbar_resp = ui.interact(hbar, id.with("hbar"), Sense::click_and_drag());

    if response.clicked() || response.drag_started() || gutter_resp.clicked() || gutter_resp.drag_started() {
        response.request_focus();
    }
    let focused = response.has_focus();
    out.focused = focused;
    if focused {
        ctx.memory_mut(|m| {
            m.set_focus_lock_filter(
                id,
                egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true },
            )
        });
    }

    let to_screen_x = |x_rel: f32, w: f32| -> f32 {
        if lay.rtl {
            text_rect.right() + view.scroll_x - x_rel - w
        } else {
            text_rect.left() - view.scroll_x + x_rel
        }
    };
    let _ = to_screen_x;

    // ── 鍵盤與輸入法 ──
    if focused {
        let events = ctx.input(|i| i.events.clone());
        let mut actions: Vec<EdAction> = Vec::new();
        for ev in &events {
            match ev {
                Event::Text(t) if !t.chars().any(|c| c.is_control()) => {
                    view.preedit = None;
                    actions.push(EdAction::Insert(t.clone()));
                }
                Event::Ime(egui::ImeEvent::Preedit { text, .. }) => {
                    view.preedit = (!text.is_empty()).then(|| text.clone());
                }
                Event::Ime(egui::ImeEvent::Commit(t)) => {
                    view.preedit = None;
                    if !t.is_empty() {
                        actions.push(EdAction::Insert(t.clone()));
                    }
                }
                Event::Key { key, pressed: true, modifiers, .. } => {
                    if view.preedit.is_some() {
                        continue;
                    }
                    if let Some(a) = key_to_action(*key, *modifiers) {
                        actions.push(a);
                    }
                }
                _ => {}
            }
        }
        for a in actions {
            apply_action(doc, view, &a, settings, widths, &lay);
            // 編輯後立即重建換行快取，後續動作與本幀繪製才不會用舊的斷行位置索引新內容
            view.update_wrap(doc, &lay, widths);
            view.clear_frame_layout();
            out.actions.push(a);
        }
        if doc.mode == EditMode::Column && !doc.editor.selection.is_block() {
            // 列塊模式下游標一律為矩形（零寬）選取，方向鍵與輸入都以欄為單位
            let h = doc.editor.selection.head;
            if doc.editor.selection.is_empty() {
                let v = char_col_to_visual_col(&doc.editor.lines[h.line], h.col);
                doc.editor.selection = Selection::block(h, h, v, v);
            }
        }
    }

    // ── 滑鼠 ──
    let pointer = ctx.input(|i| i.pointer.clone());
    let modifiers = ctx.input(|i| i.modifiers);
    let hit = |view: &ViewState, ed: &EditorState, p: Pos2| -> (TextPosition, usize) {
        let row = view.top_row + ((p.y - text_rect.top()) / lay.row_h).floor().max(0.0) as usize;
        let row = row.min(view.row_count(ed).saturating_sub(1));
        let x_rel = if lay.rtl { text_rect.right() + view.scroll_x - p.x } else { p.x - text_rect.left() + view.scroll_x };
        let pos = view.pos_at_x(ed, row, x_rel.max(0.0), &lay, widths);
        let vcol = (x_rel.max(0.0) / widths.cell_w).round() as usize;
        (pos, vcol)
    };

    if let Some(p) = pointer.interact_pos() {
        let block_drag = modifiers.alt || doc.mode == EditMode::Column;
        if response.drag_started_by(PointerButton::Primary) || (response.clicked_by(PointerButton::Primary) && !response.double_clicked()) {
            // 選取的錨點必須是「按下」的位置：egui 的 drag_started 要等指標移動超過臨界距離
            // 才會觸發，若用當下的指標位置當錨點，按下的那個字元會被漏掉（往回拖曳時最明顯）。
            let origin = pointer.press_origin().unwrap_or(p);
            let (pos, vcol) = hit(view, &doc.editor, origin);
            let ed = &mut doc.editor;
            if block_drag && view.wrap.key.is_none() {
                let anchor = if modifiers.shift { ed.selection.anchor } else { pos };
                let anchor_v = if modifiers.shift {
                    ed.selection.anchor_vcol.unwrap_or_else(|| char_col_to_visual_col(&ed.lines[anchor.line], anchor.col))
                } else {
                    vcol
                };
                ed.selection = Selection::block(anchor, pos, anchor_v, vcol);
                view.drag = Some(Drag::Block);
            } else {
                ed.move_caret_to(pos, modifiers.shift || doc.selecting_mode);
                view.drag = Some(Drag::Select);
            }
            view.preferred_x = None;
            view.highlight_word = None;
        }
        if response.double_clicked() {
            let (pos, _) = hit(view, &doc.editor, p);
            let (s, e) = doc.editor.word_range_at(pos);
            doc.editor.select_range(s, e);
            if settings.double_click_highlight && s != e {
                let w: String = doc.editor.selected_text();
                view.highlight_word = Some(w);
            }
            view.drag = None;
        }
        if response.triple_clicked() {
            let (pos, _) = hit(view, &doc.editor, p);
            let end = if pos.line + 1 < doc.editor.lines.len() {
                TextPosition::new(pos.line + 1, 0)
            } else {
                TextPosition::new(pos.line, doc.editor.line_char_count(pos.line))
            };
            doc.editor.select_range(TextPosition::new(pos.line, 0), end);
            view.drag = None;
        }
        if response.dragged_by(PointerButton::Primary) {
            // 拖曳到文字區外時自動捲動
            if p.y < text_rect.top() {
                view.top_row = view.top_row.saturating_sub(1);
            } else if p.y > text_rect.bottom() {
                view.top_row = (view.top_row + 1).min(view.row_count(&doc.editor).saturating_sub(1));
            }
            if !lay.rtl && view.wrap.key.is_none() {
                if p.x < text_rect.left() {
                    view.scroll_x = (view.scroll_x - widths.cell_w).max(0.0);
                } else if p.x > text_rect.right() {
                    view.scroll_x += widths.cell_w;
                }
            }
            let (pos, vcol) = hit(view, &doc.editor, p);
            match view.drag {
                Some(Drag::Block) => doc.editor.update_block_selection(pos, vcol),
                Some(Drag::Select) => doc.editor.move_caret_to(pos, true),
                _ => {}
            }
            ctx.request_repaint();
        }
        if response.middle_clicked() && settings.middle_click_paste {
            let (pos, _) = hit(view, &doc.editor, p);
            doc.editor.move_caret_to(pos, false);
            out.paste = true;
        }
        if response.secondary_clicked() {
            let (pos, _) = hit(view, &doc.editor, p);
            view.spell_ctx = None;
            if let Some(sc) = env.spell.filter(|_| settings.spell_check) {
                let (s, e) = doc.editor.word_range_at(pos);
                if s != e && s.line == e.line {
                    let word: String = doc.editor.lines[s.line].chars().skip(s.col).take(e.col - s.col).collect();
                    if sc.misspelled_ranges(&word).first() == Some(&(0, word.chars().count())) {
                        view.spell_ctx = Some(SpellContext { line: s.line, start: s.col, end: e.col, suggestions: sc.suggest(&word), word });
                    }
                }
            }
            // 在選取範圍外按右鍵時移動游標
            let sel = doc.editor.selection;
            if sel.is_empty() || pos < sel.start() || pos > sel.end() {
                doc.editor.move_caret_to(pos, false);
            }
        }
    }
    if response.drag_stopped() {
        if settings.auto_copy_mouse_selection && !doc.editor.selection.is_empty() {
            out.copy = Some(doc.editor.copy_to_clipboard());
        }
        view.drag = None;
    }

    // 行號欄：點選或拖曳選取整行，點書籤區切換書籤
    if let Some(p) = pointer.interact_pos() {
        let row_at = |view: &ViewState, ed: &EditorState, at: Pos2| {
            let r = view.top_row + ((at.y - text_rect.top()) / lay.row_h).floor().max(0.0) as usize;
            view.row_info(ed, r.min(view.row_count(ed).saturating_sub(1))).0
        };
        if gutter_resp.clicked() && settings.show_bookmarks && p.x < gutter_rect.left() + mark_w {
            let line = row_at(view, &doc.editor, p);
            doc.editor.move_caret_to(TextPosition::new(line, 0), false);
            doc.editor.toggle_bookmark();
        } else if gutter_resp.drag_started() || gutter_resp.clicked() {
            // 起始行同樣取「按下」的位置，否則拖曳起點會落在拖曳開始時的指標位置
            let origin = pointer.press_origin().unwrap_or(p);
            let line = row_at(view, &doc.editor, origin);
            view.drag = Some(Drag::Lines { anchor_line: line });
        }
        if let Some(Drag::Lines { anchor_line }) = view.drag {
            let line = row_at(view, &doc.editor, p);
            let (a, b) = (anchor_line.min(line), anchor_line.max(line));
            let end = if b + 1 < doc.editor.lines.len() { TextPosition::new(b + 1, 0) } else { TextPosition::new(b, doc.editor.line_char_count(b)) };
            let start = TextPosition::new(a, 0);
            if line < anchor_line {
                doc.editor.select_range(end, start);
            } else {
                doc.editor.select_range(start, end);
            }
            if !gutter_resp.dragged() {
                view.drag = None;
            }
        }
    }

    // 滾輪：垂直捲動（Shift 水平），Ctrl/⌘ 交由 App 縮放
    if ui.rect_contains_pointer(full) && !modifiers.command {
        let delta = ctx.input(|i| i.smooth_scroll_delta());
        if delta.y != 0.0 {
            let rows = -delta.y / lay.row_h;
            let acc = ctx.data_mut(|d| {
                let acc: &mut f32 = d.get_temp_mut_or_default(id.with("wheel"));
                *acc += rows;
                let whole = acc.trunc();
                *acc -= whole;
                whole
            });
            let max_top = view.row_count(&doc.editor).saturating_sub(1) as i64;
            view.top_row = (view.top_row as i64 + acc as i64).clamp(0, max_top) as usize;
        }
        if delta.x != 0.0 && view.wrap.key.is_none() {
            view.scroll_x = (view.scroll_x - delta.x).max(0.0);
        }
    }

    // ── 捲軸 ──
    let total_rows = view.row_count(&doc.editor);
    let vis = view.visible_rows;
    let max_top = total_rows.saturating_sub(1);
    let v_track = vbar.shrink2(Vec2::new(2.0, 2.0));
    let v_thumb_h = (v_track.height() * vis as f32 / (total_rows + vis).max(1) as f32).clamp(20.0, v_track.height());
    let v_frac = if max_top == 0 { 0.0 } else { view.top_row as f32 / max_top as f32 };
    let v_thumb = Rect::from_min_size(
        Pos2::new(v_track.left(), v_track.top() + (v_track.height() - v_thumb_h) * v_frac),
        Vec2::new(v_track.width(), v_thumb_h),
    );
    if let Some(p) = vbar_resp.interact_pointer_pos() {
        if vbar_resp.drag_started() {
            view.drag = Some(Drag::VScroll { grab: if v_thumb.contains(p) { p.y - v_thumb.top() } else { v_thumb_h / 2.0 } });
        }
        if let Some(Drag::VScroll { grab }) = view.drag {
            let frac = ((p.y - grab - v_track.top()) / (v_track.height() - v_thumb_h).max(1.0)).clamp(0.0, 1.0);
            view.top_row = (frac * max_top as f32).round() as usize;
        } else if vbar_resp.clicked() {
            if p.y < v_thumb.top() {
                view.top_row = view.top_row.saturating_sub(vis);
            } else if p.y > v_thumb.bottom() {
                view.top_row = (view.top_row + vis).min(max_top);
            }
        }
    }
    if vbar_resp.drag_stopped() {
        view.drag = None;
    }

    // ── 游標可見性 ──
    let caret_key = (doc.editor.selection.head, doc.version, doc.editor.selection.head_vcol);
    if view.last_caret != Some(caret_key) || view.ensure_visible {
        if view.drag.is_none() || matches!(view.drag, Some(Drag::Select | Drag::Block)) {
            view.scroll_to_caret(&doc.editor, &lay, widths, text_rect.width(), settings.typewriter_mode && view.drag.is_none());
        }
        view.last_caret = Some(caret_key);
        view.ensure_visible = false;
        view.blink_start = ctx.input(|i| i.time);
    }
    view.top_row = view.top_row.min(max_top);

    // ── 繪製 ──
    let painter = ui.painter_at(full);
    painter.rect_filled(full, CornerRadius::ZERO, bg);
    let gutter_bg = if env.is_dark { bg.gamma_multiply(0.8) } else { Color32::from_gray(240) };
    painter.rect_filled(Rect::from_min_max(full.min, Pos2::new(text_rect.left() - 2.0, text_rect.bottom())), CornerRadius::ZERO, gutter_bg);
    let text_painter = painter.with_clip_rect(text_rect.expand2(Vec2::new(0.0, 0.0)));

    let first_row = view.top_row;
    let last_row = (first_row + vis + 1).min(total_rows);
    let first_line = view.row_info(&doc.editor, first_row).0;
    let last_line = if last_row > first_row { view.row_info(&doc.editor, last_row - 1).0 } else { first_line };
    // 每行水平可見的字元範圍：超長行只解析這段視窗，成本與行長無關
    let mut windows: Vec<(usize, usize, usize)> = Vec::with_capacity(last_line + 1 - first_line);
    for row in first_row..last_row {
        let line = view.row_line(&doc.editor, row);
        let start = view.row_start(&doc.editor, row);
        let layout = view.line_layout(&doc.editor, line, lay.fixed, widths);
        let row_end = if row + 1 < last_row && view.row_line(&doc.editor, row + 1) == line {
            view.row_start(&doc.editor, row + 1)
        } else {
            layout.0.len()
        };
        let (lo, hi) = visible_chars(&layout.1, start, row_end, view.scroll_x, text_rect.width(), lay.rtl);
        match windows.last_mut() {
            Some(w) if w.0 == line => {
                w.1 = w.1.min(lo);
                w.2 = w.2.max(hi);
            }
            _ => windows.push((line, lo, hi)),
        }
    }
    let spans = doc.syntax_cache.highlight(
        &doc.editor.lines,
        first_line,
        last_line + 1 - first_line,
        &doc.syntax,
        env.theme_name,
        env.is_dark,
        &windows,
    );
    if doc.syntax_cache.has_pending() {
        // 分時解析尚未完成：要求下一幀續跑，把其餘行補上顏色
        ctx.request_repaint();
    }
    let ed = &doc.editor;
    let sel = ed.selection;
    let block_bounds = sel.is_block().then(|| sel.visual_bounds(ed));
    let head_row = view.row_of(sel.head);

    // 對應括號
    let braces = if settings.mark_brace_pair {
        let key = (sel.head, doc.version);
        match &view.brace_cache {
            Some((k, v)) if *k == key => *v,
            _ => {
                let v = textops::find_brace_pair(&ed.lines, sel.head);
                view.brace_cache = Some((key, v));
                v
            }
        }
    } else {
        None
    };

    let mut max_w = 0.0f32;
    let control_color = if env.is_dark { Color32::from_rgb(90, 110, 120) } else { Color32::from_rgb(170, 190, 200) };
    let eol_color = Color32::from_rgb(80, 170, 80);
    // 可見列依邏輯行分組：每行只建立一次字元與顏色版面，超長行不會逐視覺列重建
    let mut spell_ranges: Vec<(usize, usize)> = Vec::new();
    let mut row_iter = first_row;
    while row_iter < last_row {
        let line = view.row_line(ed, row_iter);
        let layout = view.line_layout(ed, line, lay.fixed, widths);
        let chars = &layout.0;
        let xs = &layout.1;
        let char_count = chars.len();
        // 拼字檢查每行一次（以內容雜湊快取，不複製整行文字當鍵）
        spell_ranges.clear();
        if settings.spell_check && let Some(sc) = env.spell {
            let hash = content_hash(&ed.lines[line]);
            if let Some(r) = view.spell_cache.get(&hash) {
                spell_ranges = r.clone();
            } else {
                if view.spell_cache.len() > 4000 {
                    view.spell_cache.clear();
                }
                let r = sc.misspelled_ranges(&ed.lines[line]);
                view.spell_cache.insert(hash, r.clone());
                spell_ranges = r;
            }
        }
        // Empty default spans avoid a per-character table for plain text.
        let line_spans: &[HighlightSpan] = spans.get(line - first_line).map(|v| v.as_slice()).unwrap_or(&[]);
        let default_style = GlyphStyle { color: fg, bold: false, italic: false };
        let styles: Option<Vec<GlyphStyle>> = if line_spans.len() <= 1 && line_spans.first().is_none_or(|s| s.text.is_empty() && !s.bold && !s.italic) {
            None
        } else {
            let mut v = Vec::with_capacity(chars.len());
            for sp in line_spans {
                if sp.skip_chars > 0 {
                    // 超長行只解析可見視窗：視窗之前的字元以前景色顯示
                    v.resize(v.len() + sp.skip_chars, default_style);
                }
                let style = GlyphStyle { color: rgb(sp.color_rgb), bold: sp.bold, italic: sp.italic };
                v.extend(std::iter::repeat_n(style, sp.text.chars().count()));
            }
            v.resize(chars.len(), default_style);
            Some(v)
        };
        let style_at = |i: usize| styles.as_ref().map_or(default_style, |v| v[i]);
        let mut draw_row = |row: usize, start: usize, end: usize| {
        let y = text_rect.top() + (row - first_row) as f32 * lay.row_h;
        let text_y = y + (lay.row_h - widths.text_h) / 2.0;
        let x0 = xs[start];
        let row_w = xs[end] - x0;
        max_w = max_w.max(row_w);
        let sx = |x_rel: f32, w: f32| -> f32 {
            if lay.rtl { text_rect.right() + view.scroll_x - x_rel - w } else { text_rect.left() - view.scroll_x + x_rel }
        };
        let row_rect = Rect::from_min_size(Pos2::new(text_rect.left(), y), Vec2::new(text_rect.width(), lay.row_h));

        // 目前行
        if settings.mark_active_line && sel.head.line == line && sel.is_empty() {
            painter.rect_filled(row_rect, CornerRadius::ZERO, active_line_color);
        }

        // 選取範圍
        if let Some(b) = block_bounds {
            if line >= b.min_line && line <= b.max_line {
                let a = sx(b.min_vcol as f32 * widths.cell_w, (b.max_vcol - b.min_vcol) as f32 * widths.cell_w);
                let w = ((b.max_vcol - b.min_vcol) as f32 * widths.cell_w).max(1.5);
                text_painter.rect_filled(Rect::from_min_size(Pos2::new(a, y), Vec2::new(w, lay.row_h)), CornerRadius::ZERO, sel_color);
            }
        } else if !sel.is_empty() {
            let (s, e) = (sel.start(), sel.end());
            if line >= s.line && line <= e.line {
                let a = if line == s.line { s.col.max(start) } else { start };
                let bcol = if line == e.line { e.col.min(end) } else { end };
                let last_row_of_line = end == char_count;
                if a <= bcol && (a < bcol || (line < e.line && last_row_of_line)) {
                    let xa = xs[a] - x0;
                    let mut xb = xs[bcol] - x0;
                    if line < e.line && last_row_of_line && bcol == end {
                        xb += widths.space_w; // 換行字元
                    }
                    let left = sx(xa, xb - xa);
                    text_painter.rect_filled(Rect::from_min_size(Pos2::new(left, y), Vec2::new(xb - xa, lay.row_h)), CornerRadius::ZERO, sel_color);
                }
            }
        }

        // 雙擊標示的相同字詞
        if let Some(w) = view.highlight_word.as_deref() {
            let wc: Vec<char> = w.chars().collect();
            if !wc.is_empty() {
                let mut i = start;
                while i + wc.len() <= end {
                    if chars[i..i + wc.len()] == wc[..]
                        && (i == 0 || !crate::core::editor::is_word_char(chars[i - 1]))
                        && (i + wc.len() >= chars.len() || !crate::core::editor::is_word_char(chars[i + wc.len()]))
                    {
                        let xa = xs[i] - x0;
                        let xb = xs[i + wc.len()] - x0;
                        text_painter.rect_stroke(
                            Rect::from_min_size(Pos2::new(sx(xa, xb - xa), y), Vec2::new(xb - xa, lay.row_h)),
                            CornerRadius::same(2),
                            Stroke::new(1.0, Color32::from_rgb(230, 160, 40)),
                            StrokeKind::Inside,
                        );
                        i += wc.len();
                    } else {
                        i += 1;
                    }
                }
            }
        }
        // 對應括號
        if let Some((a, b)) = braces {
            for p in [a, b] {
                if p.line == line && p.col >= start && p.col < end {
                    let xa = xs[p.col] - x0;
                    let w = xs[p.col + 1] - xs[p.col];
                    text_painter.rect_filled(
                        Rect::from_min_size(Pos2::new(sx(xa, w), y), Vec2::new(w, lay.row_h)),
                        CornerRadius::same(2),
                        Color32::from_rgba_unmultiplied(80, 160, 255, 70),
                    );
                }
            }
        }

        // 文字：依語法色彩分段；等寬模式下字寬與格網相符的連續字元合併繪製（比例模式逐字繪製）
        let mut run = String::new();
        let mut run_x = 0.0f32;
        let mut run_style = default_style;
        let flush = |run: &mut String, run_x: f32, style: GlyphStyle| {
            if !run.is_empty() {
                paint_syntax_text(&text_painter, Pos2::new(sx(run_x, 0.0), text_y), std::mem::take(run), lay.font.clone(), style);
            }
        };
        let view_left = view.scroll_x - widths.cell_w * 4.0;
        let view_right = view.scroll_x + text_rect.width() + widths.cell_w * 4.0;
        for i in start..end {
            let c = chars[i];
            let xa = xs[i] - x0;
            let w = xs[i + 1] - xs[i];
            if xa + w < view_left || xa > view_right {
                flush(&mut run, run_x, run_style);
                continue;
            }
            let style = style_at(i);
            if c == '\t' || c == ' ' || c == '\u{3000}' {
                flush(&mut run, run_x, run_style);
                let show = if c == '\t' { settings.show_tab } else { settings.show_space };
                if show {
                    let sym = if c == '\t' { "»" } else { "·" };
                    let gw = widths.get(sym.chars().next().unwrap());
                    let px = sx(xa, w) + if lay.rtl { (w - gw).max(0.0) } else { 0.0 };
                    let px = if c == '\t' { px } else { px + ((w - gw) / 2.0).max(0.0) };
                    text_painter.text(Pos2::new(px, text_y), Align2::LEFT_TOP, sym, lay.font.clone(), control_color);
                }
                continue;
            }
            if c.is_control() {
                flush(&mut run, run_x, run_style);
                let label = format!("{:02X}", c as u32);
                let r = Rect::from_min_size(Pos2::new(sx(xa, w), y + 1.0), Vec2::new(w.max(widths.cell_w), lay.row_h - 2.0));
                text_painter.rect_filled(r, CornerRadius::same(2), control_color.gamma_multiply(0.6));
                text_painter.text(r.center(), Align2::CENTER_CENTER, label, FontId::monospace(widths.text_h * 0.5), bg);
                continue;
            }
            let gw = widths.get(c);
            let grid_ok = (gw - w).abs() < 0.05;
            // Proportional layout draws every glyph at its own x. egui lays a run out with pair
            // kerning, which `xs` (summed advances, shared with the caret, selection and
            // wrapping) does not have, so a long run drifted from them: 11 px over a 360-char
            // base64 line in Noto Sans TC, showing as a gap before the next run.
            if lay.rtl || !grid_ok || !lay.fixed {
                flush(&mut run, run_x, run_style);
                // 字寬與格子不同（例如 CJK 後備字型）：在格子內置中
                let px = sx(xa, w) + ((w - gw) / 2.0).max(0.0);
                paint_syntax_text(&text_painter, Pos2::new(px, text_y), c.to_string(), lay.font.clone(), style);
                continue;
            }
            // 相鄰的 ASCII 符號分開繪製，避免程式字型把 -> => != 等連字成單一圖形
            let ligature_risk = c.is_ascii_punctuation() && run.chars().last().is_some_and(|p| p.is_ascii_punctuation());
            if run.is_empty() || style != run_style || ligature_risk {
                flush(&mut run, run_x, run_style);
                run_x = xa;
                run_style = style;
            }
            run.push(c);
        }
        flush(&mut run, run_x, run_style);

        // 行尾符號
        if settings.show_eol && end == char_count && line + 1 < ed.lines.len() {
            let px = sx(row_w, widths.get('↵'));
            text_painter.text(Pos2::new(px, text_y), Align2::LEFT_TOP, "↵", lay.font.clone(), eol_color);
        }

        // 拼字錯誤底線
        for &(a, b) in &spell_ranges {
            {
                let (a, b) = (a.max(start), b.min(end));
                if a < b {
                    let xa = xs[a] - x0;
                    let xb = xs[b] - x0;
                    let left = sx(xa, xb - xa);
                    let base = text_y + widths.text_h - 1.0;
                    let mut pts = Vec::new();
                    let mut x = left;
                    let mut up = true;
                    while x < left + (xb - xa) {
                        pts.push(Pos2::new(x, if up { base - 1.5 } else { base + 0.5 }));
                        x += 2.0;
                        up = !up;
                    }
                    if pts.len() > 1 {
                        text_painter.add(egui::Shape::line(pts, Stroke::new(1.0, Color32::from_rgb(230, 50, 50))));
                    }
                }
            }
        }

        // 行號與書籤（只畫在每行第一個視覺列）
        if start == 0 {
            if settings.show_line_numbers {
                let num_color = if line == sel.head.line { fg } else { faint };
                painter.text(
                    Pos2::new(full.left() + mark_w + num_w - 6.0, text_y),
                    Align2::RIGHT_TOP,
                    (line + 1).to_string(),
                    lay.font.clone(),
                    num_color,
                );
            }
            if settings.show_bookmarks && ed.bookmarks.contains(&line) {
                painter.circle_filled(Pos2::new(full.left() + mark_w / 2.0 + 1.0, y + lay.row_h / 2.0), (lay.row_h * 0.22).min(5.0), Color32::from_rgb(60, 120, 230));
            }
        }

        // 游標
        if row == head_row && focused {
            let caret_x_rel = if let Some(v) = sel.head_vcol.filter(|_| sel.is_block()) {
                v as f32 * widths.cell_w
            } else {
                xs[sel.head.col.min(xs.len() - 1)] - x0
            };
            let mut caret_x = sx(caret_x_rel, 0.0);
            if let Some(pre) = &view.preedit {
                // 輸入法組字：在游標處以底線標示組字中的文字
                let w: f32 = pre.chars().map(|c| widths.get(c)).sum();
                let r = Rect::from_min_size(Pos2::new(caret_x, y), Vec2::new(w, lay.row_h));
                text_painter.rect_filled(r, CornerRadius::ZERO, bg);
                text_painter.text(Pos2::new(caret_x, text_y), Align2::LEFT_TOP, pre, lay.font.clone(), fg);
                text_painter.hline(r.x_range(), text_y + widths.text_h, Stroke::new(1.0, fg));
                caret_x += w;
            }
            let time = ctx.input(|i| i.time);
            let blink_on = ((time - view.blink_start) % 1.0) < 0.6;
            let caret_rect = if ed.overwrite_mode && view.preedit.is_none() {
                let w = xs.get(sel.head.col + 1).map(|x| x - xs[sel.head.col]).unwrap_or(widths.cell_w);
                Rect::from_min_size(Pos2::new(caret_x, y + lay.row_h - 3.0), Vec2::new(w, 2.0))
            } else {
                Rect::from_min_size(Pos2::new(caret_x - 0.5, y), Vec2::new(2.0, lay.row_h))
            };
            if blink_on || view.preedit.is_some() {
                // 列塊矩形選取時，於每一行畫出游標
                if let Some(b) = block_bounds.filter(|b| b.min_vcol == b.max_vcol) {
                    for l in b.min_line..=b.max_line {
                        if l >= first_line && l <= last_line {
                            let ry = text_rect.top() + (l - first_row) as f32 * lay.row_h;
                            text_painter.rect_filled(Rect::from_min_size(Pos2::new(caret_rect.left(), ry), caret_rect.size()), CornerRadius::ZERO, fg);
                        }
                    }
                } else {
                    text_painter.rect_filled(caret_rect, CornerRadius::ZERO, fg);
                }
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(300));
            // 輸入法候選窗位置
            ctx.output_mut(|o| {
                o.ime = Some(egui::output::IMEOutput { purpose: egui::IMEPurpose::Normal,
                    rect: text_rect,
                    cursor_rect: Rect::from_min_size(Pos2::new(caret_x, y), Vec2::new(2.0, lay.row_h)),
                    should_interrupt_composition: false,
                })
            });
        }
        };
        // 這一行的每個可見視覺列
        while row_iter < last_row && view.row_line(ed, row_iter) == line {
            let row = row_iter;
            row_iter += 1;
            let start = view.row_start(ed, row);
            let end = if row_iter < last_row && view.row_line(ed, row_iter) == line {
                view.row_start(ed, row_iter)
            } else {
                char_count
            };
            draw_row(row, start, end);
        }
    }
    if focused && (head_row < first_row || head_row >= last_row) {
        // 游標不在可見範圍時仍啟用輸入法
        ctx.output_mut(|o| {
            o.ime = Some(egui::output::IMEOutput { purpose: egui::IMEPurpose::Normal, rect: text_rect, cursor_rect: Rect::from_min_size(text_rect.min, Vec2::new(2.0, lay.row_h)), should_interrupt_composition: false })
        });
    }

    // 欄位提示線
    if settings.show_col_hint && !lay.rtl {
        let x = text_rect.left() - view.scroll_x + settings.col_hint_position as f32 * widths.cell_w;
        if text_rect.x_range().contains(x) {
            text_painter.vline(x, text_rect.y_range(), Stroke::new(1.0, faint));
        }
    }

    // 尺標
    if settings.show_ruler {
        let r = Rect::from_min_max(Pos2::new(full.left(), full.top()), Pos2::new(text_rect.right(), text_rect.top()));
        painter.rect_filled(r, CornerRadius::ZERO, gutter_bg);
        painter.hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, faint));
        let rp = painter.with_clip_rect(Rect::from_min_max(Pos2::new(text_rect.left(), r.top()), r.max));
        let first_col = (view.scroll_x / widths.cell_w).floor() as usize;
        let cols = (text_rect.width() / widths.cell_w).ceil() as usize + 1;
        let small = FontId::proportional((ruler_h * 0.6).max(8.0));
        for col in first_col..first_col + cols {
            let x = if lay.rtl {
                text_rect.right() + view.scroll_x - col as f32 * widths.cell_w
            } else {
                text_rect.left() - view.scroll_x + col as f32 * widths.cell_w
            };
            let h = if col % 10 == 0 { ruler_h * 0.5 } else if col % 5 == 0 { ruler_h * 0.3 } else { ruler_h * 0.15 };
            rp.vline(x, (r.bottom() - h)..=r.bottom(), Stroke::new(1.0, faint));
            if col % 10 == 0 {
                rp.text(Pos2::new(x + 2.0, r.top()), Align2::LEFT_TOP, col.to_string(), small.clone(), faint);
            }
        }
        // 游標欄位標記
        let caret_v = sel.head_vcol.filter(|_| sel.is_block()).unwrap_or_else(|| char_col_to_visual_col(&ed.lines[sel.head.line], sel.head.col));
        let cx = text_rect.left() - view.scroll_x + caret_v as f32 * widths.cell_w;
        if !lay.rtl {
            rp.vline(cx, r.y_range(), Stroke::new(2.0, Color32::from_rgb(220, 80, 60)));
        }
    }

    // 捲軸繪製
    let track_color = visuals.extreme_bg_color;
    let thumb_color = visuals.widgets.inactive.bg_fill;
    painter.rect_filled(vbar, CornerRadius::ZERO, track_color);
    painter.rect_filled(v_thumb, CornerRadius::same(3), if vbar_resp.dragged() { visuals.widgets.active.bg_fill } else { thumb_color });

    view.content_width = view.content_width.max(max_w + widths.cell_w * 2.0);
    painter.rect_filled(hbar, CornerRadius::ZERO, track_color);
    if view.wrap.key.is_none() {
        let total_w = view.content_width.max(view.scroll_x + text_rect.width());
        let h_track = hbar.shrink2(Vec2::new(2.0, 2.0));
        let thumb_w = (h_track.width() * text_rect.width() / total_w).clamp(20.0, h_track.width());
        let max_scroll = (total_w - text_rect.width()).max(0.0);
        let frac = if max_scroll <= 0.0 { 0.0 } else { view.scroll_x / max_scroll };
        let h_thumb = Rect::from_min_size(
            Pos2::new(h_track.left() + (h_track.width() - thumb_w) * frac, h_track.top()),
            Vec2::new(thumb_w, h_track.height()),
        );
        if let Some(p) = hbar_resp.interact_pointer_pos() {
            if hbar_resp.drag_started() {
                view.drag = Some(Drag::HScroll { grab: if h_thumb.contains(p) { p.x - h_thumb.left() } else { thumb_w / 2.0 } });
            }
            if let Some(Drag::HScroll { grab }) = view.drag {
                let f = ((p.x - grab - h_track.left()) / (h_track.width() - thumb_w).max(1.0)).clamp(0.0, 1.0);
                view.scroll_x = f * max_scroll;
            }
        }
        if hbar_resp.drag_stopped() {
            view.drag = None;
        }
        painter.rect_filled(h_thumb, CornerRadius::same(3), thumb_color);
    }

    // 右鍵選單：拼字建議與編輯命令
    let spell_ctx = &mut view.spell_ctx;
    let mut replace_word: Option<(usize, usize, usize, String)> = None;
    let lang = i18n::resolve(settings.ui_language);
    let spell_none = i18n::translate(lang, "spell.none", "（沒有建議）");
    let spell_add = i18n::translate(lang, "spell.add", "將「{q}」加入字典");
    response.context_menu(|ui| {
        ui.set_min_width(menus::SUBMENU_MIN_WIDTH);
        if let Some(sc) = spell_ctx.as_ref() {
            if sc.suggestions.is_empty() {
                ui.label(spell_none);
            }
            for s in &sc.suggestions {
                if ui.button(menus::check_label(false, s)).clicked() {
                    replace_word = Some((sc.line, sc.start, sc.end, s.clone()));
                    ui.close();
                }
            }
            if ui.button(menus::check_label(false, &i18n::fill(spell_add, None, Some(&sc.word)))).clicked() {
                out.add_word = Some(sc.word.clone());
                ui.close();
            }
            ui.separator();
        }
        for c in [C::Undo, C::Redo, C::Cut, C::Copy, C::Paste, C::Delete, C::SelectAll] {
            if ui.button(menus::check_label(false, &i18n::command_text(lang, c))).clicked() {
                out.commands.push(c);
                ui.close();
            }
        }
        ui.separator();
        for c in [C::ToUpperCase, C::ToLowerCase, C::Comment, C::Uncomment] {
            if ui.button(menus::check_label(false, &i18n::command_text(lang, c))).clicked() {
                out.commands.push(c);
                ui.close();
            }
        }
    });
    if let Some((line, s, e, word)) = replace_word {
        let hint = line..line + 1;
        doc.edit(EditKind::Other, hint, |ed| {
            ed.select_range(TextPosition::new(line, s), TextPosition::new(line, e));
            ed.insert_text(&word);
        });
        view.clear_frame_layout();
        view.spell_cache.clear();
    }
    if out.add_word.is_some() {
        view.spell_cache.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widths() -> CharWidths {
        let mut w = CharWidths::default();
        w.key = (f32::to_bits(14.0), "test".to_string(), 0);
        w.cell_w = 10.0;
        w.space_w = 10.0;
        w.text_h = 14.0;
        w
    }

    fn layout(wrap: WrapMode) -> Layout {
        let mut lay = Layout::new(&Settings::default(), EditMode::Text);
        lay.wrap = wrap;
        lay.wrap_width = 100.0;
        lay
    }

    /// A character missing from the sampled set must get its real width, not the cell width:
    /// the line layout and wrap caches keep whatever width they were built with.
    #[test]
    fn unsampled_char_is_measured_not_guessed() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            let font = FontId::proportional(14.0);
            let mut w = CharWidths::default();
            w.prepare(ui.ctx(), &font, "proportional", "0".chars());
            let real = ui.ctx().fonts_mut(|f| f.glyph_width(&font, 'm'));
            assert!((real - w.cell_w).abs() > 0.5, "test font must be proportional");
            assert!((w.get('m') - real).abs() < 0.01, "'m' measured {} instead of {real}", w.get('m'));
            let xs = line_xs("mm", false, &w);
            assert!((xs[2] - 2.0 * real).abs() < 0.01);
        });
        // The font atlas upload is not rendered anywhere in this test.
        out.textures_delta.clear();
    }

    #[test]
    fn content_hash_detects_content_changes() {
        assert_eq!(content_hash("hello"), content_hash("hello"));
        assert_ne!(content_hash("hello"), content_hash("hellp"));
        // 長度相同但內容不同、以及長度不同的情況都要區分
        assert_ne!(content_hash("abc"), content_hash("abd"));
        assert_ne!(content_hash("abc"), content_hash("abcd"));
    }

    #[test]
    fn wrap_cache_matches_full_rebuild_after_edit() {
        let long = "x".repeat(5000);
        let mut doc = Document::default();
        doc.editor = EditorState::from_text(&format!("hello world\n{long}\ntiny\n"));
        let mut view = ViewState::default();
        let lay = layout(WrapMode::ByWindow);
        let w = widths();
        view.update_wrap(&doc, &lay, &w);
        let rows_before = view.wrap.rows.clone();

        // 只編輯中間的超長行：其他行的換行結果應沿用，整體結果與全部重算一致
        let hint = 1..2;
        doc.edit(crate::core::document::EditKind::Typing, hint, |ed| {
            ed.lines[1] = "y".repeat(300);
        });
        view.update_wrap(&doc, &lay, &w);

        let mut fresh = ViewState::default();
        fresh.update_wrap(&doc, &lay, &w);
        assert_eq!(view.wrap.rows, fresh.wrap.rows, "增量重算必須與全部重算一致");
        assert_ne!(view.wrap.rows, rows_before, "編輯的行換行結果必須更新");
        assert_eq!(view.row_count(&doc.editor), fresh.row_count(&doc.editor));
    }

    #[test]
    fn wrap_cache_tracks_line_count_changes() {
        let long = "z".repeat(2000);
        let mut doc = Document::default();
        doc.editor = EditorState::from_text(&format!("a\n{long}"));
        let mut view = ViewState::default();
        let lay = layout(WrapMode::ByWindow);
        let w = widths();
        view.update_wrap(&doc, &lay, &w);
        let rows_two_lines = view.row_count(&doc.editor);

        doc.edit(crate::core::document::EditKind::Other, 0..1, |ed| {
            ed.insert_text("b\n");
        });
        view.update_wrap(&doc, &lay, &w);
        assert!(view.row_count(&doc.editor) > rows_two_lines, "插入新行後視覺列必須增加");
        assert_eq!(view.wrap.line_first_row.len(), doc.editor.lines.len());
        // 第一行仍從第 0 列開始
        assert_eq!(view.wrap.rows[0], (0, 0));
    }

    #[test]
    fn no_wrap_bypasses_wrap_cache() {
        let mut doc = Document::default();
        doc.editor = EditorState::from_text(&"x".repeat(3000));
        let mut view = ViewState::default();
        let lay = layout(WrapMode::NoWrap);
        let w = widths();
        view.update_wrap(&doc, &lay, &w);
        assert!(!view.wrapped());
        assert_eq!(view.row_count(&doc.editor), 1);
        assert!(view.wrap_breaks(0).is_empty());
    }

    #[test]
    fn caret_x_across_wrapped_rows_stays_in_row() {
        // 每列 10 字（wrap_width = 100, cell_w = 10）：2000 字共 200 列，第 2001 列是短行
        let mut doc = Document::default();
        doc.editor = EditorState::from_text(&format!("{}\nab", "x".repeat(2000)));
        let mut view = ViewState::default();
        let lay = layout(WrapMode::ByWindow);
        let w = widths();
        view.update_wrap(&doc, &lay, &w);
        assert_eq!(view.row_count(&doc.editor), 201);
        let pos = TextPosition::new(0, 2000);
        let row = view.row_of(pos);
        assert_eq!(row, 199, "行尾在最後一個視覺列");
        let x = view.x_of(&doc.editor, pos, &lay, &w);
        assert!((x - 100.0).abs() < 0.01, "最後一列有 10 個字，游標 x 應為 100，實際 {x}");
        let down = view.pos_at_x(&doc.editor, row + 1, x, &lay, &w);
        assert_eq!(down, TextPosition::new(1, 2), "帶著記憶欄往下應落在短行行尾");
    }
}
