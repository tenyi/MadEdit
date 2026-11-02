//! 在多個檔案中尋找／取代：背景執行緒掃描資料夾，結果顯示於下方面板

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use eframe::egui::{self, RichText};
use regex::Regex;

use super::{form, App};
use crate::core::document::EditKind;
use crate::core::editor::TextPosition;
use crate::core::encoding;

/// 一筆搜尋結果
#[derive(Clone, Debug)]
pub struct Hit {
    pub path: PathBuf,
    pub line: usize,
    /// 符合在原始整行中的欄位（點擊結果後用來選取）
    pub col: usize,
    pub len: usize,
    /// 顯示用的片段：以符合處為中心，絕不超過每行最多字元設定
    pub text: String,
    /// 符合在 `text` 片段內的欄位
    pub text_col: usize,
    /// 片段從行首縮排之前開始，顯示時加上前導「…」
    pub cut: bool,
}

impl Hit {
    /// Builds one match from a regex hit in `line_text`: the excerpt keeps `HIT_CONTEXT`
    /// characters before a match that sits far into the line and drops the indent otherwise,
    /// never exceeding `max_chars` characters. `col`/`len` stay in whole-line coordinates for
    /// selecting the match on click; `text_col` addresses the match inside the excerpt, so
    /// drawing can never clamp against an out-of-range range.
    fn new(path: PathBuf, line: usize, line_text: &str, m_start: usize, m_end: usize, max_chars: usize) -> Self {
        let col = line_text[..m_start].chars().count();
        let len = line_text[m_start..m_end].chars().count();
        let chars: Vec<char> = line_text.chars().collect();
        let indent = chars.iter().take_while(|c| c.is_whitespace()).count().min(col);
        let cut = col > indent + 2 * HIT_CONTEXT;
        let start = if cut { col - HIT_CONTEXT } else { indent };
        // The match starts inside the excerpt even when `max_chars` is smaller than the
        // context window (`col + 1 <= chars.len()`, the match is non-empty).
        let end = chars.len().min((start + max_chars).max(col + 1));
        Hit { path, line, col, len, text: chars[start..end].iter().collect(), text_col: col - start, cut }
    }
}

/// A file the replacement changed or left alone, listed under the matches.
#[derive(Clone, Debug)]
pub enum Note {
    Replaced { path: PathBuf, count: usize },
    Skipped { path: PathBuf, reason: Skip },
    Failed { path: PathBuf, error: String },
}

/// Why a replacement left a file alone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Skip {
    /// Its text could not be decoded losslessly.
    Undecodable,
    /// It was opened in the editor meanwhile.
    Opened,
    /// It is open in the editor as read-only.
    ReadOnly,
    /// The replaced text cannot be saved in its encoding.
    Unencodable,
}

/// What the last search is doing or did, for the results header.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Status {
    #[default]
    Idle,
    Running { replacing: bool },
    /// `files` is how many files were scanned, or None when only the opened files were searched.
    Done { files: Option<usize>, replacing: bool },
}

enum Msg {
    Hit(Hit),
    Note(Note),
    Done { files: usize, replacing: bool },
}

#[derive(Default)]
pub struct FindInFiles {
    pub show_results: bool,
    pub results: Vec<Hit>,
    pub notes: Vec<Note>,
    pub status: Status,
    /// Occurrences replaced so far, in opened documents and on disk.
    pub replaced: usize,
    /// Set when the user stopped the search before it finished.
    pub stopped: bool,
    /// Hides every file's matches; files in `folded` show the opposite of it.
    pub collapsed: bool,
    pub folded: HashSet<PathBuf>,
    rx: Option<Receiver<Msg>>,
    pub running: bool,
    /// 已在編輯器開啟的檔案（正規化路徑），背景取代不得寫入；搜尋期間新開的檔案也會加入
    opened: Arc<Mutex<HashSet<PathBuf>>>,
    /// 要求背景執行緒在處理下一個檔案前停止
    cancel: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl FindInFiles {
    /// 檔案在編輯器中開啟時呼叫，避免執行中的背景取代覆寫它
    pub fn mark_opened(&self, canonical: &Path) {
        self.opened.lock().unwrap_or_else(|e| e.into_inner()).insert(canonical.to_path_buf());
    }

    /// 程式結束前呼叫：要求背景執行緒停止並等它寫完目前的檔案，避免寫入途中被中止
    pub fn stop_worker(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

/// 正規化路徑（解析符號連結、相對路徑與 macOS 的 /tmp 等別名），失敗時沿用原路徑
fn canonical(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

/// 簡單的萬用字元比對（* 與 ?，不分大小寫），多個樣式以 ; 或空白分隔
pub fn wildcard_match(patterns: &str, name: &str) -> bool {
    let pats: Vec<&str> = patterns.split([';', ' ', ',']).filter(|p| !p.is_empty()).collect();
    if pats.is_empty() {
        return true;
    }
    let name = name.to_lowercase();
    pats.iter().any(|p| {
        let re = format!("^{}$", regex::escape(&p.to_lowercase()).replace(r"\*", ".*").replace(r"\?", "."));
        Regex::new(&re).is_ok_and(|r| r.is_match(&name))
    })
}

fn collect_files(dir: &Path, include: &str, exclude: &str, recursive: bool, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if p.is_dir() {
            if recursive && !(!exclude.is_empty() && wildcard_match(exclude, &name)) {
                collect_files(&p, include, exclude, recursive, out);
            }
        } else if wildcard_match(include, &name) && !(!exclude.is_empty() && wildcard_match(exclude, &name)) {
            out.push(p);
        }
    }
}

impl App {
    /// 開始搜尋（`replace` 為 Some 時同時取代）
    pub fn start_find_in_files(&mut self, query: String, replace: Option<String>) {
        let re = match crate::core::search::build_regex(&query, &self.search_options()) {
            Ok(r) => r,
            Err(e) => {
                self.message(self.t("fif.bad_query", "搜尋條件錯誤"), e);
                return;
            }
        };
        let use_regex = self.settings.search_regex;
        let opts = self.settings.find_in_files.clone();
        self.fif.cancel.store(true, Ordering::Relaxed);
        self.fif.results.clear();
        self.fif.notes.clear();
        self.fif.folded.clear();
        self.fif.stopped = false;
        self.fif.show_results = true;
        self.fif.collapsed = false;
        let max_chars = self.settings.max_search_result_chars.max(20);

        // 已開啟的文件直接在記憶體中處理（取代可復原）；兩側路徑都正規化後再比對
        let dir_canon = canonical(Path::new(&opts.directory));
        let mut opened: HashSet<PathBuf> = HashSet::new();
        let mut replaced_total = 0;
        for t in &mut self.tabs {
            // 未命名文件沒有路徑，結果無法定位，略過
            let Some(p) = t.doc.path.clone() else { continue };
            let in_scope = opts.opened_files_only
                || (canonical(&p).starts_with(&dir_canon) && wildcard_match(&opts.file_filter, &p.file_name().unwrap_or_default().to_string_lossy()));
            if !in_scope {
                continue;
            }
            opened.insert(canonical(&p));
            if let Some(rep) = &replace {
                if t.doc.read_only {
                    if t.doc.editor.lines.iter().any(|line| re.find_iter(line).any(|m| m.start() != m.end())) {
                        self.fif.notes.push(Note::Skipped { path: p, reason: Skip::ReadOnly });
                    }
                } else {
                    let hint = t.doc.whole_hint();
                    let all = 0..t.doc.editor.lines.len();
                    let n = t.doc.edit(EditKind::Other, hint, |ed| super::find::replace_all_lines(ed, &re, rep, use_regex, all));
                    if n > 0 {
                        replaced_total += n;
                        self.fif.notes.push(Note::Replaced { path: p, count: n });
                    }
                }
            } else {
                for (li, line) in t.doc.editor.lines.iter().enumerate() {
                    for m in re.find_iter(line).filter(|m| m.start() != m.end()) {
                        self.fif.results.push(Hit::new(p.clone(), li, line, m.start(), m.end(), max_chars));
                    }
                }
            }
        }
        self.fif.replaced = replaced_total;
        if opts.opened_files_only {
            self.fif.status = Status::Done { files: None, replacing: replace.is_some() };
            return;
        }

        let dir = PathBuf::from(&opts.directory);
        if !dir.is_dir() {
            self.fif.status = Status::Idle;
            let text = self.tf("fif.no_folder", "資料夾不存在：{q}", None, Some(&opts.directory));
            self.message(self.t("dlg.find_files", "在檔案中尋找／取代"), text);
            return;
        }
        crate::core::settings::Settings::push_history(&mut self.settings.recent_dirs, opts.directory.clone());
        let default_enc = encoding::encoding_for_name(&self.settings.default_encoding).unwrap_or(encoding_rs::UTF_8);
        let fallback_enc = encoding::encoding_for_name(&self.settings.fallback_encoding).unwrap_or(encoding_rs::BIG5);
        let (tx, rx) = channel();
        let opened = Arc::new(Mutex::new(opened));
        self.fif.opened = Arc::clone(&opened);
        let cancel = Arc::new(AtomicBool::new(false));
        self.fif.cancel = Arc::clone(&cancel);
        self.fif.rx = Some(rx);
        self.fif.running = true;
        self.fif.status = Status::Running { replacing: replace.is_some() };
        let replacing = replace.is_some();
        self.fif.worker = Some(std::thread::spawn(move || {
            let mut files = Vec::new();
            collect_files(&dir, &opts.file_filter, &opts.exclude_filter, opts.subdirectories, &mut files);
            let is_opened = |f: &Path| opened.lock().unwrap_or_else(|e| e.into_inner()).contains(&canonical(f));
            let mut scanned = 0;
            for f in files {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                scanned += 1;
                if is_opened(&f) {
                    continue;
                }
                let Ok(bytes) = std::fs::read(&f) else { continue };
                if bytes.len() > 64 * 1024 * 1024 || encoding::looks_binary(&bytes) {
                    continue;
                }
                let d = encoding::detect_and_decode(&bytes, default_enc, fallback_enc, false);
                match &replace {
                    None => {
                        for (li, line) in d.text.lines().enumerate() {
                            for m in re.find_iter(line).filter(|m| m.start() != m.end()) {
                                let _ = tx.send(Msg::Hit(Hit::new(f.clone(), li, line, m.start(), m.end(), max_chars)));
                            }
                        }
                    }
                    Some(rep) => {
                        if d.had_errors {
                            let _ = tx.send(Msg::Note(Note::Skipped { path: f.clone(), reason: Skip::Undecodable }));
                            continue;
                        }
                        let mut count = 0;
                        let new_text = re.replace_all(&d.text, |caps: &regex::Captures| {
                            count += 1;
                            if use_regex {
                                let mut s = String::new();
                                caps.expand(rep, &mut s);
                                s
                            } else {
                                rep.clone()
                            }
                        });
                        if count == 0 {
                            continue;
                        }
                        match encoding::encode_text(&new_text, d.encoding, d.has_bom) {
                            Ok(out) => {
                                // 寫入前再確認一次（搜尋期間可能新開了這個檔案）；寫入期間持有鎖，
                                // 讓同時開檔的 UI 等到寫完才讀檔
                                let guard = opened.lock().unwrap_or_else(|e| e.into_inner());
                                if guard.contains(&canonical(&f)) {
                                    let _ = tx.send(Msg::Note(Note::Skipped { path: f.clone(), reason: Skip::Opened }));
                                    continue;
                                }
                                let note = match crate::core::document::atomic_write(&f, |w| std::io::Write::write_all(w, &out)) {
                                    Err(e) => Note::Failed { path: f.clone(), error: e.to_string() },
                                    Ok(()) => Note::Replaced { path: f.clone(), count },
                                };
                                let _ = tx.send(Msg::Note(note));
                            }
                            Err(_) => {
                                let _ = tx.send(Msg::Note(Note::Skipped { path: f.clone(), reason: Skip::Unencodable }));
                            }
                        }
                    }
                }
            }
            let _ = tx.send(Msg::Done { files: scanned, replacing });
        }));
    }

    pub fn find_results_panel(&mut self, ui: &mut egui::Ui) {
        let fif = &self.fif;
        let hits = fif.results.len();
        let mut parts = Vec::new();
        if fif.stopped {
            parts.push(self.t("fif.stopped", "已停止").to_string());
        }
        match fif.status {
            Status::Idle => {}
            Status::Running { replacing: true } => parts.push(self.t("fif.replacing", "取代中…").to_string()),
            Status::Running { replacing: false } => parts.push(format!("{} {}", self.t("fif.searching", "搜尋中…"), self.tf("fif.hits", "{n} 筆結果", Some(hits), None))),
            Status::Done { files: Some(files), replacing: true } => parts.push(self.tf("fif.scanned", "共掃描 {n} 個檔案", Some(files), None)),
            Status::Done { files: Some(files), replacing: false } => parts.push(self.tf("fif.found", "{n} 筆結果，共掃描 {q} 個檔案", Some(hits), Some(&files.to_string()))),
            Status::Done { files: None, replacing: true } => {}
            Status::Done { files: None, replacing: false } => parts.push(self.tf("fif.found_opened", "在開啟的檔案中找到 {n} 筆", Some(hits), None)),
        }
        if fif.replaced > 0 || matches!(fif.status, Status::Done { replacing: true, .. }) {
            parts.push(self.tf("fif.replaced_n", "已取代 {n} 處", Some(fif.replaced), None));
        }
        let status = parts.join(" · ");
        let l_title = self.t("fif.results", "搜尋結果");
        let l_stop = self.t("fif.stop", "停止");
        let l_clear = self.t("opt.clear", "清除");
        let l_close = self.t("btn.close", "關閉");
        let l_none = self.t("find.no_results", "無結果");
        let l_collapse = super::commands::plain_label(&self.cmd_text(super::commands::C::CollapseAllResults));
        let note_text = |note: &Note| -> (String, bool) {
            match note {
                Note::Replaced { count, .. } => (self.tf("fif.replaced_n", "已取代 {n} 處", Some(*count), None), false),
                Note::Skipped { reason: Skip::Undecodable, .. } => (self.t("fif.skip_decode", "略過：無法正確解碼").to_string(), true),
                Note::Skipped { reason: Skip::Opened, .. } => (self.t("fif.skip_opened", "略過：已在編輯器中開啟").to_string(), true),
                Note::Skipped { reason: Skip::ReadOnly, .. } => (self.t("fif.skip_readonly", "略過：唯讀").to_string(), true),
                Note::Skipped { reason: Skip::Unencodable, .. } => (self.t("fif.skip_encode", "略過：取代後的文字無法以原編碼儲存").to_string(), true),
                Note::Failed { error, .. } => (error.clone(), true),
            }
        };
        let notes: Vec<(PathBuf, String, bool)> = fif
            .notes
            .iter()
            .map(|n| {
                let (Note::Replaced { path, .. } | Note::Skipped { path, .. } | Note::Failed { path, .. }) = n;
                let (text, warn) = note_text(n);
                (path.clone(), text, warn)
            })
            .collect();
        let running = fif.running;
        let mut open: Option<Hit> = None;
        let mut open_file: Option<PathBuf> = None;
        let mut fold: Option<PathBuf> = None;
        // 清除動作延後到本幀繪製結束：本幀開頭保存的 hits 數目仍對應尚在顯示的結果
        let mut clear_requested = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new(l_title).strong());
            if running {
                ui.add(egui::Spinner::new());
            }
            ui.label(RichText::new(&status).weak());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if form::icon_button(ui, egui::Button::new("×").frame(false), l_close, l_close).clicked() {
                    self.fif.show_results = false;
                }
                if ui.add_enabled(!running && (hits > 0 || !notes.is_empty()), egui::Button::new(l_clear)).clicked() {
                    clear_requested = true;
                }
                let icon = if self.fif.collapsed { "▸" } else { "▾" };
                if form::icon_button(ui, egui::Button::new(icon), &l_collapse, &l_collapse).clicked() {
                    self.fif.toggle_collapsed();
                }
                if running && ui.button(l_stop).clicked() {
                    self.fif.stop();
                }
            });
        });
        ui.separator();
        let fif = &self.fif;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if hits == 0 && notes.is_empty() && matches!(fif.status, Status::Done { .. }) {
                ui.add_space(16.0);
                ui.vertical_centered(|ui| ui.label(RichText::new(l_none).weak()));
            }
            let mut i = 0;
            while i < hits {
                let path = &fif.results[i].path;
                let n = fif.results[i..].iter().take_while(|h| &h.path == path).count();
                let folded = fif.collapsed != fif.folded.contains(path);
                if file_row(ui, path, n, folded).clicked() {
                    fold = Some(path.clone());
                }
                if !folded {
                    for h in &fif.results[i..i + n] {
                        if hit_row(ui, h).clicked() {
                            open = Some(h.clone());
                        }
                    }
                }
                i += n;
            }
            if !notes.is_empty() {
                if hits > 0 {
                    ui.add_space(6.0);
                }
                for (path, text, warn) in &notes {
                    if note_row(ui, path, text, *warn).clicked() {
                        open_file = Some(path.clone());
                    }
                }
            }
        });
        if let Some(path) = fold
            && !self.fif.folded.remove(&path)
        {
            self.fif.folded.insert(path);
        }
        if let Some(path) = open_file {
            self.open_path(&path);
        }
        if let Some(h) = open
            && self.open_path(&h.path)
        {
            let doc = self.doc_mut();
            let from = doc.editor.selection.head;
            doc.record_jump(from);
            doc.editor.select_range(TextPosition::new(h.line, h.col), TextPosition::new(h.line, h.col + h.len));
            self.tab_mut().view.ensure_visible = true;
            self.focus_editor = true;
        }
        if clear_requested {
            self.fif.results.clear();
            self.fif.notes.clear();
            self.fif.replaced = 0;
            self.fif.status = Status::Idle;
        }
    }
}

/// Height of a results row.
const RESULT_ROW_HEIGHT: f32 = 22.0;

/// A full-width clickable results row with a hover background, announced as `name`.
fn result_row(ui: &mut egui::Ui, name: &str) -> (egui::Rect, egui::Response) {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), RESULT_ROW_HEIGHT), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Button, ui.is_enabled(), name));
    if response.hovered() {
        ui.painter().rect_filled(rect, 4.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    (rect, response)
}

/// A file's heading in the results: a fold arrow, the file name, its folder dimmed and the
/// number of matches.
fn file_row(ui: &mut egui::Ui, path: &Path, count: usize, folded: bool) -> egui::Response {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
    let folder = path.parent().map(super::dialogs::tilde_path).unwrap_or_default();
    let (rect, response) = result_row(ui, &format!("{name} ({count})"));
    if ui.is_rect_visible(rect) {
        let visuals = ui.visuals();
        let painter = ui.painter();
        let y = rect.center().y;
        let mut x = rect.left() + 6.0;
        let arrow = painter.text(egui::pos2(x, y), egui::Align2::LEFT_CENTER, if folded { "▸" } else { "▾" }, egui::FontId::proportional(12.0), visuals.weak_text_color());
        x = arrow.right() + 6.0;
        let badge = count.to_string();
        let badge_galley = painter.layout_no_wrap(badge, egui::FontId::proportional(11.0), visuals.strong_text_color());
        let badge_rect = egui::Rect::from_center_size(egui::pos2(rect.right() - 8.0 - (badge_galley.size().x + 12.0) / 2.0, y), badge_galley.size() + egui::vec2(12.0, 2.0));
        painter.rect_filled(badge_rect, badge_rect.height() / 2.0, visuals.widgets.inactive.weak_bg_fill);
        painter.galley(badge_rect.center() - badge_galley.size() / 2.0, badge_galley, visuals.strong_text_color());
        let room = (badge_rect.left() - 8.0 - x).max(0.0);
        let mut job = egui::text::LayoutJob { wrap: egui::text::TextWrapping::truncate_at_width(room), ..Default::default() };
        let body = egui::TextStyle::Body.resolve(ui.style());
        job.append(&name, 0.0, egui::TextFormat::simple(body.clone(), visuals.strong_text_color()));
        job.append(&folder, 10.0, egui::TextFormat::simple(body, visuals.weak_text_color()));
        let galley = painter.layout_job(job);
        painter.galley(egui::pos2(x, y - galley.size().y / 2.0), galley, visuals.text_color());
    }
    response
}

/// Characters of context kept before a match that starts far into its line, so the match is
/// still in view.
const HIT_CONTEXT: usize = 24;

/// Splits a match's line into the text before, inside and after the match; the coordinates
/// are the excerpt-relative ones recorded in the `Hit`, so stale or oversized values only
/// truncate the display and can never clamp against an invalid range.
fn excerpt(hit: &Hit) -> (bool, String, String, String) {
    let chars: Vec<char> = hit.text.trim_end().chars().collect();
    let col = hit.text_col.min(chars.len());
    let end = col.saturating_add(hit.len).min(chars.len());
    (hit.cut, chars[..col].iter().collect(), chars[col..end].iter().collect(), chars[end..].iter().collect())
}

/// One match: its line number dimmed in a gutter, then the line with the match highlighted.
fn hit_row(ui: &mut egui::Ui, hit: &Hit) -> egui::Response {
    let (cut, before, matched, after) = excerpt(hit);
    let line = (hit.line + 1).to_string();
    let (rect, response) = result_row(ui, &format!("{line}: {before}{matched}{after}"));
    if ui.is_rect_visible(rect) {
        let visuals = ui.visuals();
        let painter = ui.painter();
        let mono = egui::TextStyle::Monospace.resolve(ui.style());
        let gutter = rect.left() + 24.0 + 6.0 * mono.size * 0.6;
        let y = rect.center().y;
        painter.text(egui::pos2(gutter, y), egui::Align2::RIGHT_CENTER, &line, mono.clone(), visuals.weak_text_color());
        let x = gutter + 12.0;
        let mut job = egui::text::LayoutJob { wrap: egui::text::TextWrapping::truncate_at_width((rect.right() - 6.0 - x).max(0.0)), ..Default::default() };
        let normal = egui::TextFormat::simple(mono, visuals.text_color());
        let highlight = if visuals.dark_mode { egui::Color32::from_rgba_unmultiplied(255, 170, 0, 90) } else { egui::Color32::from_rgba_unmultiplied(255, 190, 0, 140) };
        if cut {
            job.append("…", 0.0, normal.clone());
        }
        job.append(&before, 0.0, normal.clone());
        job.append(&matched, 0.0, egui::TextFormat { background: highlight, color: visuals.strong_text_color(), ..normal.clone() });
        job.append(&after, 0.0, normal);
        let galley = painter.layout_job(job);
        painter.galley(egui::pos2(x, y - galley.size().y / 2.0), galley, visuals.text_color());
    }
    response
}

/// A file the replacement changed or skipped, with what happened to it.
fn note_row(ui: &mut egui::Ui, path: &Path, text: &str, warn: bool) -> egui::Response {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
    let (rect, response) = result_row(ui, &format!("{name}: {text}"));
    if ui.is_rect_visible(rect) {
        let visuals = ui.visuals();
        let body = egui::TextStyle::Body.resolve(ui.style());
        let mut job = egui::text::LayoutJob { wrap: egui::text::TextWrapping::truncate_at_width((rect.width() - 12.0).max(0.0)), ..Default::default() };
        job.append(&name, 0.0, egui::TextFormat::simple(body.clone(), visuals.strong_text_color()));
        let color = if warn { visuals.warn_fg_color } else { visuals.weak_text_color() };
        job.append(text, 10.0, egui::TextFormat::simple(body, color));
        let galley = ui.painter().layout_job(job);
        ui.painter().galley(egui::pos2(rect.left() + 6.0, rect.center().y - galley.size().y / 2.0), galley, visuals.text_color());
    }
    response.on_hover_text(path.display().to_string())
}

impl FindInFiles {
    /// 接收背景執行緒的結果
    pub fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.rx else { return };
        loop {
            match rx.try_recv() {
                Ok(Msg::Hit(h)) => self.results.push(h),
                Ok(Msg::Note(note)) => {
                    if let Note::Replaced { count, .. } = note {
                        self.replaced += count;
                    }
                    self.notes.push(note);
                }
                Ok(Msg::Done { files, replacing }) => {
                    self.running = false;
                    self.status = Status::Done { files: Some(files), replacing };
                    self.rx = None;
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                    break;
                }
                Err(_) => {
                    self.running = false;
                    let replacing = matches!(self.status, Status::Running { replacing: true });
                    self.status = Status::Done { files: None, replacing };
                    self.rx = None;
                    break;
                }
            }
        }
    }

    /// Folds or unfolds every file, dropping the files folded one by one.
    pub fn toggle_collapsed(&mut self) {
        self.collapsed = !self.collapsed;
        self.folded.clear();
    }

    /// Asks the background search to stop before its next file.
    pub fn stop(&mut self) {
        if self.running {
            self.cancel.store(true, Ordering::Relaxed);
            self.stopped = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wildcard_match() {
        assert!(wildcard_match("*.rs;*.toml", "main.rs"));
        assert!(wildcard_match("*.RS", "main.rs"));
        assert!(!wildcard_match("*.rs", "main.rsx"));
        assert!(wildcard_match("", "x"));
        assert!(wildcard_match("a?c", "abc"));
    }

    #[test]
    fn test_excerpt_drops_the_indent_and_keeps_far_matches_in_view() {
        let hit = |line: &str, start, end, max| Hit::new(PathBuf::new(), 0, line, start, end, max);
        let parts = |cut, a: &str, b: &str, c: &str| (cut, a.to_owned(), b.to_owned(), c.to_owned());
        // "    let 中文 = x;  ": the match starts at byte 8 (column 8, 中文 is two characters).
        assert_eq!(excerpt(&hit("    let 中文 = x;  ", 8, 14, 100)), parts(false, "let ", "中文", " = x;"));
        let far = format!("{}needle tail", "中".repeat(60));
        assert_eq!(excerpt(&hit(&far, 180, 186, 100)), parts(true, &"中".repeat(HIT_CONTEXT), "needle", " tail"));
        assert_eq!(excerpt(&hit("ab", 0, 2, 100)), parts(false, "", "ab", ""));
    }

    #[test]
    fn test_excerpt_contains_matches_beyond_the_char_budget() {
        // A match at column 150 with the default 100-character budget: the excerpt used to be
        // cut from the line start, so highlighting clamped an invalid range and panicked.
        let line = format!("{}needle tail", "x".repeat(150));
        let hit = Hit::new(PathBuf::new(), 0, &line, 150, 156, 100);
        assert_eq!((hit.col, hit.len, hit.cut), (150, 6, true));
        let (cut, before, matched, after) = excerpt(&hit);
        assert!(cut);
        assert_eq!(before.chars().count(), HIT_CONTEXT);
        assert_eq!((matched.as_str(), after.as_str()), ("needle", " tail"));
        // Clicking the result still selects the original whole-line columns.
        assert_eq!(hit.text_col, HIT_CONTEXT);
        assert_eq!(hit.text.chars().count(), HIT_CONTEXT + "needle tail".chars().count());
        // A match longer than the budget truncates the highlight instead of panicking.
        let parts = |cut, a: &str, b: &str, c: &str| (cut, a.to_owned(), b.to_owned(), c.to_owned());
        let long = format!("{}{}", "y".repeat(5), "a".repeat(200));
        let hit = Hit::new(PathBuf::new(), 0, &long, 5, 205, 100);
        let (cut, before, matched, after) = excerpt(&hit);
        assert!(!cut);
        assert_eq!((before.as_str(), matched.chars().count(), after.as_str()), ("yyyyy", 95, ""));
        // Stale excerpt coordinates must only truncate the display.
        let stale = Hit { path: PathBuf::new(), line: 0, col: 5, len: 3, text: "ab".into(), text_col: 5, cut: false };
        assert_eq!(excerpt(&stale), parts(false, "ab", "", ""));
    }
}
