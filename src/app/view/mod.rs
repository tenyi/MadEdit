//! 自繪文字編輯區：排版（等寬格網或比例字寬、自動換行）、繪製、滑鼠鍵盤與輸入法
//!
//! 文字內容與游標的唯一真值在 `Document.editor`；本模組只保存捲動位置、換行快取等檢視狀態。

use std::collections::HashMap;

use eframe::egui::{self, FontId, Pos2, Rect, Sense};
use serde::{Deserialize, Serialize};

use crate::app::commands::C;
#[cfg(test)]
use crate::core::content_hash;
use crate::core::document::Document;
#[cfg(test)]
use crate::core::editor::EditorState;
use crate::core::editor::{TextPosition, char_visual_columns, tab_columns_at, tab_size};
#[cfg(test)]
use crate::core::model::EditMode;
use crate::core::settings::{Settings, WrapMode};
use crate::core::spell::SpellChecker;

mod input;
mod layout;
mod paint;

pub use input::{apply_action, indent_selection, selected_line_range};

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

// ── 編輯動作（亦供巨集錄製與播放） ───────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EdAction {
    Insert(String),
    Newline {
        indent: bool,
    },
    Tab,
    BackTab,
    Backspace,
    Delete,
    DeleteWord {
        forward: bool,
    },
    /// macOS 的 ⌘Delete：刪除至行首／行尾，既有選取優先刪除。
    DeleteLineEdge {
        forward: bool,
    },
    Left {
        extend: bool,
        word: bool,
    },
    Right {
        extend: bool,
        word: bool,
    },
    Up {
        extend: bool,
    },
    Down {
        extend: bool,
    },
    PageUp {
        extend: bool,
    },
    PageDown {
        extend: bool,
    },
    Home {
        extend: bool,
    },
    LineStart {
        extend: bool,
    },
    End {
        extend: bool,
    },
    DocStart {
        extend: bool,
    },
    DocEnd {
        extend: bool,
    },
    /// Alt+Shift+方向鍵：延伸矩形選取
    Block {
        dx: i32,
        dy: i32,
    },
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

// ── 繪製與互動 ──────────────────────────────────────────────────

pub struct ViewEnv<'a> {
    pub settings: &'a Settings,
    pub widths: &'a mut CharWidths,
    pub spell: Option<&'a SpellChecker>,
    pub theme_name: &'a str,
    pub is_dark: bool,
    pub editor_family: &'a str,
}

/// 編輯區的固定 Id（App 用來判斷焦點與要求焦點）
pub fn editor_id(doc_id: u64) -> egui::Id {
    egui::Id::unique(("madedit-editor", doc_id))
}
/// 一行文字每個字元的起始 x（相對行首），長度為字元數 + 1（最後一項為行寬）
pub fn line_xs(line: &str, fixed: bool, widths: &CharWidths) -> Vec<f32> {
    let mut xs = Vec::with_capacity(line.len() + 1);
    let mut x = 0.0f32;
    let mut vcol = 0usize;
    for c in line.chars() {
        xs.push(x);
        if fixed {
            vcol += if c == '\t' {
                tab_columns_at(vcol)
            } else {
                char_visual_columns(c)
            };
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

/// Geometry and egui responses allocated once, shared by input and painting.
struct Frame {
    lay: Layout,
    full: Rect,
    text_rect: Rect,
    gutter_rect: Rect,
    vbar: Rect,
    hbar: Rect,
    mark_w: f32,
    num_w: f32,
    ruler_h: f32,
    id: egui::Id,
    response: egui::Response,
    gutter_resp: egui::Response,
    vbar_resp: egui::Response,
    hbar_resp: egui::Response,
}

struct HorizontalBar {
    track: Rect,
    thumb: Rect,
    max_scroll: f32,
}

/// 顯示文字編輯區
pub fn show(
    ui: &mut egui::Ui,
    doc: &mut Document,
    view: &mut ViewState,
    env: &mut ViewEnv,
    out: &mut ViewOutput,
) {
    let settings = env.settings;
    let mut lay = Layout::new(settings, doc.mode);
    let ctx = ui.ctx().clone();

    // 量測可見範圍的字元（含行號與控制字元符號）；取樣有上限，避免超長行每幀掃整份文件
    let first_guess = view.top_row.min(doc.editor.lines.len().saturating_sub(1));
    {
        let mut budget = 4096usize;
        let mut sample: Vec<char> = Vec::new();
        for line in doc
            .editor
            .lines
            .iter()
            .skip(first_guess.saturating_sub(5))
            .take(200)
        {
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
        env.widths.prepare(
            &ctx,
            &lay.font,
            env.editor_family,
            sample.into_iter().chain(extra).chain(pre.chars()),
        );
    }
    let widths: &CharWidths = env.widths;
    lay.row_h = (widths.text_h * settings.line_spacing.clamp(100, 250) as f32 / 100.0)
        .round()
        .max(1.0);

    // ── 版面配置：尺標、行號欄、文字區、捲軸 ──
    let full = ui.available_rect_before_wrap();
    let sb = 14.0;
    let ruler_h = if settings.show_ruler {
        (widths.text_h * 0.8).max(14.0)
    } else {
        0.0
    };
    let line_count = doc.editor.lines.len();
    let digits = line_count.to_string().len().max(3);
    let num_w = if settings.show_line_numbers {
        digits as f32 * widths.cell_w + 10.0
    } else {
        0.0
    };
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
    let gutter_rect = Rect::from_min_max(
        Pos2::new(full.left(), text_rect.top()),
        Pos2::new(text_rect.left(), text_rect.bottom()),
    );
    let gutter_resp = ui.interact(gutter_rect, id.with("gutter"), Sense::click_and_drag());
    let vbar = Rect::from_min_max(
        Pos2::new(text_rect.right(), full.top()),
        Pos2::new(full.right(), text_rect.bottom()),
    );
    let hbar = Rect::from_min_max(
        Pos2::new(full.left(), text_rect.bottom()),
        Pos2::new(text_rect.right(), full.bottom()),
    );
    let vbar_resp = ui.interact(vbar, id.with("vbar"), Sense::click_and_drag());
    let hbar_resp = ui.interact(hbar, id.with("hbar"), Sense::click_and_drag());
    let frame = Frame {
        lay,
        full,
        text_rect,
        gutter_rect,
        vbar,
        hbar,
        mark_w,
        num_w,
        ruler_h,
        id,
        response,
        gutter_resp,
        vbar_resp,
        hbar_resp,
    };
    let v_thumb = input::interact(ui, doc, view, env, &frame, out);
    let max_w = paint::text(ui, doc, view, env, &frame, out.focused);
    view.content_width = view.content_width.max(max_w + env.widths.cell_w * 2.0);
    let horizontal = layout::horizontal_bar(view, &frame);
    input::horizontal_drag(view, &frame, horizontal.as_ref());
    paint::scrollbars(ui, &frame, v_thumb, horizontal.as_ref());
    input::context_menu(doc, view, settings, &frame.response, out);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widths() -> CharWidths {
        CharWidths {
            key: (f32::to_bits(14.0), "test".to_string(), 0),
            cell_w: 10.0,
            space_w: 10.0,
            text_h: 14.0,
            ..Default::default()
        }
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
            assert!(
                (real - w.cell_w).abs() > 0.5,
                "test font must be proportional"
            );
            assert!(
                (w.get('m') - real).abs() < 0.01,
                "'m' measured {} instead of {real}",
                w.get('m')
            );
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
        assert_eq!(
            view.wrap.rows, fresh.wrap.rows,
            "增量重算必須與全部重算一致"
        );
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
        assert!(
            view.row_count(&doc.editor) > rows_two_lines,
            "插入新行後視覺列必須增加"
        );
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
        assert!(
            (x - 100.0).abs() < 0.01,
            "最後一列有 10 個字，游標 x 應為 100，實際 {x}"
        );
        let down = view.pos_at_x(&doc.editor, row + 1, x, &lay, &w);
        assert_eq!(
            down,
            TextPosition::new(1, 2),
            "帶著記憶欄往下應落在短行行尾"
        );
    }
}
