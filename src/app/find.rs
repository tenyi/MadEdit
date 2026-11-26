//! 搜尋／取代列與跳行列：文字模式以行為單位比對（Regex），十六進位模式比對位元組

use std::hash::{Hash, Hasher};

use eframe::egui::{self, Key, RichText, Role, WidgetInfo};
use regex::Regex;

use super::commands::{plain_label, C};
use super::{form, App, Confirm};
use crate::core::document::EditKind;
use crate::core::editor::{char_to_byte, EditorState, TextPosition};
use crate::core::encoding;
use crate::core::model::EditMode;
use crate::core::search::{self, SearchOptions};

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

/// 歷史下拉彈層：錨定觸發鈕（標籤列右端的時鐘鈕，仿 UE）向下展開；
/// 每列右端 hover 紅色「×」移除單筆，底部「清空」。紅 × 樣式拷貝自收藏列表的移除鈕。
pub(super) fn history_dropdown(
    trigger: &egui::Response,
    open: &mut bool,
    entries: &[String],
    remove_label: &str,
    clear_label: &str,
) -> HistAction {
    use egui::{Color32};
    let mut action = HistAction::None;
    if !*open || entries.is_empty() {
        if entries.is_empty() {
            *open = false;
        }
        return action;
    }
    let popup = egui::Popup::from_toggle_button_response(trigger)
        .open_bool(open)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    popup.show(|ui| {
        ui.set_min_width(240.0);
        let body = egui::TextStyle::Body.resolve(ui.style());
        const CLOSE_W: f32 = 20.0;
        for entry in entries {
            let picked;
            let mut removed = false;
            let (rect, response) = ui.allocate_exact_size(egui::vec2(240.0, 22.0), egui::Sense::click());
            response.widget_info(|| WidgetInfo::labeled(Role::Button, true, entry));
            // 指標在整行範圍內即整行 hover：文字與「×」一起亮（× 的熱區疊在行內，不能只看 response.hovered）
            let row_hovered = ui.rect_contains_pointer(rect);
            if ui.is_rect_visible(rect) {
                if row_hovered {
                    ui.painter().rect_filled(rect, 4.0, ui.visuals().widgets.hovered.weak_bg_fill);
                }
                let mut layout = egui::text::LayoutJob {
                    wrap: egui::text::TextWrapping::truncate_at_width((rect.width() - CLOSE_W - 16.0).max(0.0)),
                    ..Default::default()
                };
                layout.append(entry, 0.0, egui::TextFormat::simple(body.clone(), ui.visuals().text_color()));
                let galley = ui.painter().layout_job(layout);
                let gp = egui::pos2(rect.left() + 8.0, rect.center().y - galley.size().y / 2.0);
                ui.painter().galley(gp, galley, ui.visuals().text_color());
                if row_hovered {
                    // 列尾獨立熱區的紅「×」：無按鈕底，只有字符變紅（收藏列表移除鈕同款）
                    let x_rect = egui::Rect::from_min_size(
                        egui::pos2(rect.right() - 4.0 - CLOSE_W, rect.top()),
                        egui::vec2(CLOSE_W, rect.height()),
                    );
                    let r2 = ui.interact(x_rect, response.id.with("remove"), egui::Sense::click());
                    r2.widget_info(|| WidgetInfo::labeled(Role::Button, true, remove_label));
                    removed = r2.clicked();
                    r2.clone().on_hover_text(remove_label);
                    let tint = if r2.hovered() { Color32::from_rgb(232, 17, 35) } else { ui.visuals().weak_text_color() };
                    ui.painter().text(x_rect.center(), egui::Align2::CENTER_CENTER, "×", egui::FontId::proportional(13.0), tint);
                }
            }
            // 點「×」不算選中該列
            picked = response.clicked() && !removed;
            if picked {
                action = HistAction::Pick(entry.clone());
            } else if removed {
                action = HistAction::Remove(entry.clone());
            }
        }
        ui.separator();
        let clear = ui.add_sized([240.0, 20.0], egui::Button::selectable(false, RichText::new(clear_label).weak().size(body.size)));
        if clear.clicked() {
            action = HistAction::Clear;
        }
    });
    // 選中／移除／清空任何一項都自動收起彈層
    if !matches!(action, HistAction::None) {
        *open = false;
    }
    action
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
    use egui::{Color32, Pos2, Shape, Stroke};
    let mut action = HistAction::None;
    let chev_rect = egui::Rect::from_center_size(
        egui::pos2(field.right() - 14.0, field.center().y),
        egui::vec2(22.0, field.height()),
    );
    let resp = ui.interact(chev_rect, ui.scope_id().with(id_salt), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
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
    if !*open || entries.is_empty() {
        if entries.is_empty() {
            *open = false;
        }
        return action;
    }
    let popup = egui::Popup::from_toggle_button_response(&resp)
        .anchor(field)
        .frame(egui::Frame::popup(&ui.ctx().global_style()).inner_margin(egui::Margin::symmetric(4, 5)))
        .open_bool(open)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    popup.show(|ui| {
        let inner_w = field.width() - 8.0;
        ui.set_min_width(inner_w);
        ui.set_max_width(inner_w);
        let body = egui::TextStyle::Body.resolve(ui.style());
        const CLOSE_W: f32 = 20.0;
        for entry in entries {
            let picked;
            let mut removed = false;
            let (rect, response) = ui.allocate_exact_size(egui::vec2(inner_w, 22.0), egui::Sense::click());
            response.widget_info(|| WidgetInfo::labeled(Role::Button, true, entry));
            let row_hovered = ui.rect_contains_pointer(rect);
            if ui.is_rect_visible(rect) {
                if row_hovered {
                    ui.painter().rect_filled(rect, 4.0, ui.visuals().widgets.hovered.weak_bg_fill);
                }
                let mut layout = egui::text::LayoutJob {
                    wrap: egui::text::TextWrapping::truncate_at_width((rect.width() - CLOSE_W - 16.0).max(0.0)),
                    ..Default::default()
                };
                layout.append(entry, 0.0, egui::TextFormat::simple(body.clone(), ui.visuals().text_color()));
                let galley = ui.painter().layout_job(layout);
                let gp = egui::pos2(rect.left() + 8.0, rect.center().y - galley.size().y / 2.0);
                ui.painter().galley(gp, galley, ui.visuals().text_color());
                if row_hovered {
                    let x_rect = egui::Rect::from_min_size(
                        egui::pos2(rect.right() - 4.0 - CLOSE_W, rect.top()),
                        egui::vec2(CLOSE_W, rect.height()),
                    );
                    let r2 = ui.interact(x_rect, response.id.with("remove"), egui::Sense::click());
                    r2.widget_info(|| WidgetInfo::labeled(Role::Button, true, remove_label));
                    removed = r2.clicked();
                    r2.clone().on_hover_text(remove_label);
                    let tint = if r2.hovered() { Color32::from_rgb(232, 17, 35) } else { ui.visuals().weak_text_color() };
                    ui.painter().text(x_rect.center(), egui::Align2::CENTER_CENTER, "×", egui::FontId::proportional(13.0), tint);
                }
            }
            picked = response.clicked() && !removed;
            if picked {
                action = HistAction::Pick(entry.clone());
            } else if removed {
                action = HistAction::Remove(entry.clone());
            }
        }
        ui.separator();
        let clear = ui.add_sized([inner_w, 20.0], egui::Button::selectable(false, RichText::new(clear_label).weak().size(body.size)));
        if clear.clicked() {
            action = HistAction::Clear;
        }
    });
    if !matches!(action, HistAction::None) {
        *open = false;
    }
    action
}

/// The find bar's live result: the regex and the per-line match counts belong to the pattern
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
    pub(crate) error: Option<String>,
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

/// 在行陣列中尋找下一個（或上一個）符合的位置
///
/// `range` 限制只在指定範圍內搜尋（「在選取範圍內搜尋」）。
pub fn find_in_lines(
    ed: &EditorState,
    re: &Regex,
    from: TextPosition,
    backward: bool,
    wrap: bool,
    range: Option<(TextPosition, TextPosition)>,
) -> Option<(TextPosition, TextPosition)> {
    let (lo, hi) = range.unwrap_or((TextPosition::new(0, 0), TextPosition::new(ed.lines.len() - 1, ed.line_char_count(ed.lines.len() - 1))));
    let in_range = |s: TextPosition, e: TextPosition| s >= lo && e <= hi;
    let line_matches = |li: usize| -> Vec<(TextPosition, TextPosition)> {
        let line = &ed.lines[li];
        re.find_iter(line)
            .filter(|m| m.start() != m.end())
            .map(|m| {
                let s = line[..m.start()].chars().count();
                let e = s + line[m.start()..m.end()].chars().count();
                (TextPosition::new(li, s), TextPosition::new(li, e))
            })
            .filter(|(s, e)| in_range(*s, *e))
            .collect()
    };
    let n = ed.lines.len();
    if !backward {
        for li in from.line..n {
            if let Some(m) = line_matches(li).into_iter().find(|(s, _)| *s >= from) {
                return Some(m);
            }
        }
        if wrap {
            for li in 0..=from.line.min(n - 1) {
                if let Some(m) = line_matches(li).into_iter().next() {
                    return Some(m);
                }
            }
        }
    } else {
        for li in (0..=from.line.min(n - 1)).rev() {
            if let Some(m) = line_matches(li).into_iter().rev().find(|(_, e)| *e <= from) {
                return Some(m);
            }
        }
        if wrap {
            for li in (from.line..n).rev() {
                if let Some(m) = line_matches(li).into_iter().last() {
                    return Some(m);
                }
            }
        }
    }
    None
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

/// Matches per line for the live count; summing gives the document total.
fn count_matches_per_line(lines: &[String], re: &Regex) -> Vec<usize> {
    lines.iter().map(|line| re.find_iter(line).filter(|m| m.start() != m.end()).count()).collect()
}

/// 模式可能跨行：查詢含實際換行，或正則寫出 \n 逸出（此時逐行比對必為零結果）
fn pattern_may_span_lines(query: &str) -> bool {
    query.contains('\n') || query.contains(r"\n")
}

/// 每行起始的位元組位移（以 \n 連接整份文字後），供位移↔位置換算
fn line_start_offsets(lines: &[String]) -> Vec<usize> {
    let mut starts = Vec::with_capacity(lines.len());
    let mut acc = 0usize;
    for line in lines {
        starts.push(acc);
        acc += line.len() + 1;
    }
    starts
}

/// 跨行搜尋：以 \n 連接整份文字一次比對，模式可含換行（逐行比對零結果時的後備路徑）。
pub fn find_in_text(ed: &EditorState, re: &Regex, from: TextPosition, backward: bool, wrap: bool) -> Option<(TextPosition, TextPosition)> {
    let text = ed.lines.join("\n");
    let starts = line_start_offsets(&ed.lines);
    let pos_to_byte = |p: TextPosition| -> usize {
        let li = p.line.min(ed.lines.len() - 1);
        let col = p.col.min(ed.lines[li].chars().count());
        starts[li] + char_to_byte(&ed.lines[li], col)
    };
    let byte_to_pos = |b: usize| -> TextPosition {
        let b = b.min(text.len());
        let li = starts.partition_point(|&s| s <= b).saturating_sub(1);
        TextPosition::new(li, text[starts[li]..b].chars().count())
    };
    let from_b = pos_to_byte(from);
    let all: Vec<(usize, usize)> = re.find_iter(&text).filter(|m| m.start() != m.end()).map(|m| (m.start(), m.end())).collect();
    let span = if !backward {
        all.iter().find(|(s, _)| *s >= from_b).copied().or_else(|| wrap.then(|| all.first().copied()).flatten())
    } else {
        all.iter().rev().find(|(_, e)| *e <= from_b).copied().or_else(|| wrap.then(|| all.last().copied()).flatten())
    };
    span.map(|(s, e)| (byte_to_pos(s), byte_to_pos(e)))
}

/// 跨行比對的逐行計數：每個符合記到其起始行（總數即符合總數）
fn count_matches_in_text(lines: &[String], re: &Regex) -> Vec<usize> {
    let text = lines.join("\n");
    let starts = line_start_offsets(lines);
    let mut counts = vec![0usize; lines.len()];
    for m in re.find_iter(&text).filter(|m| m.start() != m.end()) {
        let li = starts.partition_point(|&s| s <= m.start()).saturating_sub(1);
        counts[li] += 1;
    }
    counts
}

/// 跨行全部取代：整份文字一次取代後重建行列表；書籤依每個取代點的淨行數位移。
fn replace_all_multiline(ed: &mut EditorState, re: &Regex, rep: &str, use_regex: bool) -> usize {
    let text = ed.lines.join("\n");
    let starts = line_start_offsets(&ed.lines);
    let mut out = String::with_capacity(text.len());
    let mut last = 0usize;
    let mut count = 0usize;
    // 每個取代點：(起始行, 淨行數變化)
    let mut deltas: Vec<(usize, i64)> = Vec::new();
    for cap in re.captures_iter(&text) {
        let m = cap.get(0).unwrap();
        if m.start() == m.end() {
            continue;
        }
        let seg = if use_regex {
            let mut dst = String::new();
            cap.expand(rep, &mut dst);
            dst
        } else {
            rep.to_string()
        };
        let old_lines = 1 + text[m.start()..m.end()].matches('\n').count();
        let new_lines = 1 + seg.matches('\n').count();
        let at_line = starts.partition_point(|&s| s <= m.start()).saturating_sub(1);
        deltas.push((at_line, new_lines as i64 - old_lines as i64));
        out.push_str(&text[last..m.start()]);
        out.push_str(&seg);
        last = m.end();
        count += 1;
    }
    out.push_str(&text[last..]);
    let new_lines: Vec<String> = out.split('\n').map(str::to_string).collect();
    // 書籤位移：位在某取代點之後的書籤累加該點的淨行數；越界的捨棄
    ed.bookmarks = ed
        .bookmarks
        .iter()
        .filter_map(|&b| {
            let shift: i64 = deltas.iter().filter(|(at, _)| *at < b).map(|(_, d)| d).sum();
            let nb = b as i64 + shift;
            (nb >= 0 && (nb as usize) < new_lines.len()).then_some(nb as usize)
        })
        .collect();
    ed.lines = new_lines;
    ed.clamp_caret();
    count
}

/// Which match (1-based) the selection covers exactly, if any. `counts` holds one entry per
/// line, so only the selection's own line is scanned; the lines before it contribute their
/// cached counts.
fn selected_match_index(ed: &EditorState, counts: &[usize], re: Option<&Regex>, s: TextPosition, e: TextPosition) -> Option<usize> {
    let re = re?;
    if s.line != e.line || s == e || s.line >= counts.len() || s.line >= ed.lines.len() {
        return None;
    }
    let line = &ed.lines[s.line];
    let before: usize = counts[..s.line].iter().sum();
    let (sb, eb) = (char_to_byte(line, s.col), char_to_byte(line, e.col));
    for (rank, m) in re.find_iter(line).filter(|m| m.start() != m.end()).enumerate() {
        if m.start() == sb && m.end() == eb {
            return Some(before + rank + 1);
        }
    }
    None
}

/// 全部取代（以行為單位），並維護書籤（取代文字含換行時，書籤隨行號位移）
pub fn replace_all_lines(ed: &mut EditorState, re: &Regex, rep: &str, use_regex: bool, lines: std::ops::Range<usize>) -> usize {
    let mut count = 0;
    let mut new_lines: Vec<String> = Vec::with_capacity(ed.lines.len());
    let mut line_map: Vec<usize> = Vec::with_capacity(ed.lines.len());
    for (li, line) in ed.lines.iter().enumerate() {
        line_map.push(new_lines.len());
        if !lines.contains(&li) {
            new_lines.push(line.clone());
            continue;
        }
        let mut n = 0;
        let replaced = re.replace_all(line, |caps: &regex::Captures| {
            n += 1;
            if use_regex {
                let mut dst = String::new();
                caps.expand(rep, &mut dst);
                dst
            } else {
                rep.to_string()
            }
        });
        count += n;
        if n > 0 && replaced.contains(['\n', '\r']) {
            let norm = replaced.replace("\r\n", "\n").replace('\r', "\n");
            new_lines.extend(norm.split('\n').map(str::to_string));
        } else {
            new_lines.push(replaced.into_owned());
        }
    }
    if count > 0 {
        // P1-11：書籤依新的行號重新對應
        ed.bookmarks = ed.bookmarks.iter().filter_map(|&b| line_map.get(b).copied()).collect();
        ed.lines = new_lines;
        ed.clamp_caret();
    }
    count
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

impl App {
    pub fn search_options(&self) -> SearchOptions {
        SearchOptions::new(self.settings.search_match_case, self.settings.search_whole_word, self.settings.search_regex)
    }

    /// 建立文字搜尋用的 Regex（十六進位查詢先依文件編碼解碼成文字）
    fn text_regex(&self) -> Result<Regex, String> {
        let q = &self.find.query;
        if self.find.hex {
            let bytes = search::parse_hex_query(q)?;
            let d = encoding::decode_with(&bytes, self.doc().encoding);
            let opts = SearchOptions::new(true, false, false);
            return search::build_regex(&d.text, &opts);
        }
        search::build_regex(q, &self.search_options())
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
        let re = match self.text_regex() {
            Ok(r) => r,
            Err(e) => {
                self.find.info = Some(e);
                return false;
            }
        };
        let multiline = pattern_may_span_lines(&self.find.query);
        let doc = self.doc_mut();
        let sel = doc.editor.selection;
        let range = None;
        let from = if backward { sel.start() } else { sel.end() };
        // 逐行比對為主；模式含換行（逐行必零結果）時落到整文跨行搜尋
        let found = match find_in_lines(&doc.editor, &re, from, backward, wrap, range) {
            Some(m) => Some(m),
            None if multiline => find_in_text(&doc.editor, &re, from, backward, wrap),
            None => None,
        };
        match found {
            Some((s, e)) => {
                let from_pos = sel.head;
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
                let q = self.find.query.clone();
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
                    self.find.info = Some(e);
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
    fn update_live_search(&mut self) {
        let doc = self.doc();
        let hex_doc = doc.mode == EditMode::Hex;
        let opts = (self.settings.search_match_case, self.settings.search_whole_word, self.settings.search_regex);
        let pattern_id = pattern_key(&self.find.query, self.find.hex, opts, doc.encoding.name());
        let counts_key = counts_key(doc.id, doc.version, pattern_id);
        let query_empty = self.find.query.is_empty();
        if self.find.live.counts_key != counts_key {
            let mut live = LiveSearch { counts_key, pattern_id, ..Default::default() };
            if !query_empty && !hex_doc {
                match self.text_regex() {
                    Ok(re) => {
                        live.counts = count_matches_per_line(&self.doc().editor.lines, &re);
                        live.total = live.counts.iter().sum();
                        // 跨行模式：逐行比對必為零結果，改以整文比對、符合記到起始行
                        if live.total == 0 && pattern_may_span_lines(&self.find.query) {
                            live.counts = count_matches_in_text(&self.doc().editor.lines, &re);
                            live.total = live.counts.iter().sum();
                        }
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
        let ed = self.doc();
        let (s, e) = (ed.editor.selection.start(), ed.editor.selection.end());
        let key = selection_key(s, e);
        if self.find.live.current_key != Some(key) {
            let re = self.find.live.regex.clone();
            let current = selected_match_index(&ed.editor, &self.find.live.counts, re.as_ref(), s, e);
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
                self.find.info = Some(e);
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
                self.find.info = Some(e);
                return;
            }
        };
        let use_regex = self.settings.search_regex && !self.find.hex;
        let rep = self.find.replacement.clone();
        crate::core::settings::Settings::push_history(&mut self.settings.recent_replaces, rep.clone());
        if self.macros.recording {
            self.macros.record(super::macros::MacroStep::ReplaceAll {
                query: self.find.query.clone(),
                replacement: rep.clone(),
                match_case: self.settings.search_match_case,
                whole_word: self.settings.search_whole_word,
                regex: self.settings.search_regex,
            });
        }
        let in_sel = self.settings.search_in_selection;
        let multiline = pattern_may_span_lines(&self.find.query) || rep.contains('\n');
        let doc = self.doc_mut();
        let sel = doc.editor.selection;
        let count = if multiline && !in_sel {
            // 跨行取代：整份文字一次取代，書籤依淨行數位移
            let hint = doc.whole_hint();
            doc.edit(EditKind::Other, hint, |ed| replace_all_multiline(ed, &re, &rep, use_regex))
        } else if in_sel && !sel.is_empty() && !sel.is_block() {
            // 只取代選取範圍：對選取文字整段處理，以 insert_text 維護書籤
            let text = doc.editor.selected_text();
            let mut n = 0;
            let new_text = re
                .replace_all(&text, |caps: &regex::Captures| {
                    n += 1;
                    if use_regex {
                        let mut d = String::new();
                        caps.expand(&rep, &mut d);
                        d
                    } else {
                        rep.clone()
                    }
                })
                .into_owned();
            if n > 0 {
                let hint = doc.selection_hint();
                doc.edit(EditKind::Other, hint, |ed| ed.replace_selection(&new_text));
            }
            n
        } else {
            let hint = doc.whole_hint();
            let all = 0..doc.editor.lines.len();
            doc.edit(EditKind::Other, hint, |ed| replace_all_lines(ed, &re, &rep, use_regex, all))
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
        let hex_doc = self.doc().mode == EditMode::Hex;
        let can_replace = !hex_doc && !self.doc().read_only;
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
        let failed = !self.find.query.is_empty() && !hex_doc && (live.error.is_some() || live.total == 0);
        let status = if self.find.query.is_empty() || hex_doc {
            None
        } else if let Some(e) = &live.error {
            Some((e.lines().next().unwrap_or_default().to_string(), true))
        } else if live.total == 0 {
            Some((self.t("find.no_results", "無結果").to_string(), true))
        } else if let Some(i) = live.current {
            Some((format!("{i} / {}", live.total), false))
        } else {
            Some((self.tf("find.count_n", "共 {n} 個符合", Some(live.total), None), false))
        };
        let error_detail = live.error.clone();
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
                    egui::pos2(full.right() + m.right as f32, full.top() - m.top as f32 + 30.0),
                );
                let strip_p = ui.painter_at(strip);
                strip_p.rect_filled(strip, egui::CornerRadius { nw: rad.nw, ne: rad.ne, sw: 0, se: 0 }, form::tab_strip_bg(ui));
                strip_p.hline(strip.left()..=strip.right(), strip.bottom() - 0.5, ui.visuals().widgets.noninteractive.bg_stroke);
                // 統一間距節奏
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
                // 緊湊標題列：在條內垂直置中。小字粗體標題 + 右端 hover 紅色「×」
                // （拷貝收藏列表的移除樣式）。介面字型鏈只有單一字重，
                // RichText::strong 對中文無效，改以雙重繪製加粗。
                let lbl_close = self.t("find.close", "關閉 (Esc)");
                {
                    let title_row = egui::Rect::from_min_max(strip.min, strip.max);
                    // 先佔位推進父級游標（new_child 不推進，會被頁籤蓋住），再用 child 繪製。
                    // 佔位用「內容寬」：用擴展後的全窗寬佔位會把內容最小寬度撐到窗寬，逐幀回饋變寬
                    ui.allocate_exact_size(egui::vec2(full.width(), 30.0), egui::Sense::hover());
                    let mut row = ui.new_child(
                        egui::UiBuilder::new().id_salt("find-title-row").max_rect(title_row).layout(egui::Layout::left_to_right(egui::Align::Center)),
                    );
                    let font = egui::FontId::proportional(12.5);
                    row.add_space(m.left as f32);
                    let color = row.visuals().text_color();
                    let galley = row.painter().layout_no_wrap(title_str.clone(), font, color);
                    let (trect, _) = row.allocate_exact_size(galley.size() + egui::vec2(1.0, 0.0), egui::Sense::hover());
                    let tp = egui::pos2(trect.left(), trect.center().y - galley.size().y / 2.0);
                    row.painter().galley(tp, galley.clone(), color);
                    row.painter().galley(tp + egui::vec2(0.7, 0.0), galley, color);
                    row.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(m.right as f32);
                        let (rect, resp) = ui.allocate_exact_size(egui::vec2(24.0, 22.0), egui::Sense::click());
                        resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, &lbl_close));
                        let weak = ui.visuals().widgets.noninteractive.text_color();
                        let tint = if resp.hovered() { egui::Color32::from_rgb(232, 17, 35) } else { weak };
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
                        ui.label(RichText::new(lbl_find.as_str()).size(12.5).weak());
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
                            match history_dropdown(trigger, &mut self.find.history_open, &self.settings.recent_finds, &lbl_hist_remove, &lbl_hist_clear) {
                                HistAction::Pick(h) => self.find.query = h,
                                HistAction::Remove(h) => self.settings.recent_finds.retain(|x| *x != h),
                                HistAction::Clear => self.settings.recent_finds.clear(),
                                HistAction::None => {}
                            }
                        }
                    }
                    // 查找框：多行、撐滿整行（可搜尋含換行的內容，跨行比對引擎見 find_in_text）。
                    let field_w = ui.available_width() - 12.0;
                    let hint = if self.find.hex { lbl_hex_hint.clone() } else { String::new() };
                    let hint = if hint.is_empty() { self.t("find.find_what", "查找內容…").to_string() } else { hint };
                    let query_before = self.find.query.clone();
                    let query_lines = self.find.query.lines().count().saturating_sub(1);
                    let field = egui::TextEdit::multiline(&mut self.find.query)
                        .desired_width(field_w)
                        .desired_rows(4)
                        .frame(egui::Frame::NONE)
                        .margin(egui::Margin::symmetric(6, 4))
                        .hint_text(hint);
                    // 固定高度滾動容器：TextEdit 隨內容長高，由 ScrollArea 裁剪＋內部捲動；
                    // 邊框／箭頭錨定容器視口（不隨內容滾動而破版）
                    let box_h = ui.text_style_height(&egui::TextStyle::Body) * 4.0 + 24.0;
                    let vp_rect = egui::Rect::from_min_size(egui::pos2(ui.max_rect().left(), ui.cursor().top()), egui::vec2(ui.max_rect().width(), box_h));
                    // 底色先鋪（TextEdit 已無自身邊框／底色）
                    ui.painter().rect_filled(vp_rect, 6.0, ui.visuals().extreme_bg_color);
                    // 佔位推進父級游標（new_child 不推進，後續控件會全部疊上來）
                    ui.allocate_exact_size(egui::vec2(vp_rect.width(), box_h), egui::Sense::hover());
                    let mut scroll_ui = ui.new_child(
                        egui::UiBuilder::new().id_salt("find-query-scroll-area").max_rect(vp_rect.shrink2(egui::vec2(6.0, 4.0))).layout(egui::Layout::top_down(egui::Align::LEFT)),
                    );
                    let resp = egui::ScrollArea::vertical()
                        .id_salt("find-query-scroll")
                        .auto_shrink([false, false])
                        .min_scrolled_height(vp_rect.height() - 8.0)
                        .max_height(vp_rect.height() - 8.0)
                        .show(&mut scroll_ui, |ui| ui.add(field))
                        .inner;
                    // 滾輪：游標整行步進（滾輪上＝往上、滾輪下＝往下；egui 自動把游標行捲入視野）
                    if ui.rect_contains_pointer(vp_rect) {
                        let dy = ui.input(|i| i.smooth_scroll_delta.y);
                        if dy != 0.0 {
                            let step = ((dy.abs() / ui.text_style_height(&egui::TextStyle::Body)).round() as usize).max(1);
                            let dir: isize = if dy > 0.0 { -1 } else { 1 };
                            if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), resp.id) {
                                let char_idx = st
                                    .cursor
                                    .char_range()
                                    .map(|r| r.primary.index.0.min(r.secondary.index.0))
                                    .unwrap_or(0);
                                let (cur_l, cur_c) = char_index_to_line_col(&self.find.query, char_idx);
                                let new_l = (cur_l as isize + dir * step as isize).clamp(0, query_lines as isize) as usize;
                                let line_len = self.find.query.split('\n').nth(new_l).map(|l| l.chars().count()).unwrap_or(0);
                                let col = cur_c.min(line_len);
                                let new_idx = line_col_to_char_index(&self.find.query, new_l, col);
                                st.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(new_idx))));
                                st.store(ui.ctx(), resp.id);
                            }
                        }
                    }
                    // 邊框畫在容器視口上：聚焦＝白框、hover／未聚焦＝灰框，查詢錯誤時改紅框
                    let focused = resp.has_focus();
                    let hovered = ui.rect_contains_pointer(vp_rect);
                    let border = if failed {
                        egui::Stroke::new(1.0, ui.visuals().error_fg_color)
                    } else if focused {
                        ui.visuals().widgets.active.bg_stroke
                    } else if hovered {
                        ui.visuals().widgets.hovered.bg_stroke
                    } else {
                        ui.visuals().widgets.noninteractive.bg_stroke
                    };
                    ui.painter().rect_stroke(vp_rect, 6.0, border, egui::StrokeKind::Inside);
                    // Windows 慣例：純 Enter 觸發查找不換行（還原本幀插入的 \n）；Ctrl+Enter 才插入換行
                    let enter_pressed = ui.input(|i| i.key_pressed(Key::Enter));
                    let with_ctrl = ui.input(|i| i.modifiers.ctrl);
                    if enter_pressed && !with_ctrl {
                        self.find.query = query_before;
                        do_next = Some(ui.input(|i| i.modifiers.shift_only()));
                        resp.request_focus();
                    } else if enter_pressed && with_ctrl {
                        // egui 帶修飾鍵時不插入換行：手動在游標處補一個 \n 並移動游標
                        if self.find.query == query_before {
                            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), resp.id) {
                                let head = state.cursor.char_range().map_or_else(
                                    || self.find.query.chars().count(),
                                    |r| r.primary.index.0.min(r.secondary.index.0),
                                );
                                let byte = char_to_byte(&self.find.query, head);
                                self.find.query.insert(byte, '\n');
                                state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                                    egui::text::CCursor::new(head + 1),
                                    egui::text::CCursor::new(head + 1),
                                )));
                                state.store(ui.ctx(), resp.id);
                            }
                        }
                    }
                    if resp.changed() {
                        self.find.info = None;
                    }
                    if std::mem::take(&mut self.find.focus) {
                        resp.request_focus();
                        // 全選以便直接輸入新的搜尋文字
                        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), resp.id) {
                            let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(self.find.query.chars().count()));
                            state.cursor.set_char_range(Some(all));
                            state.store(ui.ctx(), resp.id);
                        }
                    }
                    // （純 Enter＝下一個、Shift+Enter＝上一個、Ctrl+Enter＝換行，見上方 Enter 處理）
                    // 狀態列：固定一行的槽位（有無內容高度都不變，避免推擠布局）
                    ui.horizontal(|ui| {
                        ui.set_min_height(ui.text_style_height(&egui::TextStyle::Body) + 4.0);
                        if let Some((text, bad)) = &status {
                            let text = RichText::new(text.as_str()).size(12.5);
                            let label = ui.label(if *bad { text.color(ui.visuals().error_fg_color) } else { text.weak() });
                            if let Some(detail) = &error_detail {
                                label.on_hover_text(detail.as_str());
                            }
                        } else if let Some(info) = self.find.info.as_deref().filter(|i| !i.is_empty() && (hex_doc || *i != lbl_not_found)) {
                            ui.label(RichText::new(info).weak().size(12.5));
                        }
                    });
                    if self.find.replace {
                        // 替換頁：取代框（與查找框同高）→ 操作排：左齒輪＋選定文字核取方塊、右下一個／替換／全部取代（無上一個）
                        let lbl_rep_hist_tip = self.t("find.replace_history_tip", "替換歷史");
                        let mut rep_hist_clicked = false;
                        let rep_hist_btn = std::cell::RefCell::new(None::<egui::Response>);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(lbl_replace.as_str()).size(12.5).weak());
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
                                match history_dropdown(trigger, &mut self.find.replace_history_open, &self.settings.recent_replaces, &lbl_hist_remove, &lbl_hist_clear) {
                                    HistAction::Pick(h) => self.find.replacement = h,
                                    HistAction::Remove(h) => self.settings.recent_replaces.retain(|x| *x != h),
                                    HistAction::Clear => self.settings.recent_replaces.clear(),
                                    HistAction::None => {}
                                }
                            }
                        }
                        let rep_before = self.find.replacement.clone();
                        let rep_lines = self.find.replacement.lines().count().saturating_sub(1);
                        let field = egui::TextEdit::multiline(&mut self.find.replacement)
                            .desired_width(ui.available_width() - 12.0)
                            .desired_rows(4)
                            .frame(egui::Frame::NONE)
                            .margin(egui::Margin::symmetric(6, 4))
                            .hint_text(lbl_replace.clone());
                        // 替換框：固定高度滾動容器（同查找框）
                        let rep_vp = egui::Rect::from_min_size(egui::pos2(ui.max_rect().left(), ui.cursor().top()), egui::vec2(ui.max_rect().width(), box_h));
                        ui.painter().rect_filled(rep_vp, 6.0, ui.visuals().extreme_bg_color);
                        ui.allocate_exact_size(egui::vec2(rep_vp.width(), box_h), egui::Sense::hover());
                        let mut rep_scroll = ui.new_child(
                            egui::UiBuilder::new().id_salt("replace-scroll-area").max_rect(rep_vp.shrink2(egui::vec2(6.0, 4.0))).layout(egui::Layout::top_down(egui::Align::LEFT)),
                        );
                        let resp = egui::ScrollArea::vertical()
                            .id_salt("replace-scroll")
                            .auto_shrink([false, false])
                            .min_scrolled_height(rep_vp.height() - 8.0)
                            .max_height(rep_vp.height() - 8.0)
                            .show(&mut rep_scroll, |ui| ui.add(field))
                            .inner;
                        if ui.rect_contains_pointer(rep_vp) {
                            let dy = ui.input(|i| i.smooth_scroll_delta.y);
                            if dy != 0.0 {
                                let step = ((dy.abs() / ui.text_style_height(&egui::TextStyle::Body)).round() as usize).max(1);
                                let dir: isize = if dy > 0.0 { -1 } else { 1 };
                                if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), resp.id) {
                                    let char_idx = st
                                        .cursor
                                        .char_range()
                                        .map(|r| r.primary.index.0.min(r.secondary.index.0))
                                        .unwrap_or(0);
                                    let (cur_l, cur_c) = char_index_to_line_col(&self.find.replacement, char_idx);
                                    let new_l = (cur_l as isize + dir * step as isize).clamp(0, rep_lines as isize) as usize;
                                    let line_len = self.find.replacement.split('\n').nth(new_l).map(|l| l.chars().count()).unwrap_or(0);
                                    let col = cur_c.min(line_len);
                                    let new_idx = line_col_to_char_index(&self.find.replacement, new_l, col);
                                    st.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(new_idx))));
                                    st.store(ui.ctx(), resp.id);
                                }
                            }
                        }
                        let focused = resp.has_focus();
                        let hovered = ui.rect_contains_pointer(rep_vp);
                        let border = if focused {
                            ui.visuals().widgets.active.bg_stroke
                        } else if hovered {
                            ui.visuals().widgets.hovered.bg_stroke
                        } else {
                            ui.visuals().widgets.noninteractive.bg_stroke
                        };
                        ui.painter().rect_stroke(rep_vp, 6.0, border, egui::StrokeKind::Inside);
                        // Windows 慣例：純 Enter 觸發替換（還原本幀插入的 \n）；Ctrl+Enter 才插入換行
                        if ui.input(|i| i.key_pressed(Key::Enter)) && !ui.input(|i| i.modifiers.ctrl) && can_replace {
                            self.find.replacement = rep_before;
                            do_replace = true;
                            resp.request_focus();
                        }
                        // 操作區（UE 同構）：左欄＝選定文字核取方塊＋左下角齒輪；右欄按鈕縱排、
                        // 顯式 max_rect 貼內容右緣（with_layout 的 RTL 子布局拿不到剩餘寬，會擠在左欄旁）
                        let row_h = 26.0 * 3.0 + 4.0 * 2.0;
                        let row_rect = egui::Rect::from_min_size(
                            egui::pos2(full.left(), ui.cursor().top()),
                            egui::vec2(full.width(), row_h),
                        );
                        ui.allocate_exact_size(egui::vec2(full.width(), row_h), egui::Sense::hover());
                        let col_rect = egui::Rect::from_min_max(
                            egui::pos2(row_rect.right() - 104.0, row_rect.top()),
                            egui::pos2(row_rect.right(), row_rect.bottom()),
                        );
                        let col_left = egui::Rect::from_min_max(
                            egui::pos2(row_rect.left(), row_rect.top()),
                            egui::pos2(col_rect.left() - 8.0, row_rect.bottom()),
                        );
                        let mut left_ui = ui.new_child(
                            egui::UiBuilder::new().id_salt("replace-actions-left").max_rect(col_left).layout(egui::Layout::top_down(egui::Align::LEFT)),
                        );
                        left_ui.horizontal(|ui| {
                            ui.checkbox(&mut self.settings.search_in_selection, lbl_in_sel);
                        });
                        left_ui.add_space(26.0);
                        left_ui.horizontal(|ui| {
                            let gear = form::icon_button(ui, egui::Button::new("⚙").frame(false), self.t("find.options", "選項"), self.t("find.options", "選項"));
                            if gear.clicked() {
                                self.find.options_open = !self.find.options_open;
                            }
                        });
                        let mut col = ui.new_child(
                            egui::UiBuilder::new().id_salt("replace-actions-col").max_rect(col_rect).layout(egui::Layout::top_down(egui::Align::Min)),
                        );
                        col.spacing_mut().item_spacing.y = 4.0;
                        let next_btn = col.add_sized([104.0, 26.0], egui::Button::new(RichText::new(lbl_next).size(12.5)));
                        if next_btn.clicked() {
                            do_next = Some(false);
                        }
                        next_btn.on_hover_text(lbl_next_tip);
                        // scope 統一 disabled 態；add_sized 走置中布局，文字居中
                        col.scope(|ui| {
                            if !can_replace {
                                ui.disable();
                            }
                            if ui.add_sized([104.0, 26.0], egui::Button::new(RichText::new(lbl_replace_btn).size(12.5))).clicked() {
                                do_replace = true;
                            }
                            if ui.add_sized([104.0, 26.0], egui::Button::new(RichText::new(lbl_replace_all).size(12.5))).clicked() {
                                do_replace_all = true;
                            }
                        });
                    } else {
                        // 查找頁操作排（與替換頁同一套處理）：右欄＝上一個／下一個縱排貼右緣；左欄齒輪貼底
                        let row_h = 26.0 * 2.0 + 4.0;
                        let row_rect = egui::Rect::from_min_size(
                            egui::pos2(full.left(), ui.cursor().top()),
                            egui::vec2(full.width(), row_h),
                        );
                        ui.allocate_exact_size(egui::vec2(full.width(), row_h), egui::Sense::hover());
                        let col_rect = egui::Rect::from_min_max(
                            egui::pos2(row_rect.right() - 104.0, row_rect.top()),
                            egui::pos2(row_rect.right(), row_rect.bottom()),
                        );
                        let col_left = egui::Rect::from_min_max(
                            egui::pos2(row_rect.left(), row_rect.top()),
                            egui::pos2(col_rect.left() - 8.0, row_rect.bottom()),
                        );
                        let mut left_ui = ui.new_child(
                            egui::UiBuilder::new().id_salt("find-actions-left").max_rect(col_left).layout(egui::Layout::top_down(egui::Align::LEFT)),
                        );
                        // 左欄高度＝右欄（2×26＋4＝56）：齒輪與「上一個」底部對齊（24＋6間距＋26＝56）
                        left_ui.add_space(26.0);
                        left_ui.horizontal(|ui| {
                            let gear = form::icon_button(ui, egui::Button::new("⚙").frame(false), self.t("find.options", "選項"), self.t("find.options", "選項"));
                            if gear.clicked() {
                                self.find.options_open = !self.find.options_open;
                            }
                        });
                        let mut col = ui.new_child(
                            egui::UiBuilder::new().id_salt("find-actions-col").max_rect(col_rect).layout(egui::Layout::top_down(egui::Align::Min)),
                        );
                        col.spacing_mut().item_spacing.y = 4.0;
                        let prev_btn = col.add_sized([104.0, 26.0], egui::Button::new(RichText::new(lbl_prev).size(12.5)));
                        if prev_btn.clicked() {
                            do_next = Some(true);
                        }
                        prev_btn.on_hover_text(lbl_prev_tip);
                        let next_btn = col.add_sized([104.0, 26.0], egui::Button::new(RichText::new(lbl_next).size(12.5)));
                        if next_btn.clicked() {
                            do_next = Some(false);
                        }
                        next_btn.on_hover_text(lbl_next_tip);
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
                    // While a dialog is open, Esc belongs to the dialog.
                    if self.dialog.is_none() && ui.input(|i| i.key_pressed(Key::Escape)) {
                        close = true;
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
    fn test_find_in_lines_wraps_and_respects_direction() {
        let ed = EditorState::from_text("foo bar\nbar foo\nfoo");
        let re = Regex::new("foo").unwrap();
        let m = find_in_lines(&ed, &re, TextPosition::new(0, 1), false, true, None).unwrap();
        assert_eq!(m.0, TextPosition::new(1, 4));
        let m = find_in_lines(&ed, &re, TextPosition::new(2, 3), false, true, None).unwrap();
        assert_eq!(m.0, TextPosition::new(0, 0));
        assert!(find_in_lines(&ed, &re, TextPosition::new(2, 3), false, false, None).is_none());
        let m = find_in_lines(&ed, &re, TextPosition::new(1, 4), true, true, None).unwrap();
        assert_eq!(m.0, TextPosition::new(0, 0));
    }

    #[test]
    fn test_live_counts_and_keys_only_invalidate_what_changed() {
        let mut ed = EditorState::from_text("ab ab\n中ab");
        let re = Regex::new("ab").unwrap();
        let counts = count_matches_per_line(&ed.lines, &re);
        assert_eq!(counts, vec![2, 1]);
        let current = |ed: &EditorState| selected_match_index(ed, &counts, Some(&re), ed.selection.start(), ed.selection.end());
        assert_eq!(current(&ed), None);
        // Columns are characters: the match after "中" starts at byte 3 but column 1.
        ed.select_range(TextPosition::new(1, 1), TextPosition::new(1, 3));
        assert_eq!(current(&ed), Some(3));
        ed.select_range(TextPosition::new(0, 5), TextPosition::new(0, 3));
        assert_eq!(current(&ed), Some(2));
        ed.select_range(TextPosition::new(0, 0), TextPosition::new(0, 1));
        assert_eq!(current(&ed), None);
        // 移動游標（方向鍵、F3）只改變 selection key；pattern 與全文計數保持快取
        let opts = (false, false, false);
        let p = pattern_key("ab", false, opts, "UTF-8");
        assert_ne!(selection_key(TextPosition::new(0, 0), TextPosition::new(0, 1)), selection_key(TextPosition::new(1, 1), TextPosition::new(1, 3)));
        assert_eq!(counts_key(7, 3, p), counts_key(7, 3, p));
        assert_ne!(counts_key(7, 3, p), counts_key(7, 4, p), "an edit must invalidate the counts");
        assert_ne!(counts_key(7, 3, p), counts_key(8, 3, p), "another document must invalidate the counts");
        assert_ne!(p, pattern_key("ab", false, (true, false, false), "UTF-8"), "an option change must invalidate the pattern");
        assert_ne!(p, pattern_key("ab", false, opts, "BIG5"), "a different encoding must invalidate the pattern");
    }

    #[test]
    fn test_replace_all_keeps_bookmarks() {
        let mut ed = EditorState::from_text("a\nb\na\nc");
        ed.bookmarks.insert(2);
        ed.bookmarks.insert(3);
        let re = Regex::new("a").unwrap();
        // 取代文字含換行：第 0 行變兩行，之後的書籤往後移一行
        let n = replace_all_lines(&mut ed, &re, "x\ny", false, 0..1);
        assert_eq!(n, 1);
        assert_eq!(ed.lines, vec!["x", "y", "b", "a", "c"]);
        assert_eq!(ed.bookmarks.iter().copied().collect::<Vec<_>>(), vec![3, 4]);
    }

    #[test]
    fn test_multiline_search_and_replace() {
        let ed = EditorState::from_text("foo bar\nbar foo\nend");
        // 跨行模式：`bar\nbar` 只能整文比對找到
        let re = Regex::new("bar\nbar").unwrap();
        let m = find_in_text(&ed, &re, TextPosition::new(0, 0), false, true).unwrap();
        assert_eq!(m.0, TextPosition::new(0, 4));
        assert_eq!(m.1, TextPosition::new(1, 3));
        // 起始行向後：找不到時循環回首個
        let m = find_in_text(&ed, &re, TextPosition::new(1, 0), false, true).unwrap();
        assert_eq!(m.0, TextPosition::new(0, 4));
        // 計數記到起始行
        let counts = count_matches_in_text(&ed.lines, &re);
        assert_eq!(counts, vec![1, 0, 0]);
    }

    #[test]
    fn test_replace_all_multiline_and_bookmarks() {
        let mut ed = EditorState::from_text("a\nXX\nXX\nb\nXX");
        ed.bookmarks.insert(1);
        ed.bookmarks.insert(4);
        let re = Regex::new("XX\nXX").unwrap();
        // 兩行合併為一行：後方書籤上移一行，前面的書籤不動；剩餘單獨 XX 不符合
        let n = replace_all_multiline(&mut ed, &re, "M", true);
        assert_eq!(n, 1);
        assert_eq!(ed.lines, vec!["a", "M", "b", "XX"]);
        assert_eq!(ed.bookmarks.iter().copied().collect::<Vec<_>>(), vec![1, 3]);
    }
}
