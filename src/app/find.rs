//! 搜尋／取代列與跳行列：文字模式以行為單位比對（Regex），十六進位模式比對位元組

use std::hash::{Hash, Hasher};

use eframe::egui::{self, Key, RichText, Role, WidgetInfo};
use regex::Regex;

use super::commands::{plain_label, C};
use super::{form, App, Confirm};
use crate::core::document::EditKind;
use crate::core::editor::TextPosition;
use crate::core::encoding;
use crate::core::model::EditMode;
use crate::core::search::{self, SearchError, SearchOptions};

/// 「只在選取範圍內」缺少一般選取範圍時的提示（繁體中文原文；譯文鍵 `find.in_selection_need_range`）
const NEED_RANGE: &str = "「只在選取範圍內」需要先選取文字（不支援欄位選取）";

#[derive(Default)]
pub struct FindBar {
    pub visible: bool,
    pub replace: bool,
    pub query: String,
    pub replacement: String,
    /// 以十六進位位元組搜尋
    pub hex: bool,
    pub focus: bool,
    pub info: Option<String>,
    /// 搜尋／取代歷史下拉是否展開
    pub history_open: bool,
    pub replace_history_open: bool,
    /// 標題欄快速搜尋框的歷史下拉是否展開
    pub quick_history_open: bool,
    /// 「選項」齒輪展開的底部選項區
    pub options_open: bool,
    /// 記憶位置是否已在本輪彈出時應用過（只應用一次，否則會和拖動互相搶位置）
    pub pos_applied: bool,
    pub(crate) live: LiveSearch,
}

/// 歷史下拉的選擇結果
pub(super) enum HistAction {
    None,
    /// 選了某一筆歷史
    Pick(String),
    /// 移除某一筆歷史
    Remove(String),
    /// 清空全部歷史
    Clear,
}

/// `text` on one line for display, with line breaks written as `\n`: a history entry or a
/// query inside a message must never break the row it is drawn on.
pub(super) fn shown_one_line(text: &str) -> String {
    text.replace("\r\n", "\\n").replace('\n', "\\n").replace('\r', "\\r")
}

/// A query for a status message: on one line, and no longer than a few dozen characters.
fn short_query(query: &str) -> String {
    const MAX_CHARS: usize = 60;
    let flat = shown_one_line(query);
    if flat.chars().count() <= MAX_CHARS {
        return flat;
    }
    flat.chars().take(MAX_CHARS).chain(std::iter::once('…')).collect()
}

/// 歷史下拉彈層：錨定觸發鈕（標籤列右端的時鐘鈕，仿 UE）向下展開；
/// 每列右端 hover 紅色「×」移除單筆，底部「清空」。選取、移除或清空後自動收起。
pub(super) fn history_dropdown(
    ui: &egui::Ui,
    trigger: &egui::Response,
    open: &mut bool,
    entries: &[String],
    remove_label: &str,
    clear_label: &str,
) -> HistAction {
    history_popup(ui, trigger, open, None, HISTORY_WIDTH, entries, remove_label, clear_label)
}

/// 單行輸入框（標題欄快速查找）的歷史下拉：細線 chevron 疊在框內右端，彈層與框同寬。
pub(super) fn history_chevron_dropdown(
    ui: &mut egui::Ui,
    id_salt: &str,
    field: egui::Rect,
    open: &mut bool,
    entries: &[String],
    tooltip: &str,
    remove_label: &str,
    clear_label: &str,
) -> HistAction {
    use egui::{Pos2, Shape, Stroke};
    let chev_rect = egui::Rect::from_center_size(egui::pos2(field.right() - 14.0, field.center().y), egui::vec2(22.0, field.height()));
    let resp = ui.interact(chev_rect, ui.scope_id().with(id_salt), egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    let hovered = resp.hovered();
    let clicked = resp.clicked();
    resp.clone().widget_info(|| WidgetInfo::labeled(Role::Button, true, tooltip));
    resp.clone().on_hover_text(tooltip);
    let weak = ui.visuals().widgets.noninteractive.text_color();
    let strong = ui.visuals().text_color();
    let stroke = Stroke::new(1.5, if hovered { strong } else { weak });
    let (cx, cy) = (chev_rect.center().x, chev_rect.center().y);
    ui.painter().add(Shape::line_segment([Pos2::new(cx - 4.0, cy - 2.0), Pos2::new(cx, cy + 2.0)], stroke));
    ui.painter().add(Shape::line_segment([Pos2::new(cx, cy + 2.0), Pos2::new(cx + 4.0, cy - 2.0)], stroke));
    if clicked {
        *open = !*open;
    }
    history_popup(ui, &resp, open, Some(field), field.width() - 8.0, entries, remove_label, clear_label)
}

/// The popup of a history dropdown, anchored to `trigger` and `width` points wide; with an
/// `anchor` field it also takes that field's width limit and a compact frame. A history
/// without entries never opens, and any action closes it.
fn history_popup(
    ui: &egui::Ui,
    trigger: &egui::Response,
    open: &mut bool,
    anchor: Option<egui::Rect>,
    width: f32,
    entries: &[String],
    remove_label: &str,
    clear_label: &str,
) -> HistAction {
    if !*open || entries.is_empty() {
        if entries.is_empty() {
            *open = false;
        }
        return HistAction::None;
    }
    let mut popup = egui::Popup::from_toggle_button_response(trigger).open_bool(open).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    if let Some(field) = anchor {
        popup = popup.anchor(field).frame(egui::Frame::popup(&ui.ctx().global_style()).inner_margin(egui::Margin::symmetric(4, 5)));
    }
    let action = popup
        .show(|ui| {
            ui.set_min_width(width);
            if anchor.is_some() {
                ui.set_max_width(width);
            }
            history_list(ui, width, entries, remove_label, clear_label)
        })
        .map(|r| r.inner)
        .unwrap_or(HistAction::None);
    if !matches!(action, HistAction::None) {
        *open = false;
    }
    action
}

/// The rows of a history dropdown, `width` points wide: one row per entry, a red "×" at the
/// right end while the row is hovered to remove that entry, and a "clear" row at the bottom.
/// 紅 × 樣式拷貝自收藏列表的移除鈕。
fn history_list(ui: &mut egui::Ui, width: f32, entries: &[String], remove_label: &str, clear_label: &str) -> HistAction {
    let mut action = HistAction::None;
    let body = egui::TextStyle::Body.resolve(ui.style());
    const CLOSE_W: f32 = 20.0;
    for entry in entries {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(width, HISTORY_ROW_HEIGHT), egui::Sense::click());
        response.widget_info(|| WidgetInfo::labeled(Role::Button, true, entry));
        // 指標在整行範圍內即整行 hover：文字與「×」一起亮（× 的熱區疊在行內，不能只看 response.hovered）
        let row_hovered = ui.rect_contains_pointer(rect);
        let mut removed = false;
        if ui.is_rect_visible(rect) {
            if row_hovered {
                ui.painter().rect_filled(rect, 4.0, ui.visuals().widgets.hovered.weak_bg_fill);
            }
            let shown = shown_one_line(entry);
            let mut layout = egui::text::LayoutJob {
                wrap: egui::text::TextWrapping::truncate_at_width((rect.width() - CLOSE_W - 16.0).max(0.0)),
                ..Default::default()
            };
            layout.append(&shown, 0.0, egui::TextFormat::simple(body.clone(), ui.visuals().text_color()));
            let galley = ui.painter().layout_job(layout);
            let gp = egui::pos2(rect.left() + 8.0, rect.center().y - galley.size().y / 2.0);
            ui.painter().galley(gp, galley, ui.visuals().text_color());
            if row_hovered {
                // 列尾獨立熱區的紅「×」：無按鈕底，只有字符變紅
                let x_rect = egui::Rect::from_min_size(egui::pos2(rect.right() - 4.0 - CLOSE_W, rect.top()), egui::vec2(CLOSE_W, rect.height()));
                let r2 = ui.interact(x_rect, response.id.with("remove"), egui::Sense::click());
                r2.widget_info(|| WidgetInfo::labeled(Role::Button, true, remove_label));
                removed = r2.clicked();
                r2.clone().on_hover_text(remove_label);
                let tint = if r2.hovered() { REMOVE_HOVER } else { ui.visuals().weak_text_color() };
                ui.painter().text(x_rect.center(), egui::Align2::CENTER_CENTER, "×", egui::FontId::proportional(13.0), tint);
            }
        }
        // 點「×」不算選中該列
        if removed {
            action = HistAction::Remove(entry.clone());
        } else if response.clicked() {
            action = HistAction::Pick(entry.clone());
        }
    }
    ui.separator();
    let clear = ui.add_sized([width, 20.0], egui::Button::selectable(false, RichText::new(clear_label).weak().size(body.size)));
    if clear.clicked() {
        action = HistAction::Clear;
    }
    action
}

// Sizes of the find window in points at 1x zoom. They carry its compact UltraEdit-style
// layout (title row, tab strip, 4-row boxes, stacked buttons); change them together.
/// Height of the title row the window is dragged by.
const TITLE_HEIGHT: f32 = 30.0;
/// Lines visible in the search and replace boxes; longer text scrolls inside the box.
const BOX_ROWS: usize = 4;
/// Size of one stacked action button under the boxes.
const BUTTON_SIZE: (f32, f32) = (104.0, 26.0);
/// Vertical gap between the stacked buttons.
const BUTTON_GAP: f32 = 4.0;
/// Space between the button column and the free area to its left.
const COLUMN_GAP: f32 = 8.0;
/// Width of the history dropdown under the clock buttons.
const HISTORY_WIDTH: f32 = 240.0;
const HISTORY_ROW_HEIGHT: f32 = 22.0;
/// Slightly smaller than body text: the labels, title and buttons of the window.
const SMALL_TEXT: f32 = 12.5;
/// The red of a "remove" glyph while it is hovered (the favorites list uses the same).
const REMOVE_HOVER: egui::Color32 = egui::Color32::from_rgb(232, 17, 35);

/// A title drawn "bold": the interface fonts ship a single weight, which leaves
/// `RichText::strong` without effect on Chinese, so the glyphs are drawn twice, 0.7 points apart.
fn paint_bold(painter: &egui::Painter, pos: egui::Pos2, galley: std::sync::Arc<egui::Galley>, color: egui::Color32) {
    painter.galley(pos, galley.clone(), color);
    painter.galley(pos + egui::vec2(0.7, 0.0), galley, color);
}

/// Focus a text box and select all of `text` in it, ready to be replaced by typing.
fn focus_select_all(ctx: &egui::Context, id: egui::Id, text: &str) {
    if let Some(mut state) = egui::TextEdit::load_state(ctx, id) {
        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(text.chars().count()))));
        state.store(ctx, id);
    }
}

/// One multiline box of the find window, as drawn by [`multiline_field`].
struct MultilineField {
    response: egui::Response,
    /// A plain Enter in the focused box: `true` with Shift. A plain Enter never breaks the
    /// line; Ctrl+Enter does, and the edit handles that itself.
    enter: Option<bool>,
}

/// One multiline box of the find window: a fixed-height scrolling text box (see
/// `form::scrolling_text_edit`) whose wheel steps the caret line by line and whose plain
/// Enter is reported instead of doing anything to the text.
fn multiline_field(ui: &mut egui::Ui, id_salt: &'static str, text: &mut String, hint: &str, invalid: bool) -> MultilineField {
    let id = ui.scope_id().with(id_salt);
    // Windows 慣例：Ctrl+Enter 才插入換行。換行交給輸入框自己的 return_key：egui 移動游標並
    // 把游標行捲入視野。繪製後手動插入曾讓視口留在原地、游標跑出視野。
    let edit = form::scrolling_text_edit(
        ui,
        id_salt,
        id,
        text,
        hint,
        BOX_ROWS,
        Some(egui::KeyboardShortcut::new(egui::Modifiers::CTRL, Key::Enter)),
    );
    form::field_border(ui, &edit, invalid);
    let resp = &edit.response;
    // 滾輪：游標整行步進（滾輪上＝往上、滾輪下＝往下；egui 自動把游標行捲入視野）
    let dy = ui.input(|i| i.smooth_scroll_delta.y);
    if dy != 0.0 && ui.rect_contains_pointer(edit.rect) && let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), resp.id) {
        let step = ((dy.abs() / ui.text_style_height(&egui::TextStyle::Body)).round() as usize).max(1);
        let dir: isize = if dy > 0.0 { -1 } else { 1 };
        let char_idx = st.cursor.char_range().map(|r| r.primary.index.0.min(r.secondary.index.0)).unwrap_or(0);
        let (cur_l, cur_c) = char_index_to_line_col(text, char_idx);
        let last_line = text.lines().count().saturating_sub(1);
        let new_l = (cur_l as isize + dir * step as isize).clamp(0, last_line as isize) as usize;
        let line_len = text.split('\n').nth(new_l).map(|l| l.chars().count()).unwrap_or(0);
        let new_idx = line_col_to_char_index(text, new_l, cur_c.min(line_len));
        st.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(new_idx))));
        st.store(ui.ctx(), resp.id);
    }
    // 只有持有焦點的輸入框回應純 Enter（替換頁兩個框同時顯示，各自處理自己的按鍵）；
    // 修飾鍵的判斷與輸入框的 return_key 相同，插入換行的那一下不算「純 Enter」。
    let enter = (resp.has_focus() && ui.input(|i| i.key_pressed(Key::Enter) && !i.modifiers.matches_logically(egui::Modifiers::CTRL)))
        .then(|| ui.input(|i| i.modifiers.shift_only()));
    MultilineField { response: edit.response, enter }
}

/// What the user pressed in the action row under the boxes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FindAction {
    None,
    Previous,
    Next,
    Replace,
    ReplaceAll,
}

/// One stacked button of the action row.
struct ActionButton<'a> {
    label: &'a str,
    /// Shown while hovering the enabled button; "" for none.
    tip: &'a str,
    action: FindAction,
    enabled: bool,
    /// Why it is unavailable, shown while hovering the disabled button; "" for none.
    disabled_tip: &'a str,
}

/// The "Options" gear at the bottom-left: it folds the option section out and in.
fn options_gear(ui: &mut egui::Ui, label: &str, options_open: &mut bool) {
    ui.horizontal(|ui| {
        if form::icon_button(ui, egui::Button::new("⚙").frame(false), label, label).clicked() {
            *options_open = !*options_open;
        }
    });
}

/// The row under the boxes: a column of stacked buttons at the right edge of the content and
/// the area to its left for `left` to fill. The column gets an explicit rectangle: a
/// right-to-left child layout would not see the remaining width and squeeze next to `left`.
fn action_row(ui: &mut egui::Ui, salt: &'static str, full: egui::Rect, buttons: &[ActionButton<'_>], left: impl FnOnce(&mut egui::Ui)) -> FindAction {
    let (w, h) = BUTTON_SIZE;
    let height = h * buttons.len() as f32 + BUTTON_GAP * buttons.len().saturating_sub(1) as f32;
    let row = egui::Rect::from_min_size(egui::pos2(full.left(), ui.cursor().top()), egui::vec2(full.width(), height));
    ui.allocate_exact_size(row.size(), egui::Sense::hover());
    let column = egui::Rect::from_min_max(egui::pos2(row.right() - w, row.top()), row.max);
    let free = egui::Rect::from_min_max(row.min, egui::pos2(column.left() - COLUMN_GAP, row.bottom()));
    let mut left_ui = ui.new_child(
        egui::UiBuilder::new().id_salt(format!("{salt}-left")).max_rect(free).layout(egui::Layout::top_down(egui::Align::LEFT)),
    );
    left(&mut left_ui);
    let mut col = ui.new_child(egui::UiBuilder::new().id_salt(salt).max_rect(column).layout(egui::Layout::top_down(egui::Align::Min)));
    col.spacing_mut().item_spacing.y = BUTTON_GAP;
    let mut pressed = FindAction::None;
    // add_sized 走置中布局，文字居中
    for button in buttons {
        let response = col
            .add_enabled_ui(button.enabled, |ui| ui.add_sized([w, h], egui::Button::new(RichText::new(button.label).size(SMALL_TEXT))))
            .inner;
        if response.clicked() {
            pressed = button.action;
        }
        if button.enabled {
            if !button.tip.is_empty() {
                response.on_hover_text(button.tip);
            }
        } else if !button.disabled_tip.is_empty() {
            response.on_disabled_hover_text(button.disabled_tip);
        }
    }
    pressed
}

/// The find window's live result: the regex and the per-line match counts belong to the pattern
/// and the document version, so moving the selection or pressing Find Next only relocates
/// `current` within the cached counts instead of rescanning the whole document.
#[derive(Default)]
pub(crate) struct LiveSearch {
    /// Changes with the document id, version and pattern; recomputes the counts when it does.
    pub(crate) counts_key: u64,
    /// Changes with the query and its options only, so the editor's match cache survives edits.
    pub(crate) pattern_id: u64,
    /// Match count for every line; empty for hex documents, an empty query or a bad pattern.
    pub(crate) counts: Vec<usize>,
    /// Cached sum of `counts`.
    pub(crate) total: usize,
    /// Which match (1-based) the selection covers exactly.
    pub(crate) current: Option<usize>,
    /// The selection the cached `current` was computed for.
    pub(crate) current_key: Option<u64>,
    pub(crate) regex: Option<Regex>,
    pub(crate) error: Option<SearchError>,
    /// The document joined into one text, for a pattern that crosses line breaks, with the key
    /// of the document version it was built for. It outlives query and option changes.
    pub(crate) joined: Option<(u64, search::JoinedText)>,
    /// Byte ranges of the cross-line matches in `joined`, in document order.
    pub(crate) spans: Vec<(usize, usize)>,
}

/// Identity of the search pattern: the query, the hex flag, the three search options and the
/// document encoding (hex queries are decoded through it).
fn pattern_key(query: &str, hex: bool, opts: (bool, bool, bool), encoding: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (query, hex, opts, encoding).hash(&mut h);
    h.finish()
}

/// Identity of the per-line counts: one document version of one document under one pattern.
fn counts_key(doc_id: u64, doc_version: u64, pattern_id: u64) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (doc_id, doc_version, pattern_id).hash(&mut h);
    h.finish()
}

/// Identity of the joined document: one version of one document, whatever the pattern.
fn joined_key(doc_id: u64, doc_version: u64) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (doc_id, doc_version).hash(&mut h);
    h.finish()
}

/// Identity of the selection endpoints.
fn selection_key(s: TextPosition, e: TextPosition) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    ((s.line, s.col), (e.line, e.col)).hash(&mut h);
    h.finish()
}

#[derive(Default)]
pub struct GotoBar {
    pub visible: bool,
    /// 依位置（位元組位移）跳轉，而非行號
    pub by_position: bool,
    pub input: String,
    pub focus: bool,
    pub error: Option<String>,
}

/// 多行文字裡的字元索引 →（行, 行內字元欄）
fn char_index_to_line_col(text: &str, char_idx: usize) -> (usize, usize) {
    let mut seen = 0usize;
    for (li, line) in text.split('\n').enumerate() {
        let len = line.chars().count();
        if char_idx <= seen + len {
            return (li, char_idx - seen);
        }
        seen += len + 1; // 換行符
    }
    let lines: Vec<&str> = text.split('\n').collect();
    (lines.len().saturating_sub(1), lines.last().map(|l| l.chars().count()).unwrap_or(0))
}

/// （行, 行內字元欄）→ 字元索引
fn line_col_to_char_index(text: &str, line: usize, col: usize) -> usize {
    let mut idx = 0usize;
    for (li, l) in text.split('\n').enumerate() {
        if li == line {
            return idx + col.min(l.chars().count());
        }
        idx += l.chars().count() + 1;
    }
    idx
}

/// Style for the controls inside the find / replace and go-to bars: outlined fields and
/// slightly taller controls than the compact toolbar, so the small option chips are easy targets.
fn bar_style(ui: &mut egui::Ui) {
    form::outline_fields(ui);
    let spacing = ui.spacing_mut();
    spacing.interact_size.y = 24.0;
    spacing.button_padding = egui::vec2(8.0, 3.0);
}

/// Rounded, outlined panel shared by the find / replace bar and the go-to bar.
fn bar_frame(ui: &egui::Ui) -> egui::Frame {
    let visuals = ui.visuals();
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(8, 6))
        .fill(visuals.faint_bg_color)
        .stroke(visuals.widgets.noninteractive.bg_stroke)
        .corner_radius(8.0)
}

/// A search failure as the interface shows it: a translated title, and the regex engine's
/// own reason on one line when there is one.
struct SearchErrorText {
    title: String,
    reason: Option<String>,
}

/// The regex engine's explanation on one line: its last non-empty message names the actual
/// problem (`error: unclosed group`), without the `error: ` prefix.
fn reason_line(msg: &str) -> String {
    msg.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or_default().trim_start_matches("error: ").to_string()
}

impl App {
    pub fn search_options(&self) -> SearchOptions {
        SearchOptions::new(self.settings.search_match_case, self.settings.search_whole_word, self.settings.search_regex)
    }

    /// 建立文字搜尋用的 Regex（十六進位查詢先依文件編碼解碼成文字）
    fn text_regex(&self) -> Result<Regex, SearchError> {
        let q = &self.find.query;
        if self.find.hex {
            let bytes = search::parse_hex_query(q)?;
            let d = encoding::decode_with(&bytes, self.doc().encoding);
            let opts = SearchOptions::new(true, false, false);
            return search::build_regex(&d.text, &opts);
        }
        search::build_regex(q, &self.search_options())
    }

    /// Whether the query is matched on the joined document instead of line by line.
    fn cross_line(&self) -> bool {
        !self.find.hex && search::pattern_may_span_lines(&self.find.query, self.settings.search_regex)
    }

    /// A search failure for display. `core::search` reports only the kind of failure; the
    /// text is chosen here, in the interface language.
    fn search_error(&self, e: &SearchError) -> SearchErrorText {
        let (key, zh) = match e {
            SearchError::EmptyQuery => ("find.err_empty_query", "搜尋關鍵字不可為空"),
            SearchError::PatternTooLarge => ("find.err_too_large", "搜尋文字過長，無法建立比對式"),
            SearchError::InvalidRegex(_) => ("find.err_bad_regex", "無效的正規表達式"),
            SearchError::EmptyHexQuery => ("find.err_hex_empty", "搜尋內容不可為空"),
            SearchError::HexDigit(_) => ("find.err_hex_char", "無效的十六進位字元：{q}"),
            SearchError::OddHexLength => ("find.err_hex_odd", "十六進位序列長度必須為偶數"),
        };
        let title = match e {
            SearchError::HexDigit(c) => self.tf(key, zh, None, Some(&c.to_string())),
            _ => self.t(key, zh).to_string(),
        };
        let reason = match e {
            SearchError::InvalidRegex(msg) => Some(reason_line(msg)),
            _ => None,
        };
        SearchErrorText { title, reason }
    }

    /// The same failure as one line, for the status bar and message boxes.
    pub fn search_error_line(&self, e: &SearchError) -> String {
        let text = self.search_error(e);
        match text.reason {
            Some(reason) if !reason.is_empty() => format!("{}{}{}", text.title, self.t("find.err_sep", "："), reason),
            _ => text.title,
        }
    }

    /// 找下一個或上一個；回傳是否找到
    pub fn find_next(&mut self, backward: bool) -> bool {
        if self.find.query.is_empty() {
            self.find.visible = true;
            self.find.focus = true;
            return false;
        }
        crate::core::settings::Settings::push_history(&mut self.settings.recent_finds, self.find.query.clone());
        if self.macros.recording {
            let step = super::macros::MacroStep::Find {
                query: self.find.query.clone(),
                hex: self.find.hex,
                match_case: self.settings.search_match_case,
                whole_word: self.settings.search_whole_word,
                regex: self.settings.search_regex,
                backward,
            };
            self.macros.record(step);
        }
        let wrap = self.settings.search_wrap_around;
        if self.doc().mode == EditMode::Hex {
            return self.find_bytes(backward, wrap);
        }
        let from = {
            let sel = self.doc().editor.selection;
            if backward { sel.start() } else { sel.end() }
        };
        // A pattern that can match a line break is matched on the joined document through the
        // live match index (see `update_live_search`); every other pattern line by line.
        let found = if self.cross_line() {
            self.update_live_search();
            if let Some(e) = self.find.live.error.clone() {
                let msg = self.search_error_line(&e);
                self.find.info = Some(msg.clone());
                self.set_status(msg);
                return false;
            }
            let live = &self.find.live;
            live.joined.as_ref().and_then(|(_, joined)| {
                let from_b = joined.pos_to_byte(&self.doc().editor.lines, from);
                search::find_in_spans(&live.spans, from_b, backward, wrap).map(|(s, e)| (joined.byte_to_pos(s), joined.byte_to_pos(e)))
            })
        } else {
            match self.text_regex() {
                Ok(re) => search::find_in_lines(&self.doc().editor, &re, from, backward, wrap),
                Err(e) => {
                    let msg = self.search_error_line(&e);
                    self.find.info = Some(msg.clone());
                    self.set_status(msg);
                    return false;
                }
            }
        };
        match found {
            Some((s, e)) => {
                let doc = self.doc_mut();
                let from_pos = doc.editor.selection.head;
                doc.record_jump(from_pos);
                if backward {
                    doc.editor.select_range(e, s);
                } else {
                    doc.editor.select_range(s, e);
                }
                self.tab_mut().view.ensure_visible = true;
                let wrapped = if backward { s > from } else { s < from };
                self.find.info = Some(if wrapped { self.t("find.wrapped", "已從另一端繼續搜尋").to_string() } else { String::new() });
                true
            }
            None => {
                self.find.info = Some(self.t("find.not_found", "找不到符合的文字").to_string());
                let q = short_query(&self.find.query);
                self.set_status(self.tf("find.not_found_q", "找不到「{q}」", None, Some(&q)));
                false
            }
        }
    }

    /// 十六進位模式：在位元組中尋找
    fn find_bytes(&mut self, backward: bool, wrap: bool) -> bool {
        let pattern = if self.find.hex {
            match search::parse_hex_query(&self.find.query) {
                Ok(p) => p,
                Err(e) => {
                    self.find.info = Some(self.search_error_line(&e));
                    return false;
                }
            }
        } else {
            match encoding::encode_text(&self.find.query, self.doc().encoding, false) {
                Ok(b) => b,
                Err(_) => {
                    self.find.info = Some(self.t("find.bad_encoding", "目前編碼無法表示搜尋文字").to_string());
                    return false;
                }
            }
        };
        let Some(hb) = self.doc_mut().hex.as_mut() else { return false };
        let len = hb.len();
        let caret = hb.caret;
        let found = if backward {
            let before = hb.selection().map(|(s, _)| s).unwrap_or(caret);
            hb.data.rfind(&pattern, before).or_else(|| if wrap { hb.data.rfind(&pattern, len) } else { None })
        } else {
            let from = hb.selection().map(|(_, e)| e).unwrap_or(caret);
            hb.data.find(&pattern, from, len).or_else(|| if wrap { hb.data.find(&pattern, 0, len) } else { None })
        };
        match found {
            Some(off) => {
                hb.move_to(off, false);
                hb.move_to(off + pattern.len() - 1, true);
                let rows = 8;
                hb.top_row = (off / crate::core::hexbuf::BYTES_PER_ROW).saturating_sub(rows);
                self.find.info = Some(self.tf("find.offset", "位移 {q}", None, Some(&format!("{off:X}h"))));
                true
            }
            None => {
                self.find.info = Some(self.t("find.no_bytes", "找不到符合的資料").to_string());
                false
            }
        }
    }

    /// Brings `find.live` up to date. Hex-mode documents have no live count: their bytes are
    /// searched only on Find Next. The regex and the per-line counts are recomputed only when
    /// the pattern or the document version changes; a selection move (arrow keys, dragging,
    /// Find Next) only relocates `current` within the cached counts.
    ///
    /// A pattern that can match a line break is matched on the joined document. That copy
    /// follows the document version alone: it is rebuilt after an edit, never for a new
    /// pattern, and never per key press or Find Next.
    fn update_live_search(&mut self) {
        let (doc_id, doc_version, hex_doc) = {
            let doc = self.doc();
            (doc.id, doc.version, doc.mode == EditMode::Hex)
        };
        let opts = (self.settings.search_match_case, self.settings.search_whole_word, self.settings.search_regex);
        let pattern_id = pattern_key(&self.find.query, self.find.hex, opts, self.doc().encoding.name());
        let counts_key = counts_key(doc_id, doc_version, pattern_id);
        let query_empty = self.find.query.is_empty();
        if self.find.live.counts_key != counts_key {
            let mut live = LiveSearch { counts_key, pattern_id, ..Default::default() };
            if !query_empty && !hex_doc {
                match self.text_regex() {
                    Ok(re) => {
                        if self.cross_line() {
                            let key = joined_key(doc_id, doc_version);
                            live.joined = match self.find.live.joined.take() {
                                Some((k, joined)) if k == key => Some((k, joined)),
                                _ => Some((key, search::JoinedText::new(&self.doc().editor.lines))),
                            };
                            if let Some((_, joined)) = &live.joined {
                                live.spans = search::text_match_spans(&re, &joined.text);
                                live.counts = search::spans_per_line(joined, &live.spans);
                            }
                        } else {
                            live.counts = search::count_matches_per_line(&self.doc().editor.lines, &re);
                        }
                        live.total = live.counts.iter().sum();
                        if live.total > 0 {
                            live.regex = Some(re);
                        }
                    }
                    Err(e) => live.error = Some(e),
                }
            }
            self.find.live = live;
        }
        // A selection change only relocates `current`: the per-line counts rank the match
        // without rescanning the lines before the cursor.
        let (s, e) = {
            let sel = self.doc().editor.selection;
            (sel.start(), sel.end())
        };
        let key = selection_key(s, e);
        if self.find.live.current_key != Some(key) {
            let current = if s == e {
                None
            } else if let Some((_, joined)) = &self.find.live.joined {
                // A cross-line match covers a selection that spans lines.
                let lines = &self.doc().editor.lines;
                search::span_index(&self.find.live.spans, joined.pos_to_byte(lines, s), joined.pos_to_byte(lines, e)).map(|i| i + 1)
            } else {
                let re = self.find.live.regex.clone();
                search::selected_match_index(&self.doc().editor, &self.find.live.counts, re.as_ref(), s, e)
            };
            self.find.live.current_key = Some(key);
            self.find.live.current = current;
        }
    }

    /// 取代目前選取的符合文字，並找下一個
    pub fn replace_once(&mut self) {
        if self.doc().read_only || self.doc().mode == EditMode::Hex {
            return;
        }
        let re = match self.text_regex() {
            Ok(r) => r,
            Err(e) => {
                self.find.info = Some(self.search_error_line(&e));
                return;
            }
        };
        let use_regex = self.settings.search_regex && !self.find.hex;
        let rep = self.find.replacement.clone();
        crate::core::settings::Settings::push_history(&mut self.settings.recent_replaces, rep.clone());
        let doc = self.doc_mut();
        let selected = doc.editor.selected_text();
        let whole = re.find(&selected).is_some_and(|m| m.start() == 0 && m.end() == selected.len());
        if whole && !selected.is_empty() {
            let new_text = if use_regex { re.replace(&selected, rep.as_str()).into_owned() } else { rep };
            let hint = doc.selection_hint();
            doc.edit(EditKind::Other, hint, |ed| ed.insert_text(&new_text));
        }
        self.find_next(false);
    }

    /// 全部取代（在選取範圍內或整份文件）
    pub fn replace_all(&mut self) {
        if self.doc().read_only || self.doc().mode == EditMode::Hex {
            return;
        }
        let re = match self.text_regex() {
            Ok(r) => r,
            Err(e) => {
                self.find.info = Some(self.search_error_line(&e));
                return;
            }
        };
        let use_regex = self.settings.search_regex && !self.find.hex;
        let rep = self.find.replacement.clone();
        crate::core::settings::Settings::push_history(&mut self.settings.recent_replaces, rep.clone());
        let in_sel = self.settings.search_in_selection;
        if self.macros.recording {
            self.macros.record(super::macros::MacroStep::ReplaceAll {
                query: self.find.query.clone(),
                replacement: rep.clone(),
                match_case: self.settings.search_match_case,
                whole_word: self.settings.search_whole_word,
                regex: self.settings.search_regex,
                in_selection: in_sel,
            });
        }
        let multiline = self.cross_line() || rep.contains('\n');
        let doc = self.doc_mut();
        let sel = doc.editor.selection;
        if in_sel && (sel.is_empty() || sel.is_block()) {
            // 「只在選取範圍內」而沒有一般選取範圍時不取代任何文字：落到整份文件會改寫
            // 使用者沒有選擇的內容（巨集重播也走這裡）。
            let msg = self.t("find.in_selection_need_range", NEED_RANGE).to_string();
            self.find.info = Some(msg.clone());
            self.set_status(msg);
            return;
        }
        let count = if multiline && !in_sel {
            // 跨行取代：整份文字一次取代，書籤依淨行數位移
            let hint = doc.whole_hint();
            doc.edit(EditKind::Other, hint, |ed| search::replace_all_multiline(ed, &re, &rep, use_regex))
        } else if in_sel {
            // 只取代選取範圍：對選取文字整段處理，以 insert_text 維護書籤
            let text = doc.editor.selected_text();
            let (new_text, n) = search::replace_in_text(&text, &re, &rep, use_regex);
            if n > 0 {
                let hint = doc.selection_hint();
                doc.edit(EditKind::Other, hint, |ed| ed.replace_selection(&new_text));
            }
            n
        } else {
            let hint = doc.whole_hint();
            let all = 0..doc.editor.lines.len();
            doc.edit(EditKind::Other, hint, |ed| search::replace_all_lines(ed, &re, &rep, use_regex, all))
        };
        let msg = self.tf("find.replaced", "已取代 {n} 處", Some(count), None);
        self.find.info = Some(msg.clone());
        self.set_status(msg);
    }

    pub fn find_bar(&mut self, ui: &mut egui::Ui) {
        // Matches are highlighted in the editor only while the window is open.
        if self.find.visible {
            self.update_live_search();
        }
        let highlight = if self.find.visible { self.find.live.regex.clone().map(|re| (re, self.find.live.pattern_id)) } else { None };
        self.tab_mut().view.find_regex = highlight;
        if !self.find.visible {
            // 關閉後重開時重新應用一次記憶位置
            self.find.pos_applied = false;
            return;
        }
        let mut close = false;
        let mut do_next = None;
        let mut do_replace = false;
        let mut do_replace_all = false;
        // 繪製前的彈層狀態：egui 的 Popup 在自己的 show() 內處理 Esc 並回寫旗標，
        // 幀末檢查時旗標已是 false，必須以繪製前的狀態判斷這一下 Esc 是否歸彈層。
        let dropdown_open = self.find.history_open || self.find.replace_history_open || self.find.quick_history_open;
        let hex_doc = self.doc().mode == EditMode::Hex;
        let can_replace = !hex_doc && !self.doc().read_only;
        // 「只在選取範圍內」需要一般選取範圍；沒有（或為欄位選取）時停用「全部取代」
        let can_replace_all = !self.settings.search_in_selection || { let s = self.doc().editor.selection; !s.is_empty() && !s.is_block() };
        let lbl_need_range = self.t("find.in_selection_need_range", NEED_RANGE).to_string();
        // 先取出譯文，避免與下方對欄位的可變借用重疊
        let lbl_find = form::bare(self.t("find.find", "尋找：")).to_string();
        let lbl_hex_hint = self.t("find.hex_hint", "十六進位，例如 4D 61 64").to_string();
        let with_keys = |name: String, c: C| match self.shortcut_text(c, ui.ctx()) {
            keys if keys.is_empty() => name,
            keys => format!("{name} ({keys})"),
        };
        let lbl_prev = self.t("find.prev_btn", "上一個");
        let lbl_next = self.t("find.btn_next", "下一個");
        let lbl_prev_tip = with_keys(plain_label(&self.cmd_text(C::FindPrevious)), C::FindPrevious);
        let lbl_next_tip = with_keys(plain_label(&self.cmd_text(C::FindNext)), C::FindNext);
        let lbl_case = self.t("fif.case", "區分大小寫");
        let lbl_word = self.t("fif.whole_word", "全字比對");
        let lbl_regex = self.t("fif.regex", "使用正規表示式（Regex）");
        let lbl_hex = self.t("find.hex_hint", "十六進位，例如 4D 61 64");
        let lbl_wrap = self.t("find.wrap", "循環");
        let lbl_find_tip = self.t("find.history", "查找歷史");
        let lbl_hist_remove = self.t("find.history_remove", "移除此紀錄");
        let lbl_hist_clear = self.t("find.history_clear", "清空搜尋紀錄");
        let lbl_replace = form::bare(self.t("find.replace", "取代：")).to_string();
        let lbl_replace_btn = self.t("find.replace_btn", "取代");
        let lbl_replace_all = self.t("find.replace_all", "全部取代");
        let lbl_in_sel = self.t("find.in_selection", "只在選取範圍內");
        let lbl_not_found = self.t("find.not_found", "找不到符合的文字");
        // The live result: "3 / 12", "12 matches", "No results" or why the query is invalid.
        let live = &self.find.live;
        // The failure's title stands in the status row; the engine's reason is its hover text.
        let error = live.error.as_ref().map(|e| self.search_error(e));
        let failed = !self.find.query.is_empty() && !hex_doc && (live.error.is_some() || live.total == 0);
        let status = if self.find.query.is_empty() || hex_doc {
            None
        } else if let Some(e) = &error {
            Some((e.title.clone(), true))
        } else if live.total == 0 {
            Some((self.t("find.no_results", "無結果").to_string(), true))
        } else if let Some(i) = live.current {
            Some((format!("{i} / {}", live.total), false))
        } else {
            Some((self.tf("find.count_n", "共 {n} 個符合", Some(live.total), None), false))
        };
        let error_detail = error.as_ref().and_then(|e| e.reason.clone());
        // 非模態浮窗：自繪緊湊標題列（拖曳由 egui 的整窗拖動承接），位置由 egui 記憶
        let ctx = ui.ctx().clone();
        let screen = ctx.content_rect();
        let title_str = self.t("find.window_title", "查找和替換").to_string();
        let mut win = egui::Window::new(title_str.clone())
            .pivot(egui::Align2::RIGHT_TOP)
            .resizable(false)
            .collapsible(false)
            .title_bar(false)
            .default_pos(egui::pos2(screen.right() - 16.0, screen.top() + 48.0));
        // 只在彈出的第一幀回到上次位置；若每幀都強制定位，會覆蓋 egui 的拖動增量而拖不動
        if !self.find.pos_applied {
            if let Some((x, y)) = self.settings.find_window_pos {
                win = win.current_pos(egui::pos2(x, y));
            }
            self.find.pos_applied = true;
        }
        let inner = win.show(&ctx, |ui| {
                // 標題列底色條：越過窗框 margin 延伸到視窗兩端、上緣隨窗體圓角，底部一條分隔線
                let full = ui.max_rect();
                let m = ui.style().spacing.window_margin;
                let rad = ui.visuals().window_corner_radius;
                let strip = egui::Rect::from_min_max(
                    egui::pos2(full.left() - m.left as f32, full.top() - m.top as f32),
                    egui::pos2(full.right() + m.right as f32, full.top() - m.top as f32 + TITLE_HEIGHT),
                );
                let strip_p = ui.painter_at(strip);
                strip_p.rect_filled(strip, egui::CornerRadius { nw: rad.nw, ne: rad.ne, sw: 0, se: 0 }, form::tab_strip_bg(ui));
                strip_p.hline(strip.left()..=strip.right(), strip.bottom() - 0.5, ui.visuals().widgets.noninteractive.bg_stroke);
                // 統一間距節奏
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
                // 緊湊標題列：在條內垂直置中。小字粗體標題 + 右端 hover 紅色「×」
                // （拷貝收藏列表的移除樣式）。
                let lbl_close = self.t("find.close", "關閉 (Esc)");
                {
                    let title_row = egui::Rect::from_min_max(strip.min, strip.max);
                    // 先佔位推進父級游標（new_child 不推進，會被頁籤蓋住），再用 child 繪製。
                    // 佔位用「內容寬」：用擴展後的全窗寬佔位會把內容最小寬度撐到窗寬，逐幀回饋變寬
                    ui.allocate_exact_size(egui::vec2(full.width(), TITLE_HEIGHT), egui::Sense::hover());
                    let mut row = ui.new_child(
                        egui::UiBuilder::new().id_salt("find-title-row").max_rect(title_row).layout(egui::Layout::left_to_right(egui::Align::Center)),
                    );
                    let font = egui::FontId::proportional(SMALL_TEXT);
                    row.add_space(m.left as f32);
                    let color = row.visuals().text_color();
                    let galley = row.painter().layout_no_wrap(title_str.clone(), font, color);
                    let (trect, _) = row.allocate_exact_size(galley.size() + egui::vec2(1.0, 0.0), egui::Sense::hover());
                    let tp = egui::pos2(trect.left(), trect.center().y - galley.size().y / 2.0);
                    paint_bold(row.painter(), tp, galley, color);
                    row.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(m.right as f32);
                        let (rect, resp) = ui.allocate_exact_size(egui::vec2(24.0, 22.0), egui::Sense::click());
                        resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, &lbl_close));
                        let weak = ui.visuals().widgets.noninteractive.text_color();
                        let tint = if resp.hovered() { REMOVE_HOVER } else { weak };
                        let clicked = resp.clicked();
                        resp.on_hover_text(lbl_close);
                        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "×", egui::FontId::proportional(13.0), tint);
                        if clicked {
                            close = true;
                        }
                    });
                }
                // 查找／替換兩個分頁（取代原本「取代…」摺疊鈕）：Ctrl+F 開查找頁、Ctrl+H 開替換頁。
                // 頁籤樣式復用 File View 停靠欄的 flat_tab（與其完全一致的外觀）。
                let lbl_tab_find = self.t("find.tab_find", "查找");
                let lbl_tab_replace = self.t("find.tab_replace", "替換");
                // 頁籤條：底色與基線越過窗框 margin 延伸到視窗兩端（封口）；頁籤仍錨定內容區
                let strip = egui::Rect::from_min_max(
                    egui::pos2(full.left() - m.left as f32, ui.cursor().top()),
                    egui::pos2(full.right() + m.right as f32, ui.cursor().top() + form::TAB_STRIP_HEIGHT),
                );
                ui.painter().rect_filled(strip, 0, form::tab_strip_bg(ui));
                let tab_rect = egui::Rect::from_min_max(
                    egui::pos2(strip.left() + 8.0, strip.top() + 2.0),
                    egui::pos2(strip.right() - 8.0, strip.bottom()),
                );
                let mut tab_ui = ui.new_child(
                    egui::UiBuilder::new().id_salt("find-window-tabs").max_rect(tab_rect).layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                tab_ui.spacing_mut().item_spacing.x = 0.0;
                let mut active_rect = None;
                let tabs = [
                    (!self.find.replace, &*lbl_tab_find, false, "find-tab-find"),
                    (self.find.replace, &*lbl_tab_replace, true, "find-tab-replace"),
                ];
                for (selected, label, to_replace, salt) in tabs {
                    let item = form::flat_tab(&mut tab_ui, salt, label, selected, ui.visuals().panel_fill, None);
                    if item.body.clicked() && self.find.replace != to_replace {
                        self.find.replace = to_replace;
                        self.find.info = None;
                    }
                    if selected {
                        active_rect = Some(item.rect);
                    }
                }
                form::tab_strip_baseline(ui, strip, active_rect);
                ui.add_space(form::TAB_STRIP_HEIGHT + 2.0);
                {
                    bar_style(ui);
                    // 版面（UE 收起態同構）：整行輸入框 → 下方一排：左齒輪／右操作按鈕 → 選項區
                    // 表頭列：「查找：」標籤＋右側歷史時鐘鈕（仿 UE，框外右上）
                    let mut hist_clicked = false;
                    let hist_btn = std::cell::RefCell::new(None::<egui::Response>);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(lbl_find.as_str()).size(SMALL_TEXT).weak());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let b = form::icon_button(ui, egui::Button::new("🕘").frame(false), lbl_find_tip, lbl_find_tip);
                            hist_clicked = b.clicked();
                            hist_btn.borrow_mut().replace(b);
                        });
                    });
                    if hist_clicked {
                        self.find.history_open = !self.find.history_open;
                    }
                    if self.find.history_open {
                        if let Some(trigger) = hist_btn.borrow().as_ref() {
                            match history_dropdown(ui, trigger, &mut self.find.history_open, &self.settings.recent_finds, &lbl_hist_remove, &lbl_hist_clear) {
                                HistAction::Pick(h) => self.find.query = h,
                                HistAction::Remove(h) => self.settings.recent_finds.retain(|x| *x != h),
                                HistAction::Clear => self.settings.recent_finds.clear(),
                                HistAction::None => {}
                            }
                        }
                    }
                    // 查找框：多行、撐滿整行，可搜尋含換行的內容（跨行比對見 `core::search::find_in_spans`）。
                    let hint = if self.find.hex { lbl_hex_hint.clone() } else { String::new() };
                    let hint = if hint.is_empty() { self.t("find.find_what", "查找內容…").to_string() } else { hint };
                    let field = multiline_field(ui, "find-query", &mut self.find.query, &hint, failed);
                    if let Some(shift) = field.enter {
                        do_next = Some(shift);
                    }
                    if field.response.changed() {
                        self.find.info = None;
                    }
                    // 開啟查找視窗時聚焦並全選，以便直接輸入新的搜尋文字
                    if std::mem::take(&mut self.find.focus) {
                        field.response.request_focus();
                        focus_select_all(ui.ctx(), field.response.id, &self.find.query);
                    }
                    let lbl_options = self.t("find.options", "選項");

                    // 狀態列：固定一行的槽位（有無內容高度都不變，避免推擠布局）
                    ui.horizontal(|ui| {
                        ui.set_min_height(ui.text_style_height(&egui::TextStyle::Body) + 4.0);
                        if let Some((text, bad)) = &status {
                            let text = RichText::new(text.as_str()).size(SMALL_TEXT);
                            let label = ui.label(if *bad { text.color(ui.visuals().error_fg_color) } else { text.weak() });
                            if let Some(detail) = &error_detail {
                                label.on_hover_text(detail.as_str());
                            }
                        } else if let Some(info) = self.find.info.as_deref().filter(|i| !i.is_empty() && (hex_doc || *i != lbl_not_found)) {
                            ui.label(RichText::new(info).weak().size(SMALL_TEXT));
                        }
                    });
                    if self.find.replace {
                        // 替換頁：取代框（與查找框同高）→ 操作排：左齒輪＋選定文字核取方塊、右下一個／替換／全部取代（無上一個）
                        let lbl_rep_hist_tip = self.t("find.replace_history_tip", "替換歷史");
                        let mut rep_hist_clicked = false;
                        let rep_hist_btn = std::cell::RefCell::new(None::<egui::Response>);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(lbl_replace.as_str()).size(SMALL_TEXT).weak());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let b = form::icon_button(ui, egui::Button::new("🕘").frame(false), lbl_rep_hist_tip, lbl_rep_hist_tip);
                                rep_hist_clicked = b.clicked();
                                rep_hist_btn.borrow_mut().replace(b);
                            });
                        });
                        if rep_hist_clicked {
                            self.find.replace_history_open = !self.find.replace_history_open;
                        }
                        if self.find.replace_history_open {
                            if let Some(trigger) = rep_hist_btn.borrow().as_ref() {
                                match history_dropdown(ui, trigger, &mut self.find.replace_history_open, &self.settings.recent_replaces, &lbl_hist_remove, &lbl_hist_clear) {
                                    HistAction::Pick(h) => self.find.replacement = h,
                                    HistAction::Remove(h) => self.settings.recent_replaces.retain(|x| *x != h),
                                    HistAction::Clear => self.settings.recent_replaces.clear(),
                                    HistAction::None => {}
                                }
                            }
                        }
                    let field = multiline_field(ui, "find-replace", &mut self.find.replacement, &lbl_replace, false);
                    // 純 Enter＝替換（與「替換」鈕相同）；唯讀或十六進位文件不回應
                    if field.enter.is_some() && can_replace {
                        do_replace = true;
                    }
                    // 操作區（UE 同構）：右欄縱排貼內容右緣；左欄＝選定文字核取方塊＋左下角齒輪
                    let pressed = action_row(
                        ui,
                        "replace-actions",
                        full,
                        &[
                            ActionButton { label: lbl_next, tip: lbl_next_tip.as_str(), action: FindAction::Next, enabled: true, disabled_tip: "" },
                            ActionButton { label: lbl_replace_btn, tip: "", action: FindAction::Replace, enabled: can_replace, disabled_tip: "" },
                            // 「全部取代」另受「只在選取範圍內」約束；停用時以 tooltip 說明原因
                            ActionButton {
                                label: lbl_replace_all,
                                tip: "",
                                action: FindAction::ReplaceAll,
                                enabled: can_replace && can_replace_all,
                                disabled_tip: if can_replace_all { "" } else { lbl_need_range.as_str() },
                            },
                        ],
                        |ui| {
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut self.settings.search_in_selection, lbl_in_sel);
                            });
                            ui.add_space(BUTTON_SIZE.1);
                            options_gear(ui, lbl_options, &mut self.find.options_open);
                        },
                    );
                    match pressed {
                        FindAction::Next => do_next = Some(false),
                        FindAction::Replace => do_replace = true,
                        FindAction::ReplaceAll => do_replace_all = true,
                        FindAction::None | FindAction::Previous => {}
                    }
                } else {
                    // 查找頁操作排（與替換頁同一套處理）：右欄＝上一個／下一個縱排貼右緣；左欄齒輪貼底
                    let pressed = action_row(
                        ui,
                        "find-actions",
                        full,
                        &[
                            ActionButton { label: lbl_prev, tip: lbl_prev_tip.as_str(), action: FindAction::Previous, enabled: true, disabled_tip: "" },
                            ActionButton { label: lbl_next, tip: lbl_next_tip.as_str(), action: FindAction::Next, enabled: true, disabled_tip: "" },
                        ],
                        // 左欄高度＝右欄（2×26＋4＝56）：齒輪與按鈕底部對齊
                        |ui| {
                            ui.add_space(BUTTON_SIZE.1);
                            options_gear(ui, lbl_options, &mut self.find.options_open);
                        },
                    );
                    match pressed {
                        FindAction::Next => do_next = Some(false),
                        FindAction::Previous => do_next = Some(true),
                        FindAction::None | FindAction::Replace | FindAction::ReplaceAll => {}
                    }
                }

                    // 「選項」齒輪展開的底部選項區：標準核取方塊（UE 的齒輪參數區）
                    if self.find.options_open {
                        ui.separator();
                        ui.horizontal_wrapped(|ui| {
                            ui.checkbox(&mut self.settings.search_whole_word, lbl_word);
                            ui.checkbox(&mut self.settings.search_match_case, lbl_case);
                            ui.checkbox(&mut self.settings.search_regex, lbl_regex);
                            ui.checkbox(&mut self.find.hex, lbl_hex);
                            ui.checkbox(&mut self.settings.search_wrap_around, lbl_wrap);
                        });
                    }
                    // While a dialog is open, Esc belongs to the dialog. A history dropdown showing
                    // over the window takes this press as well: the window closes on the next one.
                    if self.dialog.is_none() && ui.input(|i| i.key_pressed(Key::Escape)) {
                        if dropdown_open {
                            self.find.history_open = false;
                            self.find.replace_history_open = false;
                            self.find.quick_history_open = false;
                        } else {
                            close = true;
                        }
                    }
                }
            });
        // 拖動結束或關閉時記住位置並寫入設定（拖動中每幀存檔太頻繁）
        if let Some(r) = inner {
            let p = r.response.rect.right_top();
            let pos = (p.x, p.y);
            if close || r.response.drag_stopped() {
                if self.settings.find_window_pos != Some(pos) {
                    self.settings.find_window_pos = Some(pos);
                    self.save_settings();
                }
            }
        }
        if let Some(back) = do_next {
            self.find_next(back);
        }
        if do_replace {
            self.replace_once();
        }
        if do_replace_all {
            self.replace_all();
        }
        if close {
            self.find.visible = false;
            self.tab_mut().view.find_regex = None;
            self.focus_editor = true;
        }
    }

    pub fn goto_bar(&mut self, ui: &mut egui::Ui) {
        if !self.goto.visible {
            return;
        }
        let mut go = false;
        let mut close = false;
        let hex_mode = self.doc().mode == EditMode::Hex;
        let line_count = self.doc().editor.lines.len();
        let lbl_goto_pos = self.t("find.goto_pos", "移至位置（位元組，可用 0x 十六進位）：");
        let lbl_goto_line = self.t("find.goto_line", "移至行（行號 或 行:欄）：");
        let lbl_go = self.t("find.go", "跳轉");
        let lbl_close = self.t("find.close", "關閉 (Esc)");
        let lbl_by_bytes = self.t("find.by_bytes", "依位元組位置");
        bar_frame(ui).show(ui, |ui| {
            bar_style(ui);
            ui.horizontal_wrapped(|ui| {
                let by_pos = self.goto.by_position || hex_mode;
                ui.label(RichText::new(form::bare(if by_pos { lbl_goto_pos } else { lbl_goto_line })).weak());
                // The valid range as a placeholder.
                let hint = if by_pos { "0x0".to_string() } else { format!("1 – {line_count}") };
                let resp = ui.add(egui::TextEdit::singleline(&mut self.goto.input).desired_width(160.0).hint_text(hint));
                if self.goto.error.is_some() {
                    ui.painter().rect_stroke(resp.rect, 6.0, egui::Stroke::new(1.0, ui.visuals().error_fg_color), egui::StrokeKind::Inside);
                }
                if resp.changed() {
                    self.goto.error = None;
                }
                if std::mem::take(&mut self.goto.focus) {
                    resp.request_focus();
                }
                if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    go = true;
                }
                if super::dialogs::primary_button(ui, lbl_go).clicked() {
                    go = true;
                }
                if !hex_mode {
                    form::toggle_chip(ui, &mut self.goto.by_position, lbl_by_bytes, lbl_by_bytes);
                }
                if let Some(e) = &self.goto.error {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
                if form::icon_button(ui, egui::Button::new("×").frame(false), lbl_close, lbl_close).clicked() || (self.dialog.is_none() && ui.input(|i| i.key_pressed(Key::Escape))) {
                    close = true;
                }
            });
        });
        if go {
            match self.goto_target() {
                Ok(()) => {
                    self.goto.visible = false;
                    self.goto.error = None;
                    self.focus_editor = true;
                }
                Err(e) => self.goto.error = Some(e),
            }
        }
        if close {
            self.goto.visible = false;
            self.focus_editor = true;
        }
    }

    fn goto_target(&mut self) -> Result<(), String> {
        let input = self.goto.input.trim().to_string();
        // 不借用 self，錯誤字串稍後再翻譯
        let parse_num = |s: &str| -> Result<usize, String> {
            let s = s.trim();
            let r = if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                usize::from_str_radix(h, 16)
            } else if let Some(h) = s.strip_suffix(['h', 'H']) {
                usize::from_str_radix(h, 16)
            } else {
                s.parse()
            };
            r.map_err(|_| s.to_string())
        };
        let hex_mode = self.doc().mode == EditMode::Hex;
        if self.goto.by_position || hex_mode {
            let off = parse_num(&input).map_err(|tok| self.tf("find.bad_number", "無效的數字：{q}", None, Some(&tok)))?;
            let doc = self.doc_mut();
            if hex_mode {
                let hb = doc.hex.as_mut().unwrap();
                let off = off.min(hb.len());
                hb.move_to(off, false);
                hb.top_row = (off / crate::core::hexbuf::BYTES_PER_ROW).saturating_sub(5);
            } else {
                // 依編碼換算位元組位移對應的文字位置
                let target = off;
                let mut pos = TextPosition::new(0, 0);
                let nl = encoding::encoded_len(doc.newline.newline_str(), doc.encoding);
                let mut acc = if doc.has_bom { encoding::bom_bytes(doc.encoding).len() } else { 0 };
                'outer: for (li, line) in doc.editor.lines.iter().enumerate() {
                    let line_len = encoding::encoded_len(line, doc.encoding);
                    if acc + line_len >= target {
                        let mut col = 0;
                        let mut b = acc;
                        for (ci, c) in line.chars().enumerate() {
                            let cl = encoding::encoded_len(c.encode_utf8(&mut [0; 4]), doc.encoding);
                            if b + cl > target {
                                col = ci;
                                pos = TextPosition::new(li, col);
                                break 'outer;
                            }
                            b += cl;
                            col = ci + 1;
                        }
                        pos = TextPosition::new(li, col);
                        break;
                    }
                    acc += line_len + nl;
                    pos = TextPosition::new(li, line.chars().count());
                }
                let from = doc.editor.selection.head;
                doc.record_jump(from);
                doc.editor.set_caret(pos.line, pos.col);
            }
        } else {
            let mut parts = input.splitn(2, [':', ',']);
            let line = parse_num(parts.next().unwrap_or("")).map_err(|tok| self.tf("find.bad_number", "無效的數字：{q}", None, Some(&tok)))?.max(1);
            let col = match parts.next() {
                Some(s) => parse_num(s).map_err(|tok| self.tf("find.bad_number", "無效的數字：{q}", None, Some(&tok)))?.max(1),
                None => 1,
            };
            let doc = self.doc_mut();
            let from = doc.editor.selection.head;
            doc.record_jump(from);
            doc.editor.set_caret(line - 1, col - 1);
        }
        self.tab_mut().view.ensure_visible = true;
        Ok(())
    }

    /// 顯示訊息對話框（供其他模組使用）
    pub fn message(&mut self, title: &str, text: impl Into<String>) {
        self.confirm = Some(Confirm::Message { title: title.into(), text: text.into() });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_live_counts_and_keys_only_invalidate_what_changed() {
        let opts = (false, false, false);
        let p = pattern_key("ab", false, opts, "UTF-8");
        assert_ne!(selection_key(TextPosition::new(0, 0), TextPosition::new(0, 1)), selection_key(TextPosition::new(1, 1), TextPosition::new(1, 3)));
        assert_eq!(counts_key(7, 3, p), counts_key(7, 3, p));
        assert_ne!(counts_key(7, 3, p), counts_key(7, 4, p), "an edit must invalidate the counts");
        assert_ne!(counts_key(7, 3, p), counts_key(8, 3, p), "another document must invalidate the counts");
        assert_ne!(p, pattern_key("ab", false, (true, false, false), "UTF-8"), "an option change must invalidate the pattern");
        assert_ne!(p, pattern_key("ab", false, opts, "BIG5"), "a different encoding must invalidate the pattern");
        // The joined copy follows the document version only, never the pattern.
        assert_eq!(joined_key(7, 3), joined_key(7, 3));
        assert_ne!(joined_key(7, 3), joined_key(7, 4));
        assert_ne!(joined_key(7, 3), joined_key(8, 3));
    }

    #[test]
    fn test_char_index_and_line_col_convert_each_way() {
        let text = "ab\n中c\n\nend";
        let at = [(0, 0, 0), (1, 0, 1), (2, 0, 2), (3, 1, 0), (4, 1, 1), (5, 1, 2), (6, 2, 0), (7, 3, 0), (8, 3, 1), (9, 3, 2)];
        for (idx, line, col) in at {
            assert_eq!(char_index_to_line_col(text, idx), (line, col), "char {idx}");
            assert_eq!(line_col_to_char_index(text, line, col), idx, "line {line} col {col}");
        }
        // Past the end: the last line, at its end.
        assert_eq!(char_index_to_line_col(text, 99), (3, 3));
    }
}
