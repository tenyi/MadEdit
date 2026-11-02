//! 搜尋／取代列與跳行列：文字模式以行為單位比對（Regex），十六進位模式比對位元組

use std::hash::{Hash, Hasher};

use eframe::egui::{self, Key, RichText};
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
    pub(crate) live: LiveSearch,
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

/// Matches per line for the live count; summing gives the document total.
fn count_matches_per_line(lines: &[String], re: &Regex) -> Vec<usize> {
    lines.iter().map(|line| re.find_iter(line).filter(|m| m.start() != m.end()).count()).collect()
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
        let doc = self.doc_mut();
        let sel = doc.editor.selection;
        let range = None;
        let from = if backward { sel.start() } else { sel.end() };
        let found = find_in_lines(&doc.editor, &re, from, backward, wrap, range);
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
                        live.regex = Some(re);
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
        let doc = self.doc_mut();
        let sel = doc.editor.selection;
        let count = if in_sel && !sel.is_empty() && !sel.is_block() {
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
        // Matches are highlighted in the editor only while the bar is open.
        if self.find.visible {
            self.update_live_search();
        }
        let highlight = if self.find.visible { self.find.live.regex.clone().map(|re| (re, self.find.live.pattern_id)) } else { None };
        self.tab_mut().view.find_regex = highlight;
        if !self.find.visible {
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
        let lbl_prev = with_keys(plain_label(&self.cmd_text(C::FindPrevious)), C::FindPrevious);
        let lbl_next = with_keys(plain_label(&self.cmd_text(C::FindNext)), C::FindNext);
        let lbl_case = self.t("fif.case", "區分大小寫");
        let lbl_word = self.t("fif.whole_word", "全字比對");
        let lbl_regex = self.t("fif.regex", "使用正規表示式（Regex）");
        let lbl_hex = self.t("find.hex_hint", "十六進位，例如 4D 61 64");
        let lbl_wrap = self.t("find.wrap", "循環");
        let lbl_toggle = self.t("find.replace_toggle", "取代…");
        let lbl_history = self.t("find.history", "搜尋紀錄");
        let lbl_close = self.t("find.close", "關閉 (Esc)");
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
        bar_frame(ui).show(ui, |ui| {
            bar_style(ui);
            let mut field_x = 0.0;
            ui.horizontal_wrapped(|ui| {
                let icon = if self.find.replace { "▾" } else { "▸" };
                if form::icon_button(ui, egui::Button::new(icon).frame(false), lbl_toggle, lbl_toggle).clicked() {
                    self.find.replace = !self.find.replace;
                }
                let hint = if self.find.hex { lbl_hex_hint.clone() } else { lbl_find.clone() };
                let field = egui::TextEdit::singleline(&mut self.find.query).desired_width(260.0).prefix(RichText::new("🔍").weak()).hint_text(hint);
                let resp = ui.add(field);
                field_x = resp.rect.left() - ui.min_rect().left();
                if failed {
                    ui.painter().rect_stroke(resp.rect, 6.0, egui::Stroke::new(1.0, ui.visuals().error_fg_color), egui::StrokeKind::Inside);
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
                if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    do_next = Some(ui.input(|i| i.modifiers.shift));
                    resp.request_focus();
                }
                if !self.settings.recent_finds.is_empty() {
                    let combo = egui::ComboBox::from_id_salt("find-history").selected_text("").width(0.0).show_ui(ui, |ui| {
                        for h in self.settings.recent_finds.clone() {
                            if ui.selectable_label(false, &h).clicked() {
                                self.find.query = h;
                            }
                        }
                    });
                    combo.response.on_hover_text(lbl_history);
                }
                form::toggle_chip(ui, &mut self.settings.search_match_case, "Aa", lbl_case);
                form::toggle_chip(ui, &mut self.settings.search_whole_word, "ab", lbl_word);
                form::toggle_chip(ui, &mut self.settings.search_regex, ".*", lbl_regex);
                form::toggle_chip(ui, &mut self.find.hex, "0x", lbl_hex);
                form::toggle_chip(ui, &mut self.settings.search_wrap_around, "⟲", lbl_wrap);
                if let Some((text, bad)) = &status {
                    let text = RichText::new(text.as_str());
                    let label = ui.label(if *bad { text.color(ui.visuals().error_fg_color) } else { text.weak() });
                    if let Some(detail) = &error_detail {
                        label.on_hover_text(detail.as_str());
                    }
                }
                if form::icon_button(ui, egui::Button::new("↑"), &lbl_prev, &lbl_prev).clicked() {
                    do_next = Some(true);
                }
                if form::icon_button(ui, egui::Button::new("↓"), &lbl_next, &lbl_next).clicked() {
                    do_next = Some(false);
                }
                // The live counter already says when nothing matches.
                if let Some(info) = self.find.info.as_deref().filter(|i| !i.is_empty() && (hex_doc || *i != lbl_not_found)) {
                    ui.label(RichText::new(info).weak());
                }
                if form::icon_button(ui, egui::Button::new("×").frame(false), lbl_close, lbl_close).clicked() {
                    close = true;
                }
            });
            if self.find.replace {
                ui.horizontal_wrapped(|ui| {
                    // Lined up under the find field.
                    ui.add_space((field_x - ui.spacing().item_spacing.x).max(0.0));
                    let resp = ui.add(egui::TextEdit::singleline(&mut self.find.replacement).desired_width(260.0).hint_text(lbl_replace.clone()));
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && can_replace {
                        do_replace = true;
                        resp.request_focus();
                    }
                    if !self.settings.recent_replaces.is_empty() {
                        let combo = egui::ComboBox::from_id_salt("replace-history").selected_text("").width(0.0).show_ui(ui, |ui| {
                            for h in self.settings.recent_replaces.clone() {
                                if ui.selectable_label(false, &h).clicked() {
                                    self.find.replacement = h;
                                }
                            }
                        });
                        combo.response.on_hover_text(lbl_history);
                    }
                    if ui.add_enabled(can_replace, egui::Button::new(lbl_replace_btn)).clicked() {
                        do_replace = true;
                    }
                    if ui.add_enabled(can_replace, egui::Button::new(lbl_replace_all)).clicked() {
                        do_replace_all = true;
                    }
                    form::toggle_chip(ui, &mut self.settings.search_in_selection, lbl_in_sel, lbl_in_sel);
                });
            }
            // While a dialog is open, Esc belongs to the dialog.
            if self.dialog.is_none() && ui.input(|i| i.key_pressed(Key::Escape)) {
                close = true;
            }
        });
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
}
