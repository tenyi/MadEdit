//! Tools > Compare Files: loads two files and shows the side-by-side result (text rows or hex
//! rows) in its own window. The diff itself lives in `core::compare`.

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

use eframe::egui::{self, pos2, vec2, Color32, FontFamily, FontId, Rect, Sense, Stroke, Vec2};

use super::fonts::EDITOR_FAMILY;
use super::i18n::{self, Lang};
use super::{form, same_target, App};
use crate::core::compare::{self, BinaryDiff, Eol, RowKind, TextDiff, TextOptions};
use crate::core::document::{EditKind, OpenOptions};
use crate::core::editor::{char_visual_columns, EditorState, Selection, TextPosition};
use crate::core::encoding;
use crate::core::model::{EditMode, NewLineFormat};

/// Seconds a compared tab must stay unchanged before the window compares again.
const REFRESH_DELAY: f64 = 0.3;

/// Both files are held in memory, so larger ones are refused.
const MAX_COMPARE_SIZE: u64 = 256 * 1024 * 1024;
const BYTES_PER_ROW: usize = 16;
/// Only the start of a very long line is drawn; the diff still compares all of it.
const MAX_DRAWN_CHARS: usize = 2000;

const CHANGED: Color32 = Color32::from_rgba_premultiplied(60, 48, 0, 60);
const DELETED: Color32 = Color32::from_rgba_premultiplied(64, 16, 16, 64);
const INSERTED: Color32 = Color32::from_rgba_premultiplied(14, 50, 22, 64);
const MOVED: Color32 = Color32::from_rgba_premultiplied(24, 40, 80, 80);
const FILLER: Color32 = Color32::from_rgba_premultiplied(30, 30, 30, 30);
/// Binary: shifted bytes shown beside their counterpart.
const SHIFTED: Color32 = Color32::from_rgba_premultiplied(10, 16, 32, 32);
/// The changed words inside a changed line.
const CHANGED_WORDS: Color32 = Color32::from_rgba_premultiplied(95, 72, 0, 95);
const MOVED_WORDS: Color32 = Color32::from_rgba_premultiplied(30, 56, 110, 110);

/// Width of the strip of links between the panes, and of the overview at the right.
const LINK_W: f32 = 44.0;
const MAP_W: f32 = 16.0;
/// Word highlights of at most this many rows are kept.
const INLINE_CACHE_ROWS: usize = 4096;

/// Why a comparison could not start; translated when shown.
#[derive(Debug)]
pub enum CompareError {
    Read(PathBuf, String),
    TooLarge(PathBuf),
}

enum Content {
    Text {
        lines: [Vec<String>; 2],
        /// The terminator after each line, for comparing newline styles; `None` where they
        /// are not known (an open tab with mixed newlines), so they are not compared.
        eols: [Option<Vec<Eol>>; 2],
        options: TextOptions,
        diff: TextDiff,
        widths: [usize; 2],
        /// Per side, [`TextDiff::line_bounds`]: each pane is drawn without filler lines.
        bounds: [Vec<usize>; 2],
        /// Changed words of the changed rows drawn so far, by row.
        inline: HashMap<usize, [Vec<Range<usize>>; 2]>,
    },
    /// `moved[1]` holds the target ranges, sorted, for the right side.
    Binary {
        bytes: [Vec<u8>; 2],
        diff: BinaryDiff,
        moved: [Vec<Range<usize>>; 2],
        /// The overview's marks: runs of differing display cells and their colour.
        marks: Vec<(Range<usize>, Color32)>,
    },
}

impl Content {
    /// Compares the text again under its options.
    fn rediff(&mut self) {
        let Content::Text { lines, eols, options, diff, bounds, inline, .. } = self else { return };
        *diff = compare::diff_lines_with(&lines[0], &lines[1], [eols[0].as_deref(), eols[1].as_deref()], *options);
        *bounds = [diff.line_bounds(false), diff.line_bounds(true)];
        inline.clear();
    }
}

/// The compare window's state.
pub struct CompareView {
    paths: [PathBuf; 2],
    names: [String; 2],
    /// The "compare as binary" option the comparison was started with.
    binary: bool,
    content: Content,
    /// Text: index of the current hunk.
    hunk: Option<usize>,
    /// Binary: the current run of differing bytes.
    run: Option<Range<usize>>,
    /// Binary: scroll offset shared by both panes.
    offset: Vec2,
    /// Text: first visible row of the aligned rows (fractional), shared by both panes, and the
    /// horizontal scroll in points.
    scroll: f32,
    hscroll: f32,
    /// Encoding and newline style of each side, shown under it.
    info: [String; 2],
    /// Row to bring into view on the next frame.
    goto_row: Option<usize>,
    /// Widest text line drawn so far, in points: measured glyph advances can exceed the
    /// column estimate (CJK fallback fonts), and the horizontal scroll must reach the end.
    drawn_w: f32,
    /// Keyboard input belongs to this window this frame (see [`CompareView::ui`]).
    keys: bool,
    /// Where the window was last drawn, for [`CompareView::takes_keys`].
    rect: Rect,
    /// The toolbar's buttons: their keyboard focus does not stop the panes' keys.
    toolbar: Vec<egui::Id>,
    /// Document version of each side's text tab when compared (`None`: not open), so edits
    /// and undo there compare again.
    pub(super) sources: [Option<u64>; 2],
    /// Each side's path resolved once (canonical where it exists), so finding its tab every
    /// frame compares paths instead of asking the file system.
    resolved: [PathBuf; 2],
    /// Rows the text panes showed last frame, to tell whether a difference is in view.
    visible: f32,
    /// When a compared tab last changed (seconds, egui time), and the versions seen then: the
    /// window compares again once they have stayed the same for `REFRESH_DELAY`.
    changed_at: Option<f64>,
    seen: [Option<u64>; 2],
}

/// A merge of the current difference: on `side`, lines `target` (now `expected`) become
/// `source`.
#[derive(Debug, PartialEq, Eq)]
pub struct MergePlan {
    pub side: usize,
    pub target: Range<usize>,
    pub expected: Vec<String>,
    /// The other side's lines (`source`) at `source_range`.
    pub source_range: Range<usize>,
    pub source: Vec<String>,
}

/// Replaces `target` lines with `source` in an editor, keeping bookmarks on their text:
/// those inside the replaced lines go, those after them move by the change in line count.
fn merge_lines(ed: &mut EditorState, target: Range<usize>, source: Vec<String>) {
    let delta = source.len() as isize - target.len() as isize;
    ed.bookmarks = std::mem::take(&mut ed.bookmarks)
        .into_iter()
        .filter_map(|b| match b {
            b if b < target.start => Some(b),
            b if b >= target.end => Some((b as isize + delta) as usize),
            _ => None,
        })
        .collect();
    let at = target.start;
    ed.lines.splice(target, source);
    if ed.lines.is_empty() {
        ed.lines.push(String::new());
    }
    ed.selection = Selection::caret(TextPosition::new(at.min(ed.lines.len() - 1), 0));
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CompareAction {
    None,
    Close,
    /// Copy the current difference to the right (`true`) or left side.
    Merge(bool),
    /// The text options were changed in the window.
    Options(TextOptions),
}

fn read(path: &Path) -> Result<Vec<u8>, CompareError> {
    let err = |e: std::io::Error| CompareError::Read(path.to_path_buf(), e.to_string());
    let meta = std::fs::metadata(path).map_err(err)?;
    if meta.is_dir() {
        return Err(CompareError::Read(path.to_path_buf(), std::io::Error::from(std::io::ErrorKind::IsADirectory).to_string()));
    }
    if meta.len() > MAX_COMPARE_SIZE {
        return Err(CompareError::TooLarge(path.to_path_buf()));
    }
    std::fs::read(path).map_err(err)
}

impl CompareView {
    /// Compares two files. `open_lines` are the current lines of a side already open as text
    /// (unsaved edits included); other sides are read from disk. Binary data, files too large to
    /// open as text, or `binary` compare bytes. `open_eols` are the open sides' newline styles
    /// (their lines all end that way; `None`: unknown, such as mixed newlines), and `text` what
    /// a text comparison ignores.
    pub fn load(paths: [&Path; 2], binary: bool, open_lines: [Option<Vec<String>>; 2], open_eols: [Option<Eol>; 2], text: TextOptions, opts: &OpenOptions) -> Result<Self, CompareError> {
        let names = paths.map(crate::core::display_path);
        let mut info = [String::new(), String::new()];
        let [ol, or] = open_lines;
        let mut sources = [ol.filter(|_| !binary).map(Ok), or.filter(|_| !binary).map(Ok)];
        for (i, s) in sources.iter_mut().enumerate() {
            if s.is_none() {
                *s = Some(Err(read(paths[i])?));
            }
        }
        let [Some(l), Some(r)] = sources else { unreachable!() };
        let as_bytes = |s: &Result<Vec<String>, Vec<u8>>| match s {
            Err(b) => encoding::looks_binary(b) || b.len() as u64 > opts.max_text_size,
            Ok(_) => false,
        };
        let content = if binary || as_bytes(&l) || as_bytes(&r) {
            let mut bytes = [Vec::new(), Vec::new()];
            for (i, s) in [l, r].into_iter().enumerate() {
                bytes[i] = match s {
                    Err(b) => b,
                    Ok(_) => read(paths[i])?,
                };
            }
            let diff = compare::diff_bytes(&bytes[0], &bytes[1]);
            let mut targets: Vec<_> = diff.moved.iter().map(|m| m.target.clone()).collect();
            targets.sort_unstable_by_key(|r| r.start);
            let moved = [diff.moved.iter().map(|m| m.source.clone()).collect(), targets];
            info = [0, 1].map(|i| bytes[i].len().to_string());
            let marks = binary_marks(&bytes, &diff);
            Content::Binary { bytes, diff, moved, marks }
        } else {
            // Each side's lines and the terminator after each one.
            let decode = |s: Result<Vec<String>, Vec<u8>>, eol: Option<Eol>, info: &mut String| match s {
                Ok(lines) => {
                    let eols = eol.map(|eol| {
                        let mut eols = vec![eol; lines.len()];
                        if let Some(last) = eols.last_mut() {
                            *last = Eol::None;
                        }
                        eols
                    });
                    (lines, eols)
                }
                Err(b) => {
                    let d = encoding::detect_and_decode(&b, opts.default_encoding, opts.fallback_encoding, opts.enforce_default_encoding);
                    *info = text_info(d.encoding.name(), d.has_bom, encoding::detect_newlines(&d.text).dominant);
                    (EditorState::from_text(&d.text).lines, Some(compare::line_endings(&d.text)))
                }
            };
            let [(l, le), (r, re)] = [decode(l, open_eols[0], &mut info[0]), decode(r, open_eols[1], &mut info[1])];
            let lines = [l, r];
            let widths = [0, 1].map(|i| lines[i].iter().map(|l| l.chars().take(MAX_DRAWN_CHARS).map(char_visual_columns).sum::<usize>()).max().unwrap_or(0));
            let mut content = Content::Text { lines, eols: [le, re], options: text, diff: TextDiff::default(), widths, bounds: [Vec::new(), Vec::new()], inline: HashMap::new() };
            content.rediff();
            content
        };
        let resolved = paths.map(|p| p.canonicalize().unwrap_or_else(|_| p.to_path_buf()));
        let paths = paths.map(Path::to_path_buf);
        let mut view = Self { paths, names, binary, content, hunk: None, run: None, offset: Vec2::ZERO, scroll: 0.0, hscroll: 0.0, info, goto_row: None, drawn_w: 0.0, keys: true, rect: Rect::NOTHING, toolbar: Vec::new(), sources: [None, None], resolved, visible: 0.0, changed_at: None, seen: [None, None] };
        view.next();
        Ok(view)
    }

    /// What merging the current difference to the right (`to_right`) or left side does:
    /// the target side, its lines to replace (and their current text, to check the target
    /// has not changed since), and the source lines to put there.
    pub fn merge_plan(&self, to_right: bool) -> Option<MergePlan> {
        self.merge_plan_at(self.hunk?, to_right)
    }

    /// After comparing again, the difference that is still the same change as `old` (same
    /// lines on both sides), nearest to where it was; it becomes current. `None` when that
    /// change is gone.
    fn find_change(&mut self, old: &MergePlan, to_right: bool) -> Option<usize> {
        let Content::Text { lines, diff, bounds, .. } = &self.content else { return None };
        let (src, dst) = if to_right { (0, 1) } else { (1, 0) };
        // Compared in place: no copies of the differences' lines.
        let found = diff
            .hunks
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                let span = |p: usize| bounds[p][r.start]..bounds[p][r.end];
                lines[dst][span(dst)] == old.expected[..] && lines[src][span(src)] == old.source[..]
            })
            // Nearest to where it was, on either side: an edit moves line numbers only in the
            // file it was made in, so the other side's distance is exact for the same change.
            .min_by_key(|(_, r)| bounds[dst][r.start].abs_diff(old.target.start).min(bounds[src][r.start].abs_diff(old.source_range.start)))
            .map(|(h, _)| h);
        self.hunk = found;
        found
    }

    /// After comparing again: the outlined change (`outlined`, taken before) stays current
    /// if it is still there, found by its lines; otherwise only a difference in view becomes
    /// current. Never by index, which a change elsewhere shifts.
    fn retarget(&mut self, outlined: Option<MergePlan>) {
        if outlined.and_then(|old| self.find_change(&old, true)).is_none() {
            self.hunk = None;
            self.keep_hunk();
        }
    }

    fn merge_plan_at(&self, hunk: usize, to_right: bool) -> Option<MergePlan> {
        let Content::Text { lines, diff, bounds, .. } = &self.content else { return None };
        let range = diff.hunks.get(hunk)?;
        let (src, dst) = if to_right { (0, 1) } else { (1, 0) };
        let span = |p: usize| bounds[p][range.start]..bounds[p][range.end];
        let target = span(dst);
        let source_range = span(src);
        Some(MergePlan { side: dst, expected: lines[dst][target.clone()].to_vec(), target, source: lines[src][source_range.clone()].to_vec(), source_range })
    }

    /// Whether the main window should leave this frame's keys to the compare window: only
    /// when it is drawn inside the main window and the pointer is over it.
    pub fn takes_keys(&self, ctx: &egui::Context) -> bool {
        ctx.embed_viewports() && ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| self.rect.contains(p))
    }

    /// Keys the compare window handles itself (scrolling, stepping, merging, closing).
    pub fn is_own_key(key: egui::Key) -> bool {
        use egui::Key::*;
        matches!(key, ArrowUp | ArrowDown | ArrowLeft | ArrowRight | PageUp | PageDown | Home | End | F8 | Escape)
    }

    /// Index of the current text difference.
    #[cfg(test)]
    pub fn current(&self) -> Option<usize> {
        self.hunk
    }

    pub fn path(&self, side: usize) -> &Path {
        &self.paths[side]
    }

    /// Carries the position over from the comparison this one replaces (after a merge): the
    /// same difference index, now the one after the merged difference, and the same scroll.
    fn keep_place(&mut self, old: &CompareView) {
        if self.is_binary() {
            return;
        }
        self.goto_row = None;
        (self.scroll, self.hscroll, self.drawn_w, self.visible) = (old.scroll, old.hscroll, old.drawn_w, old.visible);
        self.retarget(old.merge_plan(true));
    }

    /// The text options in effect (`None` for a binary comparison).
    pub fn text_options(&self) -> Option<TextOptions> {
        match &self.content {
            Content::Text { options, .. } => Some(*options),
            Content::Binary { .. } => None,
        }
    }

    /// Compares again under new text options, keeping the place where possible.
    pub fn set_text_options(&mut self, opts: TextOptions) {
        let Content::Text { options, diff, .. } = &mut self.content else { return };
        if *options == opts {
            return;
        }
        *options = opts;
        let _ = diff;
        let outlined = self.merge_plan(true);
        self.content.rediff();
        self.retarget(outlined);
    }

    /// Replaces one side's text (from its tab, after an edit) and compares again, keeping the
    /// other side as loaded and the place in the window.
    fn replace_side(&mut self, side: usize, new_lines: Vec<String>, eol: Option<Eol>, info: String) {
        let Content::Text { lines, eols, widths, .. } = &mut self.content else { return };
        widths[side] = new_lines.iter().map(|l| l.chars().take(MAX_DRAWN_CHARS).map(char_visual_columns).sum::<usize>()).max().unwrap_or(0);
        eols[side] = eol.map(|eol| {
            let mut e = vec![eol; new_lines.len()];
            if let Some(last) = e.last_mut() {
                *last = Eol::None;
            }
            e
        });
        lines[side] = new_lines;
        self.info[side] = info;
        self.content.rediff();
    }

    /// After comparing again: keeps the current difference's index where it still exists;
    /// with none current, the first difference at or after the view becomes current, so
    /// Alt+arrows can merge a difference that just appeared.
    fn keep_hunk(&mut self) {
        let Content::Text { diff, .. } = &self.content else { return };
        let n = diff.hunks.len();
        self.hunk = match self.hunk {
            _ if n == 0 => None,
            Some(h) if h < n => Some(h),
            // Only a difference in view: a merge must never change one the user can't see.
            _ => {
                let i = diff.hunks.partition_point(|h| (h.end as f32) <= self.scroll);
                diff.hunks.get(i).filter(|h| (h.start as f32) < self.scroll + self.visible).map(|_| i)
            }
        };
    }

    /// Makes difference `h` current and brings it into view.
    fn show_hunk(&mut self, h: usize) {
        let Content::Text { diff, .. } = &self.content else { return };
        if let Some(r) = diff.hunks.get(h) {
            self.hunk = Some(h);
            let visible = r.start as f32 >= self.scroll && (r.start as f32) < self.scroll + self.visible;
            if !visible {
                self.goto_row = Some(r.start);
            }
        }
    }

    /// Sets what is shown under a side that came from an open tab.
    pub fn set_info(&mut self, side: usize, info: String) {
        self.info[side] = info;
    }

    pub fn is_binary(&self) -> bool {
        matches!(self.content, Content::Binary { .. })
    }

    pub fn is_identical(&self) -> bool {
        match &self.content {
            Content::Text { diff, .. } => diff.is_identical(),
            Content::Binary { diff, .. } => diff.is_identical(),
        }
    }

    /// Goes to the next difference (the first one when none is current).
    pub fn next(&mut self) -> bool {
        match &self.content {
            Content::Text { diff, .. } => {
                let i = self.hunk.map_or(0, |h| h + 1);
                let Some(h) = diff.hunks.get(i) else { return false };
                self.hunk = Some(i);
                self.goto_row = Some(h.start);
            }
            Content::Binary { bytes, diff, .. } => {
                let from = self.run.as_ref().map_or(0, |r| r.end);
                let Some(run) = diff.next_run(&bytes[0], &bytes[1], from) else { return false };
                self.goto_row = Some(run.start / BYTES_PER_ROW);
                self.run = Some(run);
            }
        }
        true
    }

    /// Goes to the previous difference (the last one when none is current).
    pub fn prev(&mut self) -> bool {
        match &self.content {
            Content::Text { diff, .. } => {
                let i = match self.hunk {
                    Some(0) => return false,
                    Some(h) => h - 1,
                    None => match diff.hunks.len().checked_sub(1) {
                        Some(i) => i,
                        None => return false,
                    },
                };
                self.hunk = Some(i);
                self.goto_row = Some(diff.hunks[i].start);
            }
            Content::Binary { bytes, diff, .. } => {
                let before = self.run.as_ref().map_or(usize::MAX, |r| r.start);
                let Some(run) = diff.prev_run(&bytes[0], &bytes[1], before) else { return false };
                self.goto_row = Some(run.start / BYTES_PER_ROW);
                self.run = Some(run);
            }
        }
        true
    }

    fn has_next(&self) -> bool {
        match &self.content {
            Content::Text { diff, .. } => self.hunk.map_or(0, |h| h + 1) < diff.hunks.len(),
            Content::Binary { bytes, diff, .. } => diff.next_run(&bytes[0], &bytes[1], self.run.as_ref().map_or(0, |r| r.end)).is_some(),
        }
    }

    fn has_prev(&self) -> bool {
        match &self.content {
            Content::Text { .. } => self.hunk.is_some_and(|h| h > 0),
            Content::Binary { bytes, diff, .. } => self.run.as_ref().is_some_and(|r| diff.prev_run(&bytes[0], &bytes[1], r.start).is_some()),
        }
    }

    fn summary(&self, lang: Lang) -> String {
        let t = |key: &str, zh: &'static str| i18n::translate(lang, key, zh);
        // What the comparison could not do, said even when nothing differs.
        let note = match &self.content {
            Content::Text { eols, options, .. } if !options.ignore_eol && eols.iter().any(Option::is_none) => Some(t("cmp.eol_unknown", "已開啟的檔案混用換行符號，未比較換行符號")),
            Content::Binary { diff, .. } if diff.alignment_skipped => Some(t("cmp.align_skipped", "差異範圍太大，未逐位元組對齊位移的資料")),
            _ => None,
        };
        let main = self.summary_counts(lang);
        match note {
            Some(n) => format!("{main} · {n}"),
            None => main,
        }
    }

    fn summary_counts(&self, lang: Lang) -> String {
        let t = |key: &str, zh: &'static str| i18n::translate(lang, key, zh);
        if self.is_identical() {
            return t("cmp.identical", "兩個檔案內容相同").to_string();
        }
        match &self.content {
            Content::Text { diff, .. } => {
                let s = diff.stats();
                let mut parts = vec![i18n::fill(t("cmp.n_blocks", "{n} 處差異"), Some(diff.hunks.len()), None)];
                for (key, zh, n) in [
                    ("cmp.n_changed", "修改 {n} 行", s.changed),
                    ("cmp.n_deleted", "刪除 {n} 行", s.deleted),
                    ("cmp.n_inserted", "新增 {n} 行", s.inserted),
                    ("cmp.n_moved", "移動 {n} 行", s.moved),
                ] {
                    if n > 0 {
                        parts.push(i18n::fill(t(key, zh), Some(n), None));
                    }
                }
                parts.join(" · ")
            }
            Content::Binary { bytes, diff, .. } => [
                i18n::fill(t("cmp.n_bytes", "{n} 個位元組不同"), Some(diff.differing), None),
                i18n::fill(t("cmp.n_moved_blocks", "{n} 個移動區塊"), Some(diff.moved.len()), None),
                format!("{} / {}", bytes[0].len(), bytes[1].len()),
            ]
            .join(" · "),
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, lang: Lang) -> CompareAction {
        let mut action = CompareAction::None;
        // In its own native window every key is ours. When the backend draws the window inside
        // the main one, the input is shared: take keys only while the pointer is over it, so
        // typing in the editor does not also scroll, step or merge here.
        self.rect = ui.max_rect();
        self.keys = !ui.ctx().embed_viewports() || ui.rect_contains_pointer(self.rect);
        let keys = self.keys;
        if ui.ctx().input(|i| i.viewport().close_requested() || (keys && i.key_pressed(egui::Key::Escape))) {
            return CompareAction::Close;
        }
        // Alt+Down / Alt+Up (or F8 / Shift+F8) step through the differences; Alt+Right /
        // Alt+Left copy the current one to the right / left side.
        let (next, prev, to_right, to_left) = ui.ctx().input(|i| {
            let (alt, f8) = (i.modifiers.alt, i.key_pressed(egui::Key::F8));
            let k = |k| alt && i.key_pressed(k);
            (k(egui::Key::ArrowDown) || (f8 && !i.modifiers.shift), k(egui::Key::ArrowUp) || (f8 && i.modifiers.shift), k(egui::Key::ArrowRight), k(egui::Key::ArrowLeft))
        });
        if keys && next {
            self.next();
        }
        if keys && prev {
            self.prev();
        }
        let can_merge = !self.is_binary() && self.hunk.is_some();
        if keys && can_merge && (to_right || to_left) {
            action = CompareAction::Merge(to_right);
        }
        let t = |key: &str, zh: &'static str| i18n::translate(lang, key, zh);
        let mut toolbar = Vec::new();
        egui::Panel::top("compare-toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                let (l_prev, l_next) = (t("cmp.prev", "上一個差異"), t("cmp.next", "下一個差異"));
                if tool(&mut toolbar, ui.add_enabled_ui(self.has_prev(), |ui| form::icon_button(ui, egui::Button::new("▲"), &format!("{l_prev} (Alt+↑)"), l_prev)).inner).clicked() {
                    self.prev();
                }
                if tool(&mut toolbar, ui.add_enabled_ui(self.has_next(), |ui| form::icon_button(ui, egui::Button::new("▼"), &format!("{l_next} (Alt+↓)"), l_next)).inner).clicked() {
                    self.next();
                }
                if !self.is_binary() {
                    ui.separator();
                    let (l_left, l_right) = (t("cmp.merge_left", "合併到左側"), t("cmp.merge_right", "合併到右側"));
                    if tool(&mut toolbar, ui.add_enabled_ui(can_merge, |ui| form::icon_button(ui, egui::Button::new("←"), &format!("{l_left} (Alt+←)"), l_left)).inner).clicked() {
                        action = CompareAction::Merge(false);
                    }
                    if tool(&mut toolbar, ui.add_enabled_ui(can_merge, |ui| form::icon_button(ui, egui::Button::new("→"), &format!("{l_right} (Alt+→)"), l_right)).inner).clicked() {
                        action = CompareAction::Merge(true);
                    }
                }
                ui.separator();
                ui.label(self.summary(lang));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if tool(&mut toolbar, ui.button(t("btn.close", "關閉"))).clicked() {
                        action = CompareAction::Close;
                    }
                    ui.separator();
                    let legend = |ui: &mut egui::Ui, color: Color32, text: &str| {
                        ui.label(text);
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                        ui.painter().rect_filled(r, 2.0, color);
                    };
                    legend(ui, MOVED, t("cmp.moved", "移動"));
                    legend(ui, INSERTED, t("cmp.inserted", "新增"));
                    legend(ui, DELETED, t("cmp.deleted", "刪除"));
                    legend(ui, CHANGED, t("cmp.changed", "修改"));
                });
            });
        });
        // The text options, under the toolbar: changing one compares again and is remembered.
        if let Some(mut o) = self.text_options() {
            let before = o;
            let [l_blank, l_case, l_ws, l_eol] = super::dialogs::compare_option_labels(lang);
            egui::Panel::top("compare-options").show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (label, value) in [(l_blank, &mut o.ignore_blank_lines), (l_case, &mut o.ignore_case), (l_ws, &mut o.ignore_whitespace), (l_eol, &mut o.ignore_eol)] {
                        let r = ui.checkbox(value, label);
                        toolbar.push(r.id);
                    }
                });
            });
            if o != before && action == CompareAction::None {
                action = CompareAction::Options(o);
            }
        }
        self.toolbar = toolbar;
        egui::CentralPanel::default().show(ui, |ui| {
            if self.is_binary() {
                self.hex_panes(ui, lang);
            } else {
                self.text_panes(ui, lang);
            }
        });
        action
    }

    /// The text panes: each side without filler lines, scrolled together through the aligned
    /// rows, with links between the matching differences and an overview at the right.
    fn text_panes(&mut self, ui: &mut egui::Ui, lang: Lang) {
        let Content::Text { lines, eols, options, diff, bounds, inline, widths } = &mut self.content else { return };
        let font = FontId::new(13.0, FontFamily::Name(EDITOR_FAMILY.into()));
        let cw = ui.ctx().fonts_mut(|f| f.glyph_width(&font, 'M')).max(1.0);
        let row_h = ui.ctx().fonts_mut(|f| f.row_height(&font)).ceil() + 2.0;
        let full = ui.available_rect_before_wrap();
        let bar_h = row_h + 8.0;
        let pane_w = ((full.width() - LINK_W - MAP_W) / 2.0).max(40.0);
        let xs = [full.left(), full.left() + pane_w + LINK_W];
        // A horizontal scroll bar runs along the bottom of the panes, above the footers.
        let hbar_h = 10.0;
        let body = Rect::from_min_max(pos2(full.left(), full.top() + bar_h), pos2(full.right(), full.bottom() - bar_h - hbar_h));
        let pane_rect = |p: usize| Rect::from_min_max(pos2(xs[p], body.top()), pos2(xs[p] + pane_w, body.bottom()));
        let gap = Rect::from_min_max(pos2(xs[0] + pane_w, body.top()), pos2(xs[1], body.bottom()));
        let map = Rect::from_min_max(pos2(xs[1] + pane_w, body.top()), pos2(full.right(), body.bottom()));
        let visible = (body.height() / row_h).max(1.0);
        self.visible = visible;
        let rows = diff.rows.len();
        let max_scroll = (rows as f32 - visible + 1.0).max(0.0);
        let gutters = [0, 1].map(|p| (digits(lines[p].len()) as f32 + 1.0) * cw);
        let text_w = pane_w - gutters[0].max(gutters[1]) - cw;
        let content_w = ((widths[0].max(widths[1]) as f32 + 2.0) * cw).max(self.drawn_w + 2.0 * cw);
        let max_h = (content_w - text_w).max(0.0);

        // Scrolling: the wheel over the panes, the keyboard, the overview, and jumps.
        let id = ui.scope_id().with("compare-text");
        let body_resp = ui.interact(body.intersect(Rect::everything_left_of(map.left())), id, Sense::click());
        if body_resp.hovered() {
            let d = ui.input(|i| i.smooth_scroll_delta());
            self.scroll -= d.y / row_h;
            self.hscroll -= d.x;
        }
        // Plain arrows scroll; Alt+arrows are taken by difference stepping and merging.
        let keys = ui.input(|i| {
            let k = |k| !i.modifiers.alt && i.key_pressed(k);
            let cmd = i.modifiers.command;
            [
                k(egui::Key::ArrowUp),
                k(egui::Key::ArrowDown),
                k(egui::Key::PageUp),
                k(egui::Key::PageDown),
                cmd && k(egui::Key::Home),
                cmd && k(egui::Key::End),
                k(egui::Key::ArrowLeft),
                k(egui::Key::ArrowRight),
                !cmd && k(egui::Key::Home),
                !cmd && k(egui::Key::End),
            ]
        });
        let free = ui.memory(|m| m.focused().is_none_or(|f| self.toolbar.contains(&f)));
        if self.keys && free {
            let [up, down, pg_up, pg_down, top, bottom, left, right, line_start, line_end] = keys;
            let n = |b: bool| f32::from(u8::from(b));
            self.scroll += n(down) - n(up) + (n(pg_down) - n(pg_up)) * (visible - 1.0).max(1.0);
            self.hscroll += (n(right) - n(left)) * 4.0 * cw;
            if top {
                self.scroll = 0.0;
            }
            if bottom {
                self.scroll = max_scroll;
            }
            if line_start {
                self.hscroll = 0.0;
            }
            if line_end {
                self.hscroll = max_h;
            }
        }
        // The horizontal bar: drag the thumb or click the track.
        let hbar = Rect::from_min_max(pos2(full.left(), body.bottom()), pos2(map.left(), body.bottom() + hbar_h));
        let hbar_resp = ui.interact(hbar, id.with("hbar"), Sense::click_and_drag());
        let track_w = hbar.width().max(1.0);
        let thumb_w = (track_w * (text_w / content_w.max(1.0))).clamp(24.0, track_w);
        if let Some(p) = hbar_resp.interact_pointer_pos().filter(|_| hbar_resp.is_pointer_button_down_on() || hbar_resp.clicked()) {
            let frac = (p.x - hbar.left() - thumb_w / 2.0) / (track_w - thumb_w).max(1.0);
            self.hscroll = frac * max_h;
        }
        let map_resp = ui.interact(map, id.with("map"), Sense::click_and_drag());
        if let Some(p) = map_resp.interact_pointer_pos().filter(|_| map_resp.is_pointer_button_down_on() || map_resp.clicked()) {
            self.scroll = (p.y - map.top()) / map.height().max(1.0) * rows as f32 - visible / 2.0;
        }
        if let Some(row) = self.goto_row.take() {
            self.scroll = row as f32 - visible / 3.0;
        }
        self.scroll = self.scroll.clamp(0.0, max_scroll);
        self.hscroll = self.hscroll.clamp(0.0, max_h);
        let (scroll, hscroll) = (self.scroll, self.hscroll);

        // Where each pane is: its top line (fractional), from the shared row position.
        let position = |p: usize| {
            let b = &bounds[p];
            let i = (scroll.floor() as usize).min(rows);
            let (s, e) = (b[i], b[(i + 1).min(rows)]);
            s as f32 + scroll.fract() * (e - s) as f32
        };
        let pos = [position(0), position(1)];
        let line_y = |p: usize, line: usize| body.top() + (line as f32 - pos[p]) * row_h;
        let shown = |p: usize| {
            let first = pos[p].floor() as usize;
            first..(first + visible.ceil() as usize + 1).min(lines[p].len())
        };

        // Word highlights of the changed rows in view.
        if inline.len() > INLINE_CACHE_ROWS {
            inline.clear();
        }
        for p in 0..2 {
            for k in shown(p) {
                let Some(ri) = diff.row_of(p == 1, k) else { continue };
                let row = diff.rows[ri];
                if let (RowKind::Changed, Some(l), Some(r)) = (row.kind, row.left, row.right) {
                    inline.entry(ri).or_insert_with(|| {
                        let (a, b) = compare::inline_diff(&lines[0][l], &lines[1][r], *options);
                        [a, b]
                    });
                }
            }
        }

        let painter = ui.painter_at(full);
        let visuals = ui.visuals().clone();
        let current = self.hunk.map(|h| diff.hunks[h].clone());
        let pointer = ui.input(|i| i.pointer.hover_pos()).filter(|_| body_resp.hovered());
        let mut tip = None;
        let mut jump = None;
        for p in 0..2 {
            let rect = pane_rect(p);
            let text_rect = Rect::from_min_max(pos2(rect.left() + gutters[p], rect.top()), rect.max);
            let text_painter = painter.with_clip_rect(text_rect);
            let pane_painter = painter.with_clip_rect(rect);
            pane_painter.rect_filled(rect, 0.0, visuals.extreme_bg_color);
            for k in shown(p) {
                let y = line_y(p, k);
                let row_rect = Rect::from_min_size(pos2(rect.left(), y), vec2(pane_w, row_h));
                let Some(ri) = diff.row_of(p == 1, k) else { continue };
                let row = diff.rows[ri];
                let moved = if p == 0 { row.left_moved } else { row.right_moved };
                let bg = match row.kind {
                    _ if row.ignored => Color32::TRANSPARENT,
                    _ if moved.is_some() => MOVED,
                    RowKind::Changed => CHANGED,
                    RowKind::Deleted => DELETED,
                    RowKind::Inserted => INSERTED,
                    RowKind::Same => Color32::TRANSPARENT,
                };
                if bg != Color32::TRANSPARENT {
                    pane_painter.rect_filled(row_rect, 0.0, bg);
                }
                pane_painter.text(pos2(rect.left() + gutters[p] - cw * 0.5, row_rect.center().y), egui::Align2::RIGHT_CENTER, (k + 1).to_string(), font.clone(), visuals.weak_text_color());
                let words = inline.get(&ri).map_or(&[][..], |w| &w[p][..]);
                let mut job = line_job(&lines[p][k], words, if moved.is_some() { MOVED_WORDS } else { CHANGED_WORDS }, &font, visuals.text_color(), visuals.strong_text_color());
                // When newline styles count, a changed line whose terminator differs shows it.
                if !options.ignore_eol
                    && row.kind == RowKind::Changed
                    && let (Some(l), Some(r)) = (row.left, row.right)
                    && let [Some(le), Some(re)] = &*eols
                    && le.get(l) != re.get(r)
                    && let Some(mark) = eol_mark([le, re][p].get(k).copied().unwrap_or(Eol::None))
                {
                    let format = egui::TextFormat { background: CHANGED_WORDS, color: visuals.strong_text_color(), ..egui::TextFormat::simple(font.clone(), visuals.text_color()) };
                    job.append(mark, cw * 0.5, format);
                }
                let galley = ui.ctx().fonts_mut(|f| f.layout_job(job));
                self.drawn_w = self.drawn_w.max(galley.size().x);
                let x = text_rect.left() + cw * 0.5 - hscroll;
                text_painter.galley(pos2(x, row_rect.center().y - galley.size().y / 2.0), galley, visuals.text_color());
                if let (Some(to), Some(at)) = (moved, pointer)
                    && row_rect.contains(at)
                {
                    let other = if p == 0 { i18n::translate(lang, "cmp.moved_to", "已移到右側第 {n} 行") } else { i18n::translate(lang, "cmp.moved_from", "由左側第 {n} 行移來") };
                    tip = Some(format!("{}\n{}", i18n::fill(other, Some(to + 1), None), i18n::translate(lang, "cmp.click_jump", "按一下跳到該行")));
                    if body_resp.clicked() {
                        jump = diff.row_of(p == 0, to);
                    }
                }
            }
            if lines[p].is_empty() {
                continue;
            }
            // The current difference is outlined (a line where the other side has nothing).
            if let Some(c) = &current {
                let (top, bottom) = (line_y(p, bounds[p][c.start]), line_y(p, bounds[p][c.end]));
                let stroke = Stroke::new(1.5, form::ACCENT);
                if bottom - top < 1.0 {
                    pane_painter.hline(rect.x_range(), top, Stroke::new(2.0, form::ACCENT));
                } else {
                    pane_painter.rect_stroke(Rect::from_min_max(pos2(rect.left() + 1.0, top + 0.5), pos2(rect.right() - 1.0, bottom - 0.5)), 0.0, stroke, egui::StrokeKind::Inside);
                }
            }
        }

        // Links between the panes for the differences in view.
        let gap_painter = painter.with_clip_rect(gap);
        gap_painter.rect_filled(gap, 0.0, visuals.faint_bg_color);
        let row_at = |p: usize, line: usize| diff.row_of(p == 1, line.min(lines[p].len().saturating_sub(1)));
        let lo = (0..2).filter_map(|p| row_at(p, shown(p).start)).min().unwrap_or(0);
        let hi = (0..2).filter_map(|p| row_at(p, shown(p).end)).max().unwrap_or(rows);
        let first = diff.hunks.partition_point(|h| h.end <= lo);
        for (h, range) in diff.hunks.iter().enumerate().skip(first).take_while(|(_, r)| r.start <= hi + 1) {
            let span = |p: usize| (line_y(p, bounds[p][range.start]), line_y(p, bounds[p][range.end]));
            let color = hunk_color(diff, range);
            let selected = self.hunk == Some(h);
            let fill = if selected { color } else { color.gamma_multiply(0.6) };
            let edge = if selected { form::ACCENT } else { solid(color).gamma_multiply(0.5) };
            paint_link(&gap_painter, gap.left(), gap.right(), span(0), span(1), fill, edge);
        }

        // The overview of every difference, and the part in view.
        let map_painter = painter.with_clip_rect(map);
        map_painter.rect_filled(map, 0.0, visuals.faint_bg_color);
        let scale = map.height() / rows.max(1) as f32;
        // Marks are skipped when they would land on one already drawn in their column, so a
        // file with very many differences stays cheap without losing either side's marks.
        let mut last_y = [f32::NEG_INFINITY; 2];
        for range in &diff.hunks {
            let y = map.top() + range.start as f32 * scale;
            let h = (range.len() as f32 * scale).max(2.0);
            let color = solid(hunk_color(diff, range));
            let half = map.width() / 2.0 - 2.0;
            for (p, b) in bounds.iter().enumerate() {
                if b[range.start] < b[range.end] && y + h > last_y[p] + 1.0 {
                    last_y[p] = y + h;
                    let x = map.left() + 1.5 + p as f32 * (half + 1.0);
                    map_painter.rect_filled(Rect::from_min_size(pos2(x, y), vec2(half, h)), 1.0, color);
                }
            }
        }
        let view = Rect::from_min_max(pos2(map.left(), map.top() + scroll * scale), pos2(map.right(), map.top() + ((scroll + visible) * scale).min(map.height())));
        map_painter.rect_stroke(view.shrink(0.5), 2.0, Stroke::new(1.0, visuals.weak_text_color()), egui::StrokeKind::Inside);

        // The horizontal bar's thumb, shown only when a line is wider than the panes.
        if max_h > 0.0 {
            let x = hbar.left() + hscroll / max_h * (track_w - thumb_w);
            let thumb = Rect::from_min_size(pos2(x, hbar.top() + 2.0), vec2(thumb_w, hbar_h - 4.0));
            let style = if hbar_resp.hovered() || hbar_resp.dragged() { &visuals.widgets.hovered } else { &visuals.widgets.inactive };
            painter.rect_filled(thumb, 3.0, style.bg_fill);
        }

        // File names above and file details below each pane.
        let t = |key: &str, zh: &'static str| i18n::translate(lang, key, zh);
        for p in 0..2 {
            let header = Rect::from_min_size(pos2(xs[p], full.top()), vec2(pane_w, bar_h));
            let mut hui = ui.new_child(egui::UiBuilder::new().id_salt(("compare-header", p)).max_rect(header.shrink2(vec2(4.0, 2.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
            let badge = if p == 0 { t("cmp.left_side", "左側") } else { t("cmp.right_side", "右側") };
            hui.label(egui::RichText::new(badge).small().color(form::ACCENT).background_color(form::ACCENT.gamma_multiply(0.15)));
            path_label(&mut hui, &self.names[p]);
            let footer = Rect::from_min_size(pos2(xs[p], body.bottom()), vec2(pane_w, bar_h));
            let mut fui = ui.new_child(egui::UiBuilder::new().id_salt(("compare-footer", p)).max_rect(footer.shrink2(vec2(6.0, 2.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
            fui.label(egui::RichText::new(&self.info[p]).small().weak());
            fui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let top = (pos[p].floor() as usize + 1).min(lines[p].len());
                let text = i18n::translate(lang, "cmp.position", "第 {n} 行，共 {q} 行");
                ui.label(egui::RichText::new(i18n::fill(text, Some(top), Some(&lines[p].len().to_string()))).small().weak());
            });
        }
        painter.hline(full.x_range(), body.top(), Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color));
        painter.hline(full.x_range(), body.bottom(), Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color));
        ui.advance_cursor_after_rect(full);

        if let Some(tip) = tip {
            body_resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip);
        }
        if let Some(row) = jump {
            self.goto_row = Some(row);
            ui.ctx().request_repaint();
        }
    }

    /// The two hex panes under their file names, scrolled together.
    fn hex_panes(&mut self, ui: &mut egui::Ui, lang: Lang) {
        let Content::Binary { bytes, moved, diff, marks } = &self.content else { return };
        let font = FontId::new(13.0, FontFamily::Name(EDITOR_FAMILY.into()));
        let cw = ui.ctx().fonts_mut(|f| f.glyph_width(&font, 'M')).max(1.0);
        let row_h = ui.ctx().fonts_mut(|f| f.row_height(&font)).ceil() + 2.0;
        let rows = diff.cells().div_ceil(BYTES_PER_ROW);
        // Header and footer heights plus a little margin.
        let body_h = (ui.available_height() - 48.0).max(row_h);
        // The overview at the right: every difference, the part in view, click or drag to scroll.
        let full = ui.available_rect_before_wrap();
        let map = Rect::from_min_max(pos2(full.right() - MAP_W, full.top() + 24.0), pos2(full.right(), full.top() + 24.0 + body_h));
        let map_resp = ui.interact(map, ui.scope_id().with("hex-map"), Sense::click_and_drag());
        let cells = diff.cells().max(1) as f32;
        if let Some(p) = map_resp.interact_pointer_pos().filter(|_| map_resp.is_pointer_button_down_on() || map_resp.clicked()) {
            let row = (p.y - map.top()) / map.height().max(1.0) * rows as f32 - body_h / row_h / 2.0;
            self.offset.y = (row * row_h).clamp(0.0, (rows as f32 * row_h - body_h).max(0.0));
        }
        {
            let painter = ui.painter_at(map);
            painter.rect_filled(map, 0.0, ui.visuals().faint_bg_color);
            let mut last = f32::NEG_INFINITY;
            for (range, color) in marks {
                let y = map.top() + range.start as f32 / cells * map.height();
                let h = (range.len() as f32 / cells * map.height()).max(2.0);
                if y + h > last + 1.0 {
                    last = y + h;
                    painter.rect_filled(Rect::from_min_size(pos2(map.left() + 2.0, y), vec2(map.width() - 4.0, h)), 1.0, solid(*color));
                }
            }
            let total = rows as f32 * row_h;
            if total > 0.0 {
                let view = Rect::from_min_max(pos2(map.left(), map.top() + self.offset.y / total * map.height()), pos2(map.right(), map.top() + ((self.offset.y + body_h) / total).min(1.0) * map.height()));
                painter.rect_stroke(view.shrink(0.5), 2.0, Stroke::new(1.0, ui.visuals().weak_text_color()), egui::StrokeKind::Inside);
            }
        }
        let mut ui = ui.new_child(egui::UiBuilder::new().id_salt("hex-columns").max_rect(Rect::from_min_max(full.min, pos2(full.right() - MAP_W - 4.0, full.bottom()))));
        let ui = &mut ui;
        if let Some(row) = self.goto_row.take() {
            self.offset.y = (row as f32 * row_h - body_h / 3.0).clamp(0.0, (rows as f32 * row_h - body_h).max(0.0));
        }
        let mut new_offset = self.offset;
        ui.columns(2, |cols| {
            for (side, ui) in cols.iter_mut().enumerate() {
                ui.horizontal(|ui| path_label(ui, &self.names[side]));
                let area = egui::ScrollArea::both().id_salt(("compare-pane", side)).auto_shrink([false, false]).max_height(body_h).scroll_offset(self.offset);
                let out = area.show_rows(ui, row_h, rows, |ui, range| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let width = (hex_columns() as f32 * cw).max(ui.available_width());
                    for ri in range {
                        hex_row(ui, &font, cw, row_h, width, side, bytes, diff, &moved[side], ri, self.run.as_ref());
                    }
                });
                if out.state.clamped_offset() != self.offset {
                    new_offset = out.state.clamped_offset();
                }
                let size = self.info[side].parse().ok();
                ui.label(egui::RichText::new(i18n::fill(i18n::translate(lang, "cmp.size", "{n} 位元組"), size, None)).small().weak());
            }
        });
        self.offset = new_offset;
    }
}

/// The file name first, so a long folder is what gets cut off; the full path as a tooltip.
fn path_label(ui: &mut egui::Ui, path: &str) {
    let (name, folder) = name_and_folder(path);
    ui.label(egui::RichText::new(name).strong()).on_hover_text(path);
    ui.add(egui::Label::new(egui::RichText::new(folder).weak()).truncate()).on_hover_text(path);
}

/// "UTF-8 · LF" for the footer of a text side.
pub(super) fn text_info(encoding: &str, bom: bool, newline: Option<NewLineFormat>) -> String {
    let mut s = encoding.to_string();
    if bom {
        s.push_str(" BOM");
    }
    if let Some(n) = newline {
        s.push_str(match n {
            NewLineFormat::Dos => " · CRLF",
            NewLineFormat::Unix => " · LF",
            NewLineFormat::Mac => " · CR",
        });
    }
    s
}

/// The fill for a difference: moved when all its lines are, else by which sides have lines.
fn hunk_color(diff: &TextDiff, range: &Range<usize>) -> Color32 {
    let rows = &diff.rows[range.clone()];
    if rows.iter().all(|r| (r.left.is_none() || r.left_moved.is_some()) && (r.right.is_none() || r.right_moved.is_some())) {
        return MOVED;
    }
    match (rows.iter().any(|r| r.left.is_some()), rows.iter().any(|r| r.right.is_some())) {
        (true, false) => DELETED,
        (false, true) => INSERTED,
        _ => CHANGED,
    }
}

/// An opaque version of a translucent fill, for marks and edges.
fn solid(c: Color32) -> Color32 {
    let a = f32::from(c.a().max(1)) / 255.0;
    let up = |v: u8| ((f32::from(v) / a).min(255.0)) as u8;
    Color32::from_rgb(up(c.r()), up(c.g()), up(c.b()))
}

/// How a line terminator is shown where newline styles are compared.
fn eol_mark(eol: Eol) -> Option<&'static str> {
    match eol {
        Eol::None => None,
        Eol::Lf => Some("LF"),
        Eol::CrLf => Some("CRLF"),
        Eol::Cr => Some("CR"),
    }
}

/// A line's text with its changed words highlighted; tabs drawn as spaces.
fn line_job(line: &str, words: &[Range<usize>], highlight: Color32, font: &FontId, color: Color32, strong: Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let plain = egui::TextFormat::simple(font.clone(), color);
    let marked = egui::TextFormat { background: highlight, color: strong, ..plain.clone() };
    let mut wi = 0;
    let mut run = String::new();
    let mut run_marked = false;
    for (i, c) in line.char_indices().take(MAX_DRAWN_CHARS) {
        while words.get(wi).is_some_and(|w| w.end <= i) {
            wi += 1;
        }
        let m = words.get(wi).is_some_and(|w| w.contains(&i));
        if m != run_marked && !run.is_empty() {
            job.append(&std::mem::take(&mut run), 0.0, if run_marked { marked.clone() } else { plain.clone() });
        }
        run_marked = m;
        run.extend(expand_tab(c));
    }
    if !run.is_empty() {
        job.append(&run, 0.0, if run_marked { marked } else { plain });
    }
    job
}

/// A band joining a difference's lines on the left (`left` top and bottom) to the right ones,
/// eased so a side with no lines narrows to a point. Sampled, after OxideMerge.
fn paint_link(painter: &egui::Painter, x0: f32, x1: f32, left: (f32, f32), right: (f32, f32), fill: Color32, edge: Color32) {
    const STEPS: usize = 16;
    let mut mesh = egui::Mesh::default();
    let (mut upper, mut lower) = (Vec::with_capacity(STEPS + 1), Vec::with_capacity(STEPS + 1));
    for i in 0..=STEPS {
        let t = i as f32 / STEPS as f32;
        let ease = t * t * (3.0 - 2.0 * t);
        let x = egui::lerp(x0..=x1, t);
        let top = pos2(x, egui::lerp(left.0..=right.0, ease));
        let bottom = pos2(x, egui::lerp(left.1..=right.1, ease));
        upper.push(top);
        lower.push(bottom);
        mesh.colored_vertex(top, fill);
        mesh.colored_vertex(bottom, fill);
        if i > 0 {
            let k = (i * 2) as u32;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k, k - 1, k + 1);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
    painter.add(egui::Shape::line(upper, Stroke::new(1.0, edge)));
    painter.add(egui::Shape::line(lower, Stroke::new(1.0, edge)));
}

/// Notes a toolbar button's id (see [`CompareView::toolbar`]) and passes it on.
fn tool(ids: &mut Vec<egui::Id>, r: egui::Response) -> egui::Response {
    ids.push(r.id);
    r
}

fn digits(n: usize) -> usize {
    n.max(1).ilog10() as usize + 1
}

/// Columns of a hex row: offset, 16 bytes, gap, 16 characters.
fn hex_columns() -> usize {
    9 + BYTES_PER_ROW * 3 + 1 + BYTES_PER_ROW
}

/// Tabs are drawn as spaces up to the configured tab width.
fn expand_tab(c: char) -> std::iter::RepeatN<char> {
    let n = if c == '\t' { char_visual_columns(c) } else { 1 };
    std::iter::repeat_n(if c == '\t' { ' ' } else { c }, n)
}

/// Draws one side of a 16-cell row of the aligned hex layout: the offset of its first byte,
/// the bytes (blank cells where only the other side has bytes) and their characters.
#[allow(clippy::too_many_arguments)]
fn hex_row(ui: &mut egui::Ui, font: &FontId, cw: f32, row_h: f32, width: f32, side: usize, bytes: &[Vec<u8>; 2], diff: &BinaryDiff, moved: &[Range<usize>], ri: usize, current: Option<&Range<usize>>) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, row_h), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    let start = ri * BYTES_PER_ROW;
    let y = rect.center().y;
    let text_color = ui.visuals().text_color();
    let weak = ui.visuals().weak_text_color();
    if current.is_some_and(|c| c.start < start + BYTES_PER_ROW && start < c.end) {
        painter.rect_filled(Rect::from_min_size(rect.min, egui::vec2(3.0, row_h)), 0.0, form::ACCENT);
    }
    let cells: Vec<[Option<usize>; 2]> = (start..(start + BYTES_PER_ROW).min(diff.cells())).map(|d| diff.cell(d)).collect();
    if let Some(off) = cells.iter().find_map(|c| c[side]) {
        painter.text(egui::pos2(rect.min.x + cw * 0.5, y), egui::Align2::LEFT_CENTER, format!("{off:08X}"), font.clone(), weak);
    }
    let hex_x = rect.min.x + 9.0 * cw + cw * 0.5;
    let chr_x = hex_x + (BYTES_PER_ROW * 3 + 1) as f32 * cw;
    for (k, cell) in cells.iter().enumerate() {
        let d = start + k;
        let hx = hex_x + (k * 3) as f32 * cw;
        let cx = chr_x + k as f32 * cw;
        let shifted = diff.stretch_at(d).is_some_and(|s| s.matched && s.start[0] != s.start[1]);
        let bg = match (cell[side], cell[1 - side]) {
            // Shifted bytes lined up with their counterpart: only a light tint, as they read
            // as unchanged next to it; relocated blocks elsewhere are fully marked.
            (Some(_), Some(_)) if shifted => SHIFTED,
            (Some(off), _) if moved_at(moved, off) => MOVED,
            (Some(_), None) => if side == 0 { DELETED } else { INSERTED },
            (None, Some(_)) => FILLER,
            (Some(x), Some(o)) if bytes[side][x] != bytes[1 - side][o] => CHANGED,
            _ => Color32::TRANSPARENT,
        };
        if bg != Color32::TRANSPARENT {
            painter.rect_filled(Rect::from_min_size(egui::pos2(hx - cw * 0.2, rect.min.y), egui::vec2(cw * 2.4, row_h)), 0.0, bg);
            painter.rect_filled(Rect::from_min_size(egui::pos2(cx, rect.min.y), egui::vec2(cw, row_h)), 0.0, bg);
        }
        let Some(off) = cell[side] else { continue };
        let b = bytes[side][off];
        painter.text(egui::pos2(hx, y), egui::Align2::LEFT_CENTER, format!("{b:02X}"), font.clone(), text_color);
        let c = if b.is_ascii_graphic() || b == b' ' { b as char } else { '.' };
        painter.text(egui::pos2(cx, y), egui::Align2::LEFT_CENTER, c.to_string(), font.clone(), if c == '.' && b != b'.' { weak } else { text_color });
    }
}

/// The overview marks of a binary comparison: up to 8192 runs of differing cells, coloured
/// as deleted or inserted when only one side has bytes there, else as changed.
fn binary_marks(bytes: &[Vec<u8>; 2], diff: &BinaryDiff) -> Vec<(Range<usize>, Color32)> {
    let mut out = Vec::new();
    let mut from = 0;
    while out.len() < 8192 {
        let Some(run) = diff.next_run(&bytes[0], &bytes[1], from) else { break };
        from = run.end;
        let sides = run.clone().take(64).fold([false; 2], |acc, d| {
            let c = diff.cell(d);
            [acc[0] || c[0].is_some(), acc[1] || c[1].is_some()]
        });
        let color = match sides {
            [true, false] => DELETED,
            [false, true] => INSERTED,
            _ => CHANGED,
        };
        out.push((run, color));
    }
    out
}

/// Whether byte `off` is in one of the sorted, non-overlapping moved ranges.
fn moved_at(moved: &[Range<usize>], off: usize) -> bool {
    let i = moved.partition_point(|r| r.end <= off);
    moved.get(i).is_some_and(|r| r.contains(&off))
}

impl App {
    /// Starts a comparison from the Compare dialog. Returns false (with a message) when a file
    /// could not be read.
    pub(super) fn start_compare(&mut self, left: &str, right: &str, binary: bool) -> bool {
        let paths = [PathBuf::from(left.trim()), PathBuf::from(right.trim())];
        match self.load_compare(&paths, binary) {
            Ok(view) => {
                if view.is_identical() {
                    self.set_status(self.t("cmp.identical", "兩個檔案內容相同"));
                }
                self.compare = Some(Box::new(view));
                true
            }
            Err(e) => {
                self.compare_error(e);
                false
            }
        }
    }

    fn compare_error(&mut self, e: CompareError) {
        let title = self.t("cmp.fail", "無法比較檔案").to_string();
        let text = match e {
            CompareError::Read(p, err) => format!("{}\n{err}", crate::core::display_path(&p)),
            CompareError::TooLarge(p) => format!("{}\n{}", crate::core::display_path(&p), self.t("cmp.too_large", "檔案太大（上限 256 MiB）")),
        };
        self.message(&title, text);
    }

    /// Compares two files, taking a side's lines (and encoding details) from its tab when it is
    /// open as text.
    fn load_compare(&self, paths: &[PathBuf; 2], binary: bool) -> Result<CompareView, CompareError> {
        let open = [0, 1].map(|i| self.text_tab(&paths[i]).map(|t| &self.tabs[t].doc));
        let open_lines = open.map(|d| d.map(|d| d.editor.lines.clone()));
        // A tab with mixed newlines does not keep each line's terminator: not compared.
        let open_eols = open.map(|d| {
            d.filter(|d| !d.mixed_newlines).map(|d| match d.newline {
                NewLineFormat::Dos => Eol::CrLf,
                NewLineFormat::Unix => Eol::Lf,
                NewLineFormat::Mac => Eol::Cr,
            })
        });
        let mut view = CompareView::load([&paths[0], &paths[1]], binary, open_lines, open_eols, self.compare_text_options(), &self.open_options())?;
        // A side taken from a tab is matched by that tab's own path from now on: it may be
        // spelled differently from the canonical path (Save As keeps the dialog's spelling).
        for (side, doc) in open.iter().enumerate() {
            if let Some(p) = doc.and_then(|d| d.path.clone()) {
                view.resolved[side] = p;
            }
        }
        view.sources = self.compare_sources(&view);
        view.seen = view.sources;
        if !view.is_binary() {
            for (side, doc) in open.into_iter().enumerate() {
                if let Some(d) = doc {
                    view.set_info(side, text_info(d.encoding.name(), d.has_bom, Some(d.newline)));
                }
            }
        }
        Ok(view)
    }

    /// The tab showing a compared side as text, matched by path without asking the file
    /// system (cheap enough for every frame).
    fn compare_tab(&self, view: &CompareView, side: usize) -> Option<usize> {
        let (given, resolved) = (&view.paths[side], &view.resolved[side]);
        self.tabs.iter().position(|t| t.doc.mode == EditMode::Text && !t.doc.hex_only && t.doc.path.as_deref().is_some_and(|p| p == resolved || p == given))
    }

    /// The tab of a compared side, for merging: matched like the window matches it, else by
    /// `same_target` (a path spelled differently), which then becomes the side's path, so the
    /// window refreshes from that tab too.
    fn side_tab(&mut self, side: usize) -> Option<usize> {
        let view = self.compare.as_ref()?;
        if let Some(t) = self.compare_tab(view, side) {
            return Some(t);
        }
        let t = self.text_tab(&view.paths[side])?;
        let p = self.tabs[t].doc.path.clone()?;
        self.compare.as_mut()?.resolved[side] = p;
        Some(t)
    }

    /// The document version of each side's text tab (`None` when it is not open as text).
    fn compare_sources(&self, view: &CompareView) -> [Option<u64>; 2] {
        [0, 1].map(|i| self.compare_tab(view, i).map(|t| self.tabs[t].doc.version))
    }

    /// What text comparisons ignore, from the settings.
    pub(super) fn compare_text_options(&self) -> TextOptions {
        let s = &self.settings;
        TextOptions { ignore_blank_lines: s.compare_ignore_blank_lines, ignore_case: s.compare_ignore_case, ignore_whitespace: s.compare_ignore_whitespace, ignore_eol: s.compare_ignore_eol }
    }

    /// Stores new text compare options in the settings (saved right away, like View toggles)
    /// and compares the open window again under them.
    pub(super) fn set_compare_text_options(&mut self, o: TextOptions) {
        let s = &mut self.settings;
        (s.compare_ignore_blank_lines, s.compare_ignore_case, s.compare_ignore_whitespace, s.compare_ignore_eol) = (o.ignore_blank_lines, o.ignore_case, o.ignore_whitespace, o.ignore_eol);
        self.save_settings();
        if let Some(view) = self.compare.as_mut() {
            view.set_text_options(o);
        }
    }

    /// The tab showing `path` as text, if any.
    fn text_tab(&self, path: &Path) -> Option<usize> {
        self.tabs.iter().position(|t| t.doc.mode == EditMode::Text && !t.doc.hex_only && t.doc.path.as_deref().is_some_and(|p| same_target(p, path)))
    }

    /// Copies the compare window's current difference to the right (`to_right`) or left file.
    /// The change goes through the target's tab (opened first if needed) as one undoable
    /// edit, so it is saved, or undone, like any other edit.
    pub(super) fn merge_compare(&mut self, to_right: bool) {
        // Edits not yet compared (within the refresh delay) are compared now, so a merge right
        // after typing works instead of reporting the file as changed. The merge goes ahead
        // only if the change the user was looking at is still there (found by its lines, not
        // its index, which an edit above it shifts); otherwise the refreshed view is shown
        // with that change gone, and nothing is merged on this key press.
        if self.compare.as_ref().is_some_and(|v| !v.is_binary() && self.compare_sources(v) != v.sources) {
            let before = self.compare.as_ref().and_then(|v| v.merge_plan(to_right));
            self.refresh_compare();
            let Some(view) = self.compare.as_mut() else { return };
            let kept = before.as_ref().and_then(|old| view.find_change(old, to_right));
            if let Some(h) = kept {
                view.show_hunk(h);
            } else {
                // As after any refresh: a difference in view becomes current, for the next press.
                view.retarget(None);
                let msg = if before.is_some() {
                    self.t("cmp.merge_changed", "差異已變更，請確認後再合併")
                } else {
                    self.t("cmp.merge_selected", "已選取畫面中的差異，再按一次即可合併")
                };
                let msg = msg.to_string();
                self.set_status(msg);
                return;
            }
        }
        let Some(view) = self.compare.as_ref() else { return };
        let Some(MergePlan { side, target, expected, source_range, source }) = view.merge_plan(to_right) else { return };
        let path = view.path(side).to_path_buf();
        let title = self.t("cmp.merge_fail", "無法合併").to_string();
        let fail = |app: &mut Self, key: &str, zh: &'static str| {
            let text = format!("{}\n{}", crate::core::display_path(&path), app.t(key, zh));
            app.message(&title, text);
        };
        // Both sides must still be what the window shows; otherwise compare again first.
        let stale = |app: &mut Self, side: usize, range: &Range<usize>, lines: &[String]| {
            app.side_tab(side).is_some_and(|t| app.tabs[t].doc.editor.lines.get(range.clone()) != Some(lines))
        };
        if stale(self, 1 - side, &source_range, &source) || stale(self, side, &target, &expected) {
            self.refresh_compare();
            let text = self.t("cmp.merge_stale", "檔案內容已變更，已重新比較，請再試一次").to_string();
            self.message(&title, text);
            return;
        }
        if self.side_tab(side).is_none() && !self.open_path(&path) {
            return;
        }
        let Some(ti) = self.side_tab(side) else {
            fail(self, "cmp.merge_not_text", "目標檔案未以文字模式開啟");
            return;
        };
        if self.tabs[ti].doc.read_only {
            fail(self, "cmp.merge_read_only", "目標檔案為唯讀");
            return;
        }
        // A file just opened for the merge must still hold the compared lines too.
        if stale(self, side, &target, &expected) {
            self.refresh_compare();
            let text = self.t("cmp.merge_stale", "檔案內容已變更，已重新比較，請再試一次").to_string();
            self.message(&title, text);
            return;
        }
        let doc = &mut self.tabs[ti].doc;
        doc.edit(EditKind::Other, target.clone(), |ed| merge_lines(ed, target, source));
        self.refresh_compare();
        let msg = if to_right { self.t("cmp.merged_right", "已合併到右側（尚未儲存）") } else { self.t("cmp.merged_left", "已合併到左側（尚未儲存）") };
        self.set_status(msg);
    }

    /// Compares again after a compared tab changed, keeping the place in the window. Sides
    /// open in tabs are taken from them; a side not open keeps the text loaded before, so an
    /// edit does not read the other file again. Only a side whose tab was closed is read from
    /// disk again (a full reload).
    fn refresh_compare(&mut self) {
        let Some(mut view) = self.compare.take() else { return };
        let now = self.compare_sources(&view);
        let closed = (0..2).any(|i| view.sources[i].is_some() && now[i].is_none());
        if !view.is_binary() && !closed {
            // The outlined change, to keep it current by its lines rather than its index.
            let outlined = view.merge_plan(true);
            for side in 0..2 {
                if let Some(t) = self.compare_tab(&view, side).filter(|_| now[side] != view.sources[side]) {
                    let d = &self.tabs[t].doc;
                    let eol = (!d.mixed_newlines).then_some(match d.newline {
                        NewLineFormat::Dos => Eol::CrLf,
                        NewLineFormat::Unix => Eol::Lf,
                        NewLineFormat::Mac => Eol::Cr,
                    });
                    let info = text_info(d.encoding.name(), d.has_bom, Some(d.newline));
                    let lines = d.editor.lines.clone();
                    view.replace_side(side, lines, eol, info);
                }
            }
            view.retarget(outlined);
            (view.sources, view.seen) = (now, now);
            view.changed_at = None;
            self.compare = Some(view);
            return;
        }
        let old = view;
        match self.load_compare(&old.paths, old.binary) {
            Ok(mut view) => {
                view.keep_place(&old);
                self.compare = Some(Box::new(view));
            }
            // The window would no longer match the files: say why and close it.
            Err(e) => self.compare_error(e),
        }
    }

    /// Shows the compare window (its own native window where the backend supports it).
    pub fn show_compare(&mut self, ctx: &egui::Context) {
        // A compared tab was edited, undone, opened or closed: compare again once the changes
        // pause, rather than on every keystroke.
        let now = ctx.input(|i| i.time);
        if let Some(v) = self.compare.as_ref().filter(|v| !v.is_binary()) {
            let current = self.compare_sources(v);
            if current != v.sources {
                let v = self.compare.as_mut().unwrap();
                // Each further change restarts the wait, so steady typing never re-diffs.
                if current != v.seen || v.changed_at.is_none() {
                    v.seen = current;
                    v.changed_at = Some(now);
                }
                let wait = REFRESH_DELAY - (now - v.changed_at.unwrap_or(now));
                if wait <= 0.0 {
                    self.refresh_compare();
                } else {
                    ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
                }
            }
        }
        let Some(mut view) = self.compare.take() else { return };
        let lang = self.ui_lang();
        let kind = if view.is_binary() { self.t("cmp.binary", "二進位比較") } else { self.t("cmp.text", "文字比較") };
        let title = format!("{kind} - {} ↔ {}", file_name(&view.names[0]), file_name(&view.names[1]));
        let builder = egui::ViewportBuilder::default().with_title(title).with_inner_size([1100.0, 760.0]).with_min_inner_size([480.0, 320.0]);
        let action = ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("madedit-compare"), builder, |ui, _class| view.ui(ui, lang));
        match action {
            CompareAction::None => self.compare = Some(view),
            CompareAction::Close => {}
            CompareAction::Merge(to_right) => {
                self.compare = Some(view);
                self.merge_compare(to_right);
            }
            CompareAction::Options(o) => {
                self.compare = Some(view);
                self.set_compare_text_options(o);
            }
        }
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// A shown path's file name and its folder (without the trailing separator).
pub(super) fn name_and_folder(path: &str) -> (&str, &str) {
    let name = file_name(path);
    (name, path[..path.len() - name.len()].trim_end_matches(['/', '\\']))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> OpenOptions {
        OpenOptions::default()
    }

    fn write(dir: &Path, name: &str, data: &[u8]) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, data).unwrap();
        p
    }

    #[test]
    fn text_files_compare_by_line_ignoring_newline_style() {
        let dir = tempfile::tempdir().unwrap();
        let a = write(dir.path(), "a.txt", b"one\r\ntwo\r\nthree\r\n");
        let b = write(dir.path(), "b.txt", b"one\nTWO\nthree\n");
        let mut v = CompareView::load([&a, &b], false, [None, None], [None, None], TextOptions::default(), &opts()).unwrap();
        assert!(!v.is_binary());
        let Content::Text { diff, lines, .. } = &v.content else { panic!() };
        assert_eq!(lines[0], vec!["one", "two", "three", ""]);
        assert_eq!(diff.hunks, vec![1..2]);
        assert_eq!(v.hunk, Some(0), "the first difference is current on open");
        assert_eq!(v.goto_row, Some(1));
        assert!(!v.next() && !v.prev(), "only one difference");
        assert_eq!(v.hunk, Some(0));
    }

    #[test]
    fn open_lines_replace_the_file_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let a = write(dir.path(), "a.txt", b"same\n");
        let b = write(dir.path(), "b.txt", b"same\n");
        let v = CompareView::load([&a, &b], false, [Some(vec!["edited".into(), String::new()]), None], [None, None], TextOptions::default(), &opts()).unwrap();
        assert!(!v.is_identical());
        let v = CompareView::load([&a, &b], false, [None, None], [None, None], TextOptions::default(), &opts()).unwrap();
        assert!(v.is_identical());
        assert_eq!(v.hunk, None);
    }

    #[test]
    fn binary_data_or_the_binary_option_compares_bytes() {
        let dir = tempfile::tempdir().unwrap();
        // NUL bytes in no UTF-16 pattern: binary data.
        let a = write(dir.path(), "a.bin", &[0, 0, 0, 0, 1, 2, 3, 4]);
        let b = write(dir.path(), "b.bin", &[0, 0, 0, 0, 1, 9, 3, 4, 5]);
        let mut v = CompareView::load([&a, &b], false, [None, None], [None, None], TextOptions::default(), &opts()).unwrap();
        assert!(v.is_binary());
        let Content::Binary { diff, .. } = &v.content else { panic!() };
        assert_eq!(diff.differing, 2);
        assert_eq!(v.run, Some(5..6));
        assert!(v.next());
        assert_eq!(v.run, Some(8..9));
        assert!(!v.next());
        assert!(v.prev());
        assert_eq!(v.run, Some(5..6));

        // Text files compared as binary, even when open as text.
        let t = write(dir.path(), "t.txt", b"abc");
        let v = CompareView::load([&t, &t], true, [Some(vec!["xyz".into()]), None], [None, None], TextOptions::default(), &opts()).unwrap();
        assert!(v.is_binary() && v.is_identical());
    }

    #[test]
    fn unreadable_files_report_their_path() {
        let dir = tempfile::tempdir().unwrap();
        let a = write(dir.path(), "a.txt", b"x");
        let missing = dir.path().join("missing.txt");
        match CompareView::load([&a, &missing], false, [None, None], [None, None], TextOptions::default(), &opts()) {
            Err(CompareError::Read(p, _)) => assert_eq!(p, missing),
            other => panic!("unexpected {:?}", other.map(|_| ())),
        }
        assert!(matches!(CompareView::load([&a, dir.path()], false, [None, None], [None, None], TextOptions::default(), &opts()), Err(CompareError::Read(..))));
    }

    #[test]
    fn footer_info_and_word_highlights() {
        assert_eq!(text_info("UTF-8", false, Some(NewLineFormat::Unix)), "UTF-8 · LF");
        assert_eq!(text_info("UTF-16LE", true, Some(NewLineFormat::Dos)), "UTF-16LE BOM · CRLF");
        assert_eq!(text_info("Big5", false, None), "Big5");
        let font = FontId::monospace(12.0);
        let job = line_job("ab cd ef", &[3..5], Color32::RED, &font, Color32::GRAY, Color32::WHITE);
        assert_eq!(job.text, "ab cd ef");
        let parts: Vec<(&str, Color32)> = job.sections.iter().map(|s| (&job.text[s.byte_range.start.0..s.byte_range.end.0], s.format.background)).collect();
        assert_eq!(parts, vec![("ab ", Color32::TRANSPARENT), ("cd", Color32::RED), (" ef", Color32::TRANSPARENT)]);
    }

    #[test]
    fn differences_are_coloured_by_the_sides_that_have_lines() {
        assert_eq!(solid(CHANGED), Color32::from_rgb(255, 204, 0));
        let rows = |kinds: &[(Option<usize>, Option<usize>)]| {
            let left: Vec<String> = kinds.iter().filter_map(|k| k.0).map(|i| format!("l{i}")).collect();
            let right: Vec<String> = kinds.iter().filter_map(|k| k.1).map(|i| format!("r{i}")).collect();
            compare::diff_lines(&left, &right)
        };
        let d = rows(&[(Some(0), None)]);
        assert_eq!(hunk_color(&d, &d.hunks[0]), DELETED);
        let d = rows(&[(None, Some(0))]);
        assert_eq!(hunk_color(&d, &d.hunks[0]), INSERTED);
        let d = rows(&[(Some(0), Some(0))]);
        assert_eq!(hunk_color(&d, &d.hunks[0]), CHANGED);
        let v = |s: &[&str]| s.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let d = compare::diff_lines(&v(&["moved", "k1", "k2"]), &v(&["k1", "k2", "moved"]));
        assert_eq!(d.hunks.len(), 2);
        assert!(d.hunks.iter().all(|h| hunk_color(&d, h) == MOVED));
    }

    #[test]
    fn merged_lines_keep_bookmarks_on_their_text() {
        let mut ed = EditorState::from_text("a
b
c
d
e");
        ed.bookmarks = [0, 2, 4].into();
        merge_lines(&mut ed, 1..3, vec!["X".into(), "Y".into(), "Z".into()]);
        assert_eq!(ed.lines, vec!["a", "X", "Y", "Z", "d", "e"]);
        assert_eq!(ed.bookmarks, [0, 5].into(), "the bookmark on c went with it; e's moved down one");
        merge_lines(&mut ed, 0..6, Vec::new());
        assert_eq!(ed.lines, vec![""]);
        assert!(ed.bookmarks.is_empty());
    }

    #[test]
    fn unknown_line_endings_are_not_compared_and_say_so() {
        let dir = tempfile::tempdir().unwrap();
        let a = write(dir.path(), "a.txt", b"x\r\ny\n");
        let b = write(dir.path(), "b.txt", b"x\ny\n");
        let counted = TextOptions { ignore_eol: false, ..TextOptions::default() };
        // From disk, each line's ending is known: the CRLF line differs.
        let v = CompareView::load([&a, &b], false, [None, None], [None, None], counted, &opts()).unwrap();
        assert!(!v.is_identical());
        // The left side from a tab with mixed newlines: its endings are unknown, so not
        // compared, and the summary says why.
        let open = Some(vec!["x".to_string(), "y".to_string(), String::new()]);
        let v = CompareView::load([&a, &b], false, [open, None], [None, None], counted, &opts()).unwrap();
        assert!(v.is_identical());
        assert!(v.summary(Lang::ZhTw).contains("未比較換行符號"), "{}", v.summary(Lang::ZhTw));
        // A tab with one newline style is compared as ending every line that way.
        let open = Some(vec!["x".to_string(), "y".to_string(), String::new()]);
        let v = CompareView::load([&a, &b], false, [open, None], [Some(Eol::Lf), None], counted, &opts()).unwrap();
        assert!(v.is_identical() && !v.summary(Lang::ZhTw).contains("未比較"));
    }

    #[test]
    fn only_a_difference_in_view_is_selected_automatically() {
        let dir = tempfile::tempdir().unwrap();
        let body: String = (0..200).map(|i| format!("line {i}\n")).collect();
        let a = write(dir.path(), "a.txt", format!("A\n{body}").as_bytes());
        let b = write(dir.path(), "b.txt", format!("B\n{body}").as_bytes());
        let mut v = CompareView::load([&a, &b], false, [None, None], [None, None], TextOptions::default(), &opts()).unwrap();
        (v.hunk, v.visible) = (None, 20.0);
        // Scrolled to the bottom, the only difference (row 0) is out of view: none selected.
        v.scroll = 150.0;
        v.keep_hunk();
        assert_eq!(v.hunk, None);
        // In view, it is.
        v.scroll = 0.0;
        v.keep_hunk();
        assert_eq!(v.hunk, Some(0));
        // show_hunk brings an off-screen difference into view.
        v.scroll = 150.0;
        v.show_hunk(0);
        assert_eq!(v.goto_row, Some(0));
    }

    #[test]
    fn changing_options_keeps_the_outlined_change() {
        let dir = tempfile::tempdir().unwrap();
        // Blank-only differences above the real one; ignoring blank lines removes them.
        let a = write(dir.path(), "a.txt", b"x\n\ny\n\nz\nLEFT\nend\n");
        let b = write(dir.path(), "b.txt", b"x\ny\nz\nRIGHT\nend\n");
        let mut v = CompareView::load([&a, &b], false, [None, None], [None, None], TextOptions::default(), &opts()).unwrap();
        v.visible = 50.0;
        let Content::Text { diff, .. } = &v.content else { panic!() };
        let real = diff.hunks.len() - 1;
        v.hunk = Some(real);
        let before = v.merge_plan(true).unwrap();
        assert_eq!(before.source, vec!["LEFT".to_string()]);
        v.set_text_options(TextOptions { ignore_blank_lines: true, ..TextOptions::default() });
        assert_eq!(v.merge_plan(true).unwrap().source, before.source, "still LEFT/RIGHT, not the old index");
        // Turned back: the same change, at its index again.
        v.set_text_options(TextOptions::default());
        assert_eq!(v.hunk, Some(real));
    }

    #[test]
    fn a_repeated_change_is_found_where_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let mid: String = (0..39).map(|i| format!("m{i}\n")).collect();
        let a = format!("top\nLEFT\n{mid}LEFT\nend\n");
        let b = format!("top\nRIGHT\n{mid}RIGHT\nend\n");
        let pa = write(dir.path(), "a.txt", a.as_bytes());
        let pb = write(dir.path(), "b.txt", b.as_bytes());
        let mut v = CompareView::load([&pa, &pb], false, [None, None], [None, None], TextOptions::default(), &opts()).unwrap();
        v.visible = 100.0;
        v.hunk = Some(1);
        let outlined = v.merge_plan(true).unwrap();
        assert_eq!(outlined.target.start, 41);
        // 35 lines added at the top of b (the merge target): its line numbers move, a's don't.
        let pasted: String = (0..35).map(|i| format!("p{i}\n")).collect();
        let b2 = format!("{pasted}top\nRIGHT\n{mid}RIGHT\nend\n");
        let lines = EditorState::from_text(&b2).lines;
        v.replace_side(1, lines, None, String::new());
        v.retarget(Some(outlined));
        let now = v.merge_plan(true).unwrap();
        assert_eq!((now.source_range.start, now.target.start), (41, 76), "the second LEFT/RIGHT, not the first at 36");
    }

    #[test]
    fn file_name_takes_the_last_component() {
        assert_eq!(file_name(r"C:\a\b.txt"), "b.txt");
        assert_eq!(file_name("/x/y.rs"), "y.rs");
        assert_eq!(name_and_folder(r"C:\a\b.txt"), ("b.txt", r"C:\a"));
        assert_eq!(name_and_folder("y.rs"), ("y.rs", ""));
        assert_eq!(digits(0), 1);
        assert_eq!(digits(1000), 4);
    }
}
