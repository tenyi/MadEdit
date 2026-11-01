//! 對話框：選項、字型、排序、遞增數字、全形半形、在檔案中尋找、編碼轉換、字數統計等，以及確認視窗

use eframe::egui::{self, Key, RichText};

use super::commands::{self, plain_label, C};
use super::form;
use super::i18n::{self, Lang};
use super::{App, Confirm};
use crate::core::document::EditKind;
use crate::core::encoding::{self, ALL_ENCODINGS};
use crate::core::model::EditMode;
use crate::core::settings::{OpenViewMode, PrintOptions, Settings, ThemeChoice, UiLanguage, WidthOptions};
use crate::core::spell;
use crate::core::syntax;
use crate::core::textops::{self, WordCount};

/// 對話框視窗：第一次出現時置於主視窗中央（之後可自由拖曳）
fn dialog_window(ctx: &egui::Context, title: impl Into<egui::WidgetText>) -> egui::Window<'static> {
    // Dialogs are not collapsible: the title-bar arrow only hides the content and looks dated.
    egui::Window::new(title)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .collapsible(false)
}

/// Accent-filled button for a dialog's main, safe action (OK, Save). Destructive choices such as
/// Overwrite or Reload use `danger_button` instead, so they never look like the default.
fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(text).color(egui::Color32::WHITE)).fill(form::ACCENT))
}

/// Button for a choice that throws work away (Don't Save, Overwrite, Clear), in the error colour.
fn danger_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let color = ui.visuals().error_fg_color;
    ui.add(egui::Button::new(RichText::new(text).color(color)))
}

/// A dialog's button row below a separator, right-aligned like the Options dialog. `add` lays
/// the buttons out right to left, so the first one it adds (the primary action) is rightmost.
fn footer<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.add_space(4.0);
    ui.separator();
    ui.horizontal(|ui| ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add).inner).inner
}

/// A dialog's content width: `width`, or less when the main window is narrower.
fn dialog_width(ctx: &egui::Context, width: f32) -> f32 {
    width.min(ctx.content_rect().width() - DIALOG_SIDE_MARGIN).max(240.0)
}

/// A dialog's content above its footer. It scrolls when the main window is too short for it,
/// so the footer buttons always stay on screen. Shorter content keeps its natural height.
/// `opened` (the dialog's first frame) starts it at the top rather than where it was left.
fn dialog_body<R>(ui: &mut egui::Ui, opened: bool, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let max_height = (ui.ctx().content_rect().height() - DIALOG_CHROME_HEIGHT).max(80.0);
    // Without the minimum, the window's sizing pass squeezes the area to the default window
    // height and pushes the footer out of the window.
    let mut area = egui::ScrollArea::vertical().id_salt("dialog-body").auto_shrink([false, true]).min_scrolled_height(max_height).max_height(max_height);
    if opened {
        area = area.vertical_scroll_offset(0.0);
    }
    area.show(ui, add).inner
}

/// A card holding a fixed-height scrolling list of `form::list_row`s, so filtering a list never
/// resizes its dialog.
fn list_card(ui: &mut egui::Ui, id: &str, height: f32, add: impl FnOnce(&mut egui::Ui)) {
    form::card(ui, egui::Margin::same(4), |ui| {
        let area = egui::ScrollArea::vertical().id_salt(id).auto_shrink([false, false]).min_scrolled_height(height).max_height(height);
        area.show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            add(ui);
        });
    });
}

/// The title and text of a confirmation, at the same width and type as the dialogs.
fn confirm_body(ui: &mut egui::Ui, opened: bool, title: &str, texts: &[&str]) {
    ui.set_width(dialog_width(ui.ctx(), 400.0));
    ui.label(RichText::new(title).size(16.0).strong());
    ui.add_space(4.0);
    dialog_body(ui, opened, |ui| {
        for text in texts {
            ui.add(egui::Label::new(*text).wrap());
        }
    });
}

/// A small explanatory paragraph under a dialog's cards.
fn hint(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).small().weak()).wrap());
}

/// 選項對話框的狀態
#[derive(Default)]
pub struct OptionsState {
    pub(super) tab: usize,
    /// 編輯中的設定副本（按「確定」或「套用」才寫回）
    draft: Option<Settings>,
    /// The settings the draft started from (or was last applied as). Only fields that differ
    /// between the two are written back, so live changes made meanwhile are kept.
    base: Option<Settings>,
    /// 正在擷取快捷鍵的命令
    pub capturing: Option<C>,
    selected: Option<C>,
    filter: String,
    /// Theme and language last seen in the live settings (defaults until the first frame, when
    /// the draft is a fresh copy anyway). The View menu and toolbar can change them while this
    /// non-modal dialog is open; the draft follows those changes unless the user already picked
    /// a different value here, so OK / Apply cannot revert them.
    live_theme: ThemeChoice,
    live_language: UiLanguage,
}

pub enum Dialog {
    Options(Box<OptionsState>),
    Font { filter: String, family: String, size: f32 },
    PageSetup,
    RecentFiles { filter: String },
    Width { full: bool },
    InsertNumbers,
    Sort,
    FindInFiles { query: String, replacement: String },
    TempMacro { script: String, error: Option<String> },
    Highlighting,
    SourceFormatter,
    PurgeHistories,
    ConvertEncoding { encoding: String, bom: bool },
    WordCount(WordCount, bool),
    WindowList,
    /// `copied_at` is when "Copy info" was last clicked (egui time), for a short confirmation.
    About { copied_at: Option<f64> },
    /// Help > Keyboard Shortcuts: the current bindings grouped by menu, with a filter.
    Shortcuts { filter: String },
}

/// Fixed size of the Options dialog's sidebar and page area, so switching pages never resizes
/// the window.
const OPTIONS_SIZE: egui::Vec2 = egui::vec2(720.0, 440.0);

/// Index of the Keys tab in the Options dialog, opened from Help > Keyboard Shortcuts.
pub(super) const OPTIONS_KEYS_TAB: usize = 4;

/// Seconds the About dialog shows "Copied" after "Copy info".
const COPIED_SECONDS: f64 = 2.0;

/// Width of the Page Setup dialog (its height follows the content).
const PAGE_SETUP_WIDTH: f32 = 480.0;

/// Size of the Keyboard Shortcuts dialog's list, which shows the menus in two columns.
const SHORTCUTS_SIZE: egui::Vec2 = egui::vec2(760.0, 440.0);

/// Below this list width the Keyboard Shortcuts dialog uses a single column.
const SHORTCUTS_TWO_COLUMN_WIDTH: f32 = 560.0;

/// Height the Options dialog needs around its pages: title bar, separator, footer buttons
/// and margins.
const OPTIONS_CHROME_HEIGHT: f32 = 110.0;

/// Height the Keyboard Shortcuts dialog needs around its list: title bar, filter row,
/// separators, footer buttons and margins.
const SHORTCUTS_CHROME_HEIGHT: f32 = 150.0;

/// Height a list dialog (Font, Recent Files, Window List, Temporary Macro) needs around its
/// list: title bar, filter or hint, footer buttons and margins.
const LIST_DIALOG_CHROME_HEIGHT: f32 = 140.0;

/// Height a dialog needs outside `dialog_body`: title bar, footer buttons, margins and room
/// above and below the window.
const DIALOG_CHROME_HEIGHT: f32 = 120.0;

/// Height the Font dialog needs below its list on top of `LIST_DIALOG_CHROME_HEIGHT`: the
/// size row and the note about fallback fonts.
const FONT_DIALOG_EXTRA_HEIGHT: f32 = 90.0;

/// Room kept between a dialog and the sides of the main window.
const DIALOG_SIDE_MARGIN: f32 = 48.0;

/// A fixed dialog content size, shrunk when the main window (which can be as small as
/// 400 × 300) has less room, so the footer buttons always stay inside the window. `chrome` is
/// the height the dialog needs outside this content.
fn fit_to_window(ctx: &egui::Context, size: egui::Vec2, chrome: f32) -> egui::Vec2 {
    let room = ctx.content_rect().size() - egui::vec2(DIALOG_SIDE_MARGIN, chrome);
    size.min(room).max(egui::vec2(240.0, 80.0))
}

/// First command of each menu section in `commands::ALL` (the table is ordered by menu),
/// with the menu's translation key and Traditional Chinese title.
const COMMAND_SECTIONS: &[(C, &str, &str)] = &[
    (C::New, "menu.file", "檔案(F)"),
    (C::Undo, "menu.edit", "編輯(E)"),
    (C::Find, "menu.search", "搜尋(S)"),
    (C::AlwaysOnTop, "menu.view", "檢視(V)"),
    (C::RunTempMacro, "menu.macro", "巨集(M)"),
    (C::Options, "menu.tools", "工具(T)"),
    (C::ToggleWindow, "menu.window", "視窗(W)"),
    (C::KeyboardShortcuts, "menu.help", "說明(H)"),
];

/// Index into `COMMAND_SECTIONS` of the menu each command in `commands::ALL` belongs to,
/// in the same order, found in one pass over the table.
fn command_sections() -> Vec<usize> {
    let mut section = 0;
    commands::ALL
        .iter()
        .map(|&c| {
            if COMMAND_SECTIONS.get(section + 1).is_some_and(|&(first, _, _)| first == c) {
                section += 1;
            }
            section
        })
        .collect()
}

/// Where to split blocks of the given heights into two columns: blocks stay whole and in
/// order, the left column gets at least one block, and the split makes the two columns as
/// close in height as possible.
fn column_split(heights: &[usize]) -> usize {
    let total: i64 = heights.iter().map(|&h| h as i64).sum();
    let mut left = 0i64;
    for (i, &h) in heights.iter().enumerate() {
        // Stop before a block that would make the left column overshoot by more than the
        // right column now leads.
        if i > 0 && (left + h as i64) * 2 - total > total - left * 2 {
            return i;
        }
        left += h as i64;
    }
    heights.len()
}

/// The path with the home directory shown as "~", so copied info does not reveal the user name.
fn tilde_path(path: &std::path::Path) -> String {
    match dirs::home_dir().and_then(|home| path.strip_prefix(home).ok().map(std::path::Path::to_path_buf)) {
        Some(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        None => path.display().to_string(),
    }
}

/// 列印選項（頁面設定與選項對話框共用）
fn print_options_ui(ui: &mut egui::Ui, p: &mut PrintOptions, lang: Lang) {
    let t = |k: &str, zh: &'static str| i18n::translate(lang, k, zh);
    form::section(ui, t("print.sec.content", "列印內容"), |s| {
        s.toggle(t("print.syntax", "列印語法色彩"), &mut p.syntax_colors);
        s.toggle(t("print.lines", "列印行號"), &mut p.line_numbers);
        s.toggle(t("print.bookmarks", "列印書籤"), &mut p.bookmarks);
        s.toggle(t("print.eol", "列印行尾字元"), &mut p.end_of_line);
        s.toggle(t("print.tabs", "列印 Tab 字元"), &mut p.tab_chars);
        s.toggle(t("print.spaces", "列印空白字元"), &mut p.space_chars);
        let choices = [(0, t("print.none", "無")), (1, t("print.first", "第一頁")), (2, t("print.every", "每頁"))];
        s.row(t("print.hex_header", "十六進位模式位移標頭："), |ui| form::segmented(ui, &mut p.hex_offset_header, &choices));
    });
    form::section(ui, t("print.sec.header_footer", "頁首與頁尾"), |s| {
        s.toggle(t("print.header", "頁首"), &mut p.header);
        s.body(|ui| three_fields(ui, p.header, [&mut p.header_left, &mut p.header_center, &mut p.header_right]));
        s.toggle(t("print.footer", "頁尾"), &mut p.footer);
        s.body(|ui| three_fields(ui, p.footer, [&mut p.footer_left, &mut p.footer_center, &mut p.footer_right]));
        s.note(t("print.vars", "變數：%f 檔名　%p 路徑　%n 頁碼　%s 總頁數　%d 日期　%t 時間"));
    });
}

/// The left, centre and right parts of a page header or footer, sharing the row equally.
fn three_fields(ui: &mut egui::Ui, enabled: bool, fields: [&mut String; 3]) {
    ui.add_enabled_ui(enabled, |ui| {
        ui.horizontal(|ui| {
            let width = ((ui.available_width() - 2.0 * ui.spacing().item_spacing.x) / 3.0).floor();
            for field in fields {
                ui.add(egui::TextEdit::singleline(field).desired_width(width));
            }
        });
    });
}

fn encoding_combo(ui: &mut egui::Ui, id: &str, value: &mut String) {
    egui::ComboBox::from_id_salt(id).selected_text(value.as_str()).width(200.0).show_ui(ui, |ui| {
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            for &e in ALL_ENCODINGS {
                ui.selectable_value(value, e.to_string(), e);
            }
        });
    });
}

fn width_options_ui(ui: &mut egui::Ui, w: &mut WidthOptions, lang: Lang) {
    let t = |k: &str, zh: &'static str| i18n::translate(lang, k, zh);
    form::section(ui, "", |s| {
        s.toggle(t("width.letters", "英文字母"), &mut w.letters);
        s.toggle(t("width.digits", "數字"), &mut w.digits);
        s.toggle(t("width.symbols", "符號"), &mut w.symbols);
        s.toggle(t("width.space", "空白"), &mut w.space);
        s.toggle(t("width.katakana", "日文片假名"), &mut w.katakana);
    });
}

impl App {
    pub fn show_dialogs(&mut self, ctx: &egui::Context) {
        let Some(mut dlg) = self.dialog.take() else {
            self.shown_dialog = None;
            return;
        };
        let kind = std::mem::discriminant(&dlg);
        let opened = self.shown_dialog != Some(kind);
        let mut open = true;
        let mut close = false;
        // Set when a dialog hands over to another one (About → Shortcuts → Options > Keys).
        let mut switch_to: Option<Dialog> = None;
        match &mut dlg {
            Dialog::Options(st) => self.options_dialog(ctx, st, &mut open),
            Dialog::Font { filter, family, size } => {
                let filter_hint = form::bare(self.t("font.filter", "篩選：")).to_string();
                let l_auto = self.t("font.auto", "（自動選擇等寬字型）");
                let l_size = self.t("font.size", "大小：");
                let l_cjk = self.t("font.cjk", "中文、日文等字元若字型中沒有，會自動以系統的 CJK 字型補字。比例字型請搭配關閉「等寬模式」。");
                let (l_ok, l_cancel) = (self.t("btn.ok", "確定"), self.t("btn.cancel", "取消"));
                let fit = fit_to_window(ctx, egui::vec2(440.0, 260.0), LIST_DIALOG_CHROME_HEIGHT + FONT_DIALOG_EXTRA_HEIGHT);
                let mut apply = false;
                dialog_window(ctx, self.t("dlg.font", "字型")).open(&mut open).resizable(false).show(ctx, |ui| {
                    ui.set_width(fit.x);
                    ui.add(egui::TextEdit::singleline(filter).hint_text(filter_hint).desired_width(f32::INFINITY));
                    ui.add_space(6.0);
                    list_card(ui, "fonts", fit.y, |ui| {
                        if form::list_row(ui, egui::Role::Button, family.is_empty(), l_auto, "").clicked() {
                            family.clear();
                        }
                        let f = filter.to_lowercase();
                        for name in &self.fonts.families {
                            if (f.is_empty() || name.to_lowercase().contains(&f)) && form::list_row(ui, egui::Role::Button, name == family, name, "").clicked() {
                                *family = name.clone();
                            }
                        }
                    });
                    ui.add_space(8.0);
                    form::section(ui, "", |s| {
                        s.row(l_size, |ui| ui.add(egui::DragValue::new(size).range(6.0..=72.0).speed(0.5)));
                    });
                    hint(ui, l_cjk);
                    footer(ui, |ui| {
                        apply = primary_button(ui, l_ok).clicked();
                        if ui.button(l_cancel).clicked() {
                            close = true;
                        }
                    });
                });
                if apply {
                    self.settings.font_family = family.clone();
                    self.settings.font_size = *size;
                    self.save_settings();
                    close = true;
                }
            }
            Dialog::PageSetup => {
                let lang = self.ui_lang();
                let l_browser = self.t("print.browser", "列印時會以瀏覽器開啟排版後的頁面，紙張大小與邊界請在瀏覽器的列印對話框中設定。");
                let l_close = self.t("btn.close", "關閉");
                dialog_window(ctx, self.t("dlg.page", "頁面設定")).open(&mut open).resizable(false).show(ctx, |ui| {
                    ui.set_width(dialog_width(ctx, PAGE_SETUP_WIDTH));
                    dialog_body(ui, opened, |ui| {
                        print_options_ui(ui, &mut self.settings.print, lang);
                        hint(ui, l_browser);
                    });
                    footer(ui, |ui| {
                        if primary_button(ui, l_close).clicked() {
                            close = true;
                        }
                    });
                });
            }
            Dialog::RecentFiles { filter } => {
                let mut chosen = None;
                let filter_hint = form::bare(self.t("opt.filter", "篩選：")).to_string();
                let l_close = self.t("btn.close", "關閉");
                let fit = fit_to_window(ctx, egui::vec2(560.0, 320.0), LIST_DIALOG_CHROME_HEIGHT);
                dialog_window(ctx, self.t("dlg.recent", "最近使用的檔案")).open(&mut open).resizable(false).show(ctx, |ui| {
                    ui.set_width(fit.x);
                    let field = ui.add(egui::TextEdit::singleline(filter).hint_text(filter_hint).desired_width(f32::INFINITY));
                    // Focused only when the dialog opens, so Tab can still move on to the list.
                    if opened {
                        field.request_focus();
                    }
                    ui.add_space(6.0);
                    let f = filter.to_lowercase();
                    let matches = |p: &std::path::PathBuf| f.is_empty() || p.display().to_string().to_lowercase().contains(&f);
                    list_card(ui, "recent", fit.y, |ui| {
                        for p in self.settings.recent_files.iter().filter(|p| matches(p)) {
                            // The file name, with its folder dimmed at the right.
                            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string());
                            let folder = p.parent().map(tilde_path).unwrap_or_default();
                            if form::list_row(ui, egui::Role::Button, false, &name, &folder).on_hover_text(p.display().to_string()).clicked() {
                                chosen = Some(p.clone());
                            }
                        }
                    });
                    // Enter in the filter opens the first match; Enter on a focused row opens that row.
                    if chosen.is_none() && field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        chosen = self.settings.recent_files.iter().find(|p| matches(p)).cloned();
                    }
                    footer(ui, |ui| {
                        if ui.button(l_close).clicked() {
                            close = true;
                        }
                    });
                });
                if let Some(p) = chosen {
                    self.open_path(&p);
                    close = true;
                }
            }
            Dialog::Width { full } => {
                let full = *full;
                let title = if full { self.t("dlg.fullwidth", "依選項轉為全形") } else { self.t("dlg.halfwidth", "依選項轉為半形") };
                let lang = self.ui_lang();
                let (l_ok, l_cancel) = (self.t("btn.ok", "確定"), self.t("btn.cancel", "取消"));
                let mut convert = false;
                dialog_window(ctx, title).open(&mut open).resizable(false).show(ctx, |ui| {
                    ui.set_width(dialog_width(ctx, 360.0));
                    dialog_body(ui, opened, |ui| {
                        width_options_ui(ui, &mut self.settings.width, lang);
                    });
                    footer(ui, |ui| {
                        convert = primary_button(ui, l_ok).clicked();
                        if ui.button(l_cancel).clicked() {
                            close = true;
                        }
                    });
                });
                if convert {
                    let o = self.settings.width.clone();
                    let text = self.doc().editor.selected_text();
                    let all = text.is_empty();
                    let src = if all { self.doc().editor.lines.join("\n") } else { text };
                    let out = if full { textops::to_fullwidth(&src, &o) } else { textops::to_halfwidth(&src, &o) };
                    let doc = self.doc_mut();
                    if all {
                        doc.replace_all_text(&out);
                    } else {
                        let hint = doc.selection_hint();
                        doc.edit(EditKind::Other, hint, |ed| ed.replace_selection(&out));
                    }
                    close = true;
                }
            }
            Dialog::InsertNumbers => {
                let title = self.t("dlg.numbers", "插入遞增數字");
                let l_initial = self.t("num.initial", "起始值：");
                let l_step = self.t("num.step", "遞增值：");
                let l_count = self.t("num.count", "數量（無列塊選取時）：");
                let l_format = self.t("num.format", "格式：");
                let l_base = self.t("num.base", "進位：");
                let bases = [(0, self.t("num.dec", "十進位")), (1, self.t("num.hex", "十六進位")), (2, self.t("num.oct", "八進位")), (3, self.t("num.bin", "二進位"))];
                let l_affix = self.t("num.affix", "前綴／後綴：");
                let l_upper = self.t("num.upper", "十六進位使用大寫");
                let l_align = self.t("num.align", "對齊：");
                let aligns = [(false, self.t("num.right", "靠右對齊")), (true, self.t("num.left", "靠左對齊"))];
                let l_pad = self.t("num.pad", "前面補零");
                let l_hint = self.t("num.hint", "列塊模式下會在矩形選取的每一行插入一個數字；否則自游標處起每行插入一個。");
                let (l_ok, l_cancel) = (self.t("btn.ok", "確定"), self.t("btn.cancel", "取消"));
                let mut insert = false;
                dialog_window(ctx, title).open(&mut open).resizable(false).show(ctx, |ui| {
                    ui.set_width(dialog_width(ctx, 440.0));
                    dialog_body(ui, opened, |ui| {
                        let n = &mut self.settings.numbers;
                        form::section(ui, "", |s| {
                            s.row(l_initial, |ui| ui.add(egui::DragValue::new(&mut n.initial)));
                            s.row(l_step, |ui| ui.add(egui::DragValue::new(&mut n.step)));
                            s.row(l_count, |ui| ui.add(egui::DragValue::new(&mut n.total).range(1..=1_000_000)));
                        });
                        form::section(ui, l_format, |s| {
                            s.row(l_base, |ui| form::segmented(ui, &mut n.format, &bases));
                            s.toggle(l_upper, &mut n.uppercase_hex);
                            // Right to left: the suffix field first, so the prefix reads first.
                            s.row(l_affix, |ui| {
                                ui.add(egui::TextEdit::singleline(&mut n.postfix).desired_width(60.0));
                                ui.add(egui::TextEdit::singleline(&mut n.prefix).desired_width(60.0));
                            });
                            s.row(l_align, |ui| form::segmented(ui, &mut n.align_left, &aligns));
                            s.toggle(l_pad, &mut n.zero_pad);
                        });
                        hint(ui, l_hint);
                    });
                    footer(ui, |ui| {
                        insert = primary_button(ui, l_ok).clicked();
                        if ui.button(l_cancel).clicked() {
                            close = true;
                        }
                    });
                });
                if insert {
                    self.insert_numbers();
                    close = true;
                }
            }
            Dialog::Sort => {
                let title = self.t("dlg.sort", "排序選項");
                let l_order = self.t("sort.order", "順序：");
                let orders = [(false, self.t("sort.asc", "遞增")), (true, self.t("sort.desc", "遞減"))];
                let l_case = self.t("sort.case", "區分大小寫");
                let l_num = self.t("sort.numeric", "依數值排序");
                let l_dup = self.t("sort.unique", "移除重複行");
                let l_go = self.t("sort.go", "排序");
                let l_close = self.t("btn.close", "關閉");
                dialog_window(ctx, title).open(&mut open).resizable(false).show(ctx, |ui| {
                    ui.set_width(dialog_width(ctx, 380.0));
                    dialog_body(ui, opened, |ui| {
                        let s = &mut self.settings.sort;
                        form::section(ui, "", |sec| {
                            sec.row(l_order, |ui| form::segmented(ui, &mut s.descending, &orders));
                            sec.toggle(l_case, &mut s.case_sensitive);
                            sec.toggle(l_num, &mut s.numeric);
                            sec.toggle(l_dup, &mut s.remove_duplicates);
                        });
                    });
                    footer(ui, |ui| {
                        if primary_button(ui, l_go).clicked() {
                            self.pending.push(C::SortByOptions);
                            close = true;
                        }
                        if ui.button(l_close).clicked() {
                            close = true;
                        }
                    });
                });
            }
            Dialog::FindInFiles { query, replacement } => {
                let mut start: Option<Option<String>> = None;
                let title = self.t("dlg.find_files", "在檔案中尋找／取代");
                let l_find = self.t("fif.find", "尋找：");
                let l_rep = self.t("fif.replace", "取代為：");
                let l_scope = self.t("fif.scope", "搜尋範圍");
                let l_dir = self.t("fif.folder", "資料夾：");
                let l_browse = self.t("fif.browse", "選擇資料夾");
                let l_types = self.t("fif.types", "檔案類型：");
                let l_types_hint = self.t("fif.types_hint", "例如 *.rs;*.toml");
                let l_ex = self.t("fif.exclude", "排除：");
                let l_ex_hint = self.t("fif.exclude_hint", "例如 target;*.min.js");
                let l_sub = self.t("fif.subdirs", "包含子資料夾");
                let l_opened = self.t("fif.opened", "只在已開啟的檔案中");
                let l_match = self.t("fif.match", "比對方式");
                let l_case = self.t("fif.case", "區分大小寫");
                let l_word = self.t("fif.whole_word", "全字比對");
                let l_regex = self.t("fif.regex", "使用正規表示式（Regex）");
                let l_warn = self.t("fif.warn", "取代未開啟的檔案會直接寫入磁碟且無法復原；已開啟的檔案在編輯器中取代，可復原。");
                let l_find_all = self.t("fif.find_all", "全部尋找");
                let l_rep_all = self.t("find.replace_all", "全部取代");
                let l_close = self.t("btn.close", "關閉");
                let width = dialog_width(ctx, 560.0);
                // Text fields share one width so their left edges line up too.
                let field = (width * 0.6).floor();
                dialog_window(ctx, title).open(&mut open).resizable(false).show(ctx, |ui| {
                    ui.set_width(width);
                    dialog_body(ui, opened, |ui| {
                        form::section(ui, "", |s| {
                            s.row(l_find, |ui| ui.add(egui::TextEdit::singleline(query).desired_width(field)));
                            s.row(l_rep, |ui| ui.add(egui::TextEdit::singleline(replacement).desired_width(field)));
                        });
                        let o = &mut self.settings.find_in_files;
                        form::section(ui, l_scope, |s| {
                            s.row(l_dir, |ui| {
                                if form::icon_button(ui, egui::Button::new("…"), l_browse, l_browse).clicked()
                                    && let Some(d) = rfd::FileDialog::new().pick_folder()
                                {
                                    o.directory = d.display().to_string();
                                }
                                ui.add(egui::TextEdit::singleline(&mut o.directory).desired_width(field - ui.min_rect().width() - ui.spacing().item_spacing.x));
                            });
                            s.row(l_types, |ui| ui.add(egui::TextEdit::singleline(&mut o.file_filter).desired_width(field).hint_text(l_types_hint)));
                            s.row(l_ex, |ui| ui.add(egui::TextEdit::singleline(&mut o.exclude_filter).desired_width(field).hint_text(l_ex_hint)));
                            s.toggle(l_sub, &mut o.subdirectories);
                            s.toggle(l_opened, &mut o.opened_files_only);
                        });
                        form::section(ui, l_match, |s| {
                            s.toggle(l_case, &mut self.settings.search_match_case);
                            s.toggle(l_word, &mut self.settings.search_whole_word);
                            s.toggle(l_regex, &mut self.settings.search_regex);
                        });
                        hint(ui, l_warn);
                    });
                    let ready = !self.fif.running && !query.is_empty();
                    footer(ui, |ui| {
                        if ui.add_enabled_ui(ready, |ui| primary_button(ui, l_find_all)).inner.clicked() {
                            start = Some(None);
                        }
                        if ui.add_enabled(ready, egui::Button::new(l_rep_all)).clicked() {
                            start = Some(Some(replacement.clone()));
                        }
                        if ui.button(l_close).clicked() {
                            close = true;
                        }
                    });
                });
                if let Some(rep) = start {
                    self.find.query = query.clone();
                    self.start_find_in_files(query.clone(), rep);
                }
            }
            Dialog::TempMacro { script, error } => {
                let l_hint = self.t("macro.hint", "巨集為 JSON 格式的步驟清單（可由「開始錄製」產生後在此修改）：");
                let (l_run, l_close) = (self.t("macro.run", "執行"), self.t("btn.close", "關閉"));
                let fit = fit_to_window(ctx, egui::vec2(560.0, 300.0), LIST_DIALOG_CHROME_HEIGHT);
                let mut run = false;
                dialog_window(ctx, self.t("dlg.macro", "執行暫存巨集")).open(&mut open).resizable(false).show(ctx, |ui| {
                    ui.set_width(fit.x);
                    hint(ui, l_hint);
                    ui.add_space(4.0);
                    // The editor fills the card, so a click anywhere in it starts typing, and the
                    // card's border shows the focus the frameless editor cannot.
                    let editor_id = ui.make_persistent_id("macro-script");
                    let visuals = ui.visuals();
                    let border = if ui.memory(|m| m.has_focus(editor_id)) { visuals.selection.stroke } else { visuals.widgets.noninteractive.bg_stroke };
                    form::card_with_stroke(ui, egui::Margin::same(4), border, |ui| {
                        egui::ScrollArea::vertical().auto_shrink([false, false]).min_scrolled_height(fit.y).max_height(fit.y).show(ui, |ui| {
                            let editor = egui::TextEdit::multiline(script).id(editor_id).code_editor().frame(egui::Frame::NONE);
                            ui.add_sized([ui.available_width(), fit.y], editor);
                        });
                    });
                    if let Some(e) = error {
                        ui.colored_label(ui.visuals().error_fg_color, e.as_str());
                    }
                    footer(ui, |ui| {
                        run = primary_button(ui, l_run).clicked();
                        if ui.button(l_close).clicked() {
                            close = true;
                        }
                    });
                });
                if run {
                    match super::macros::from_json(script) {
                        Ok(steps) => {
                            self.play_macro(steps);
                            close = true;
                        }
                        Err(e) => *error = Some(e),
                    }
                }
            }
            Dialog::Highlighting => {
                let l_light = self.t("hl.light", "淺色主題：");
                let l_dark = self.t("hl.dark", "深色主題：");
                let l_ui = self.t("opt.theme", "介面主題：");
                let ui_themes = [
                    (ThemeChoice::System, self.t("opt.theme_system", "跟隨系統")),
                    (ThemeChoice::Light, self.t("opt.theme_light", "淺色")),
                    (ThemeChoice::Dark, self.t("opt.theme_dark", "深色")),
                ];
                let l_close = self.t("btn.close", "關閉");
                let themes = syntax::theme_names();
                let mut theme = self.settings.theme;
                let syntax_before = (self.settings.syntax_theme_light.clone(), self.settings.syntax_theme_dark.clone());
                dialog_window(ctx, self.t("dlg.syntax", "語法突顯設定")).open(&mut open).resizable(false).show(ctx, |ui| {
                    ui.set_width(dialog_width(ctx, 440.0));
                    dialog_body(ui, opened, |ui| {
                        form::section(ui, "", |s| {
                            for (label, id, value) in [(l_light, "light-theme", &mut self.settings.syntax_theme_light), (l_dark, "dark-theme", &mut self.settings.syntax_theme_dark)] {
                                s.row(label, |ui| {
                                    egui::ComboBox::from_id_salt(id).selected_text(value.as_str()).width(200.0).show_ui(ui, |ui| {
                                        for t in &themes {
                                            ui.selectable_value(value, t.clone(), t);
                                        }
                                    });
                                });
                            }
                            s.row(l_ui, |ui| form::segmented(ui, &mut theme, &ui_themes));
                        });
                    });
                    footer(ui, |ui| {
                        if primary_button(ui, l_close).clicked() {
                            close = true;
                        }
                    });
                });
                // Saved at once like the UI theme, which `set_theme` saves.
                if theme != self.settings.theme {
                    self.set_theme(ctx, theme);
                } else if (&self.settings.syntax_theme_light, &self.settings.syntax_theme_dark) != (&syntax_before.0, &syntax_before.1) {
                    let _ = self.settings.save();
                }
            }
            Dialog::SourceFormatter => {
                let l_opt = self.t("fmt.options", "AStyle 命令列選項：");
                let l_hint = self.t("fmt.hint", "需要系統已安裝 astyle 執行檔。選項說明請見 https://astyle.sourceforge.net/astyle.html");
                let l_close = self.t("btn.close", "關閉");
                dialog_window(ctx, self.t("dlg.formatter", "原始碼格式化設定（AStyle）")).open(&mut open).resizable(false).show(ctx, |ui| {
                    ui.set_width(dialog_width(ctx, 480.0));
                    dialog_body(ui, opened, |ui| {
                        form::section(ui, l_opt, |s| {
                            s.body(|ui| ui.add(egui::TextEdit::singleline(&mut self.settings.astyle_options).code_editor().desired_width(f32::INFINITY)));
                            s.note(l_hint);
                        });
                    });
                    footer(ui, |ui| {
                        if primary_button(ui, l_close).clicked() {
                            close = true;
                        }
                    });
                });
            }
            Dialog::PurgeHistories => {
                let l_ask = self.t("opt.history_ask", "要清除最近使用的檔案、搜尋與取代紀錄、資料夾紀錄與游標位置紀錄嗎？");
                let l_clear = self.t("opt.clear", "清除");
                let l_done = self.t("msg.history_cleared", "已清除歷史紀錄");
                let l_cancel = self.t("btn.cancel", "取消");
                let mut clear = false;
                dialog_window(ctx, self.t("dlg.history", "清除歷史紀錄")).open(&mut open).resizable(false).show(ctx, |ui| {
                    ui.set_width(dialog_width(ctx, 380.0));
                    dialog_body(ui, opened, |ui| {
                        ui.add(egui::Label::new(l_ask).wrap());
                    });
                    footer(ui, |ui| {
                        if ui.button(l_cancel).clicked() {
                            close = true;
                        }
                        clear = danger_button(ui, l_clear).clicked();
                    });
                });
                if clear {
                    self.settings.purge_history();
                    self.set_status(l_done);
                    close = true;
                }
            }
            Dialog::ConvertEncoding { encoding, bom } => {
                let current = self.doc().encoding.name().to_string();
                // The "{q}" template without its value is the row label.
                let l_current = self.tf("enc.current", "目前編碼：{q}", None, Some(""));
                let l_to = self.t("enc.to", "轉換為：");
                let l_bom = self.t("opt.add_bom", "加入 BOM");
                let l_hint = self.t("enc.hint", "轉換編碼會保留文字內容，存檔時改用新編碼寫出。若只是開檔時編碼判斷錯誤，請改用「檢視 → 編碼」重新解碼。");
                let l_convert = self.t("enc.convert", "轉換");
                let l_need = self.t("enc.need_text", "請先切換到文字模式").to_string();
                let l_fail = self.t("enc.fail", "無法轉換編碼").to_string();
                let l_cancel = self.t("btn.cancel", "取消");
                let mut convert = false;
                dialog_window(ctx, self.t("dlg.encoding", "轉換檔案編碼")).open(&mut open).resizable(false).show(ctx, |ui| {
                    ui.set_width(dialog_width(ctx, 440.0));
                    dialog_body(ui, opened, |ui| {
                        form::section(ui, "", |s| {
                            s.row(&l_current, |ui| ui.label(RichText::new(&current).weak()));
                            s.row(l_to, |ui| encoding_combo(ui, "conv-enc", encoding));
                            if encoding::encoding_for_name(encoding).is_some_and(encoding::supports_bom) {
                                s.toggle(l_bom, bom);
                            }
                        });
                        hint(ui, l_hint);
                    });
                    footer(ui, |ui| {
                        convert = primary_button(ui, l_convert).clicked();
                        if ui.button(l_cancel).clicked() {
                            close = true;
                        }
                    });
                });
                if convert {
                    if let Some(enc) = encoding::encoding_for_name(encoding) {
                        let r = if self.doc().mode == EditMode::Hex { Err(l_need) } else { self.doc_mut().convert_encoding(enc, *bom) };
                        match r {
                            Ok(()) => {
                                let name = enc.name().to_string();
                                self.set_status(self.tf("enc.done", "已轉換為 {q}，存檔時生效", None, Some(&name)));
                            }
                            Err(e) => self.message(&l_fail, e),
                        }
                    }
                    close = true;
                }
            }
            Dialog::WordCount(wc, sel) => {
                let scope = if *sel { self.t("wc.selection", "（選取範圍）") } else { self.t("wc.document", "（整份文件）") };
                let scope = scope.trim_matches(['（', '）', '(', ')', ' ']);
                let rows = [
                    (self.t("wc.words", "字數"), wc.words),
                    (self.t("wc.chars", "字元數（不含換行）"), wc.chars),
                    (self.t("wc.spaces", "空白字元"), wc.spaces),
                    (self.t("wc.half", "半形字元"), wc.halfwidth),
                    (self.t("wc.full", "全形字元"), wc.fullwidth),
                    (self.t("wc.cjk", "中日韓文字"), wc.cjk),
                    (self.t("wc.lines", "行數"), wc.lines),
                ];
                let l_close = self.t("btn.close", "關閉");
                dialog_window(ctx, self.t("dlg.words", "字數統計")).open(&mut open).resizable(false).show(ctx, |ui| {
                    ui.set_width(dialog_width(ctx, 360.0));
                    dialog_body(ui, opened, |ui| {
                        form::section(ui, scope, |s| {
                            for (label, value) in rows {
                                s.row(label, |ui| ui.label(RichText::new(value.to_string()).monospace().strong()));
                            }
                        });
                    });
                    footer(ui, |ui| {
                        if primary_button(ui, l_close).clicked() {
                            close = true;
                        }
                    });
                });
            }
            Dialog::WindowList => {
                let mut go = None;
                let mut close_ids = Vec::new();
                let l_close_one = self.t("btn.close", "關閉");
                let l_close_list = self.t("btn.close_list", "關閉清單");
                let fit = fit_to_window(ctx, egui::vec2(520.0, 300.0), LIST_DIALOG_CHROME_HEIGHT);
                dialog_window(ctx, self.t("dlg.windows", "視窗清單")).open(&mut open).resizable(false).show(ctx, |ui| {
                    ui.set_width(fit.x);
                    list_card(ui, "windows", fit.y, |ui| {
                        for (i, t) in self.tabs.iter().enumerate() {
                            let size = egui::vec2(ui.available_width(), 28.0);
                            ui.allocate_ui_with_layout(size, egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                // Named like the tab bar's close buttons; the file name tells the rows apart.
                                let close_name = format!("{l_close_one} {}", t.doc.display_name());
                                if form::icon_button(ui, egui::Button::new("×").small(), l_close_one, &close_name).clicked() {
                                    close_ids.push(t.doc.id);
                                }
                                let name = format!("{}{}", t.doc.display_name(), if t.doc.is_modified() { " *" } else { "" });
                                let folder = t.doc.path.as_ref().and_then(|p| p.parent()).map(tilde_path).unwrap_or_default();
                                if form::list_row(ui, egui::Role::Button, i == self.active, &name, &folder).clicked() {
                                    go = Some(i);
                                }
                            });
                        }
                    });
                    footer(ui, |ui| {
                        if primary_button(ui, l_close_list).clicked() {
                            close = true;
                        }
                    });
                });
                if let Some(i) = go {
                    self.activate(i);
                }
                if !close_ids.is_empty() {
                    self.request_close(close_ids, false);
                }
            }
            Dialog::About { copied_at } => {
                let version = env!("CARGO_PKG_VERSION");
                let repo = env!("CARGO_PKG_REPOSITORY");
                let platform = format!("{} / {}", std::env::consts::OS, std::env::consts::ARCH);
                let settings_path = Settings::path().map(|p| tilde_path(&p)).unwrap_or_else(|| "—".into());
                let now = ctx.input(|i| i.time);
                let copied = copied_at.is_some_and(|t| now - t < COPIED_SECONDS);
                if let Some(t) = *copied_at
                    && copied
                {
                    ctx.request_repaint_after(std::time::Duration::from_secs_f64(COPIED_SECONDS - (now - t)));
                }
                let l_version = self.tf("about.version", "版本 {q}", None, Some(version));
                let l_tag = self.t("about.tagline", "以 Rust 與 egui 重製的文字／十六進位編輯器。");
                let l_platform = self.t("about.platform", "平台");
                let l_built = self.t("about.built_with", "使用技術");
                let l_settings = self.t("about.settings", "設定檔");
                let l_project = self.t("about.project", "專案網頁");
                let l_credit = self.t("about.credit", "原版 MadEdit 由 Alston Chen 開發，MadEdit-Mod 由 Minggang Li 維護（GPL）。");
                let l_assets = self.t("about.assets", "工具列圖示、簡繁對照表與拼字字典取自原版。");
                let l_copy = if copied { self.t("about.copied", "已複製") } else { self.t("about.copy_info", "複製資訊") };
                let l_keys = plain_label(&self.cmd_text(C::KeyboardShortcuts));
                let l_ok = self.t("btn.ok", "確定");
                // Plain-text summary for bug reports.
                let info = format!("MadEdit {version}\n{platform}\n{settings_path}");
                let icon = self.icons.get("app").map(|(t, _)| t.id());
                dialog_window(ctx, self.t("about.title", "關於 MadEdit")).open(&mut open).resizable(false).show(ctx, |ui| {
                    ui.set_width(dialog_width(ctx, 440.0));
                    dialog_body(ui, opened, |ui| {
                        ui.horizontal(|ui| {
                            if let Some(id) = icon {
                                ui.add(egui::Image::new((id, egui::vec2(56.0, 56.0))));
                            }
                            ui.vertical(|ui| {
                                ui.label(RichText::new("MadEdit").size(24.0).strong());
                                ui.label(RichText::new(&l_version).weak());
                                ui.label(l_tag);
                            });
                        });
                        ui.add_space(8.0);
                        form::card(ui, egui::Margin::symmetric(12, 8), |ui| egui::Grid::new("about-info").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
                            ui.label(RichText::new(l_platform).weak());
                            ui.label(&platform);
                            ui.end_row();
                            ui.label(RichText::new(l_built).weak());
                            ui.label("Rust · egui / eframe · syntect · encoding_rs");
                            ui.end_row();
                            ui.label(RichText::new(l_settings).weak());
                            ui.add(egui::Label::new(RichText::new(&settings_path).small()).wrap());
                            ui.end_row();
                            ui.label(RichText::new(l_project).weak());
                            ui.hyperlink_to(repo.trim_start_matches("https://"), repo);
                            ui.end_row();
                        }));
                        ui.add_space(8.0);
                        hint(ui, l_credit);
                        hint(ui, l_assets);
                    });
                    footer(ui, |ui| {
                        if primary_button(ui, l_ok).clicked() {
                            close = true;
                        }
                        // Secondary actions start at the left edge.
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            if ui.button(l_copy).clicked() {
                                ui.ctx().copy_text(info.clone());
                                *copied_at = Some(now);
                            }
                            if ui.button(&l_keys).clicked() {
                                switch_to = Some(Dialog::Shortcuts { filter: String::new() });
                            }
                        });
                    });
                });
            }
            Dialog::Shortcuts { filter } => {
                let title = plain_label(&self.cmd_text(C::KeyboardShortcuts));
                let filter_hint = form::bare(self.t("opt.filter", "篩選：")).to_string();
                let l_none = self.t("keys.none", "沒有符合的快捷鍵");
                let l_customize = self.t("keys.customize", "自訂快捷鍵…");
                let l_close = self.t("btn.close", "關閉");
                let titles: Vec<String> = COMMAND_SECTIONS.iter().map(|&(_, key, zh)| plain_label(self.t(key, zh))).collect();
                // Each menu with its (command name, keys one per line) rows, in menu order. The
                // filter also matches the spelled-out key names, so "ctrl" or "cmd" finds "⌘F".
                let f = filter.to_lowercase();
                let mut sections: Vec<(&str, Vec<(String, String)>)> = titles.iter().map(|t| (t.as_str(), Vec::new())).collect();
                for (&c, s) in commands::ALL.iter().zip(command_sections()) {
                    let bound: Vec<&egui::KeyboardShortcut> = self.bindings.iter().filter(|(_, b)| *b == c).map(|(sc, _)| sc).collect();
                    if bound.is_empty() {
                        continue;
                    }
                    let name = plain_label(&self.cmd_text(c));
                    let keys: Vec<String> = bound.iter().map(|sc| ctx.format_shortcut(sc)).collect();
                    let hit = f.is_empty()
                        || name.to_lowercase().contains(&f)
                        || bound.iter().zip(&keys).any(|(sc, shown)| {
                            let names = egui::ModifierNames::NAMES;
                            [shown.clone(), sc.format(&names, true), sc.format(&names, false)].iter().any(|k| k.to_lowercase().contains(&f))
                        });
                    if hit {
                        sections[s].1.push((name, keys.join("\n")));
                    }
                }
                sections.retain(|(_, rows)| !rows.is_empty());
                // Height in lines: a command with several shortcuts takes one line per key, and
                // a menu title about two.
                let heights: Vec<usize> = sections.iter().map(|(_, rows)| rows.iter().map(|(_, keys)| keys.lines().count()).sum::<usize>() + 2).collect();
                let size = fit_to_window(ctx, SHORTCUTS_SIZE, SHORTCUTS_CHROME_HEIGHT);
                let two_columns = size.x >= SHORTCUTS_TWO_COLUMN_WIDTH;
                let split = if two_columns { column_split(&heights) } else { sections.len() };
                let (left, right) = sections.split_at(split);
                let show_column = |ui: &mut egui::Ui, column: &[(&str, Vec<(String, String)>)]| {
                    let name_width = (ui.available_width() * 0.58).floor();
                    for (title, rows) in column {
                        ui.add_space(4.0);
                        ui.label(RichText::new(*title).strong());
                        egui::Grid::new(("keys", *title)).num_columns(2).striped(true).min_col_width(ui.available_width() - name_width - 8.0).show(ui, |ui| {
                            for (cmd, keys) in rows {
                                ui.scope(|ui| {
                                    ui.set_width(name_width);
                                    ui.add(egui::Label::new(cmd.as_str()).wrap());
                                });
                                ui.label(RichText::new(keys.as_str()).monospace());
                                ui.end_row();
                            }
                        });
                    }
                };
                dialog_window(ctx, title).open(&mut open).resizable(false).show(ctx, |ui| {
                    ui.set_width(size.x);
                    ui.add(egui::TextEdit::singleline(filter).hint_text(filter_hint).desired_width(f32::INFINITY));
                    ui.add_space(6.0);
                    // Fixed height so filtering does not resize the window; `min_scrolled_height`
                    // keeps the area from eating the footer's room on the window's sizing pass.
                    egui::ScrollArea::vertical().auto_shrink([false, false]).min_scrolled_height(size.y).max_height(size.y).show(ui, |ui| {
                        if sections.is_empty() {
                            ui.label(RichText::new(l_none).weak());
                        }
                        if two_columns {
                            ui.columns(2, |cols| {
                                show_column(&mut cols[0], left);
                                show_column(&mut cols[1], right);
                            });
                        } else {
                            show_column(ui, left);
                        }
                    });
                    footer(ui, |ui| {
                        if primary_button(ui, l_close).clicked() {
                            close = true;
                        }
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            if ui.button(l_customize).clicked() {
                                let options = OptionsState { tab: OPTIONS_KEYS_TAB, ..Default::default() };
                                switch_to = Some(Dialog::Options(Box::new(options)));
                            }
                        });
                    });
                });
            }
        }
        if let Some(next) = switch_to {
            self.dialog = Some(next);
        } else if open && !close {
            self.dialog = Some(dlg);
        }
        // The kind drawn this frame, not a replacement set meanwhile (About handing over to
        // Keyboard Shortcuts), so the replacement counts as opened on its first frame.
        self.shown_dialog = self.dialog.as_ref().map(|_| kind);
    }

    /// 插入遞增數字：列塊選取時每行一個，否則自游標處起逐行插入
    fn insert_numbers(&mut self) {
        let opts = self.settings.numbers.clone();
        let doc = self.doc_mut();
        let sel = doc.editor.selection;
        if sel.is_block() {
            let b = sel.visual_bounds(&doc.editor);
            let nums = textops::incremental_numbers(b.max_line - b.min_line + 1, &opts);
            let hint = doc.selection_hint();
            doc.edit(EditKind::Other, hint, |ed| {
                ed.delete_selection();
                for (k, l) in (b.min_line..=b.max_line).enumerate() {
                    let line_v = crate::core::editor::char_col_to_visual_col(&ed.lines[l], ed.line_char_count(l));
                    if line_v < b.min_vcol {
                        ed.lines[l].push_str(&" ".repeat(b.min_vcol - line_v));
                    }
                    let col = crate::core::editor::visual_col_to_char_col(&ed.lines[l], b.min_vcol);
                    let byte = crate::core::editor::char_to_byte(&ed.lines[l], col);
                    ed.lines[l].insert_str(byte, &nums[k]);
                }
                ed.clamp_caret();
            });
        } else {
            let nums = textops::incremental_numbers(opts.total.max(1), &opts);
            let head = sel.head;
            let hint = head.line..doc.editor.lines.len();
            doc.edit(EditKind::Other, hint, |ed| {
                let vcol = crate::core::editor::char_col_to_visual_col(&ed.lines[head.line], head.col);
                for (k, n) in nums.iter().enumerate() {
                    let l = head.line + k;
                    if l >= ed.lines.len() {
                        ed.lines.push(String::new());
                    }
                    let line_v = crate::core::editor::char_col_to_visual_col(&ed.lines[l], ed.line_char_count(l));
                    if line_v < vcol {
                        ed.lines[l].push_str(&" ".repeat(vcol - line_v));
                    }
                    let col = crate::core::editor::visual_col_to_char_col(&ed.lines[l], vcol);
                    let byte = crate::core::editor::char_to_byte(&ed.lines[l], col);
                    ed.lines[l].insert_str(byte, n);
                }
                ed.clamp_caret();
            });
        }
    }

    fn options_dialog(&mut self, ctx: &egui::Context, st: &mut OptionsState, open: &mut bool) {
        if st.draft.is_none() {
            st.base = Some(self.settings.clone());
        }
        let draft = st.draft.get_or_insert_with(|| self.settings.clone());
        if draft.theme == st.live_theme {
            draft.theme = self.settings.theme;
        }
        if draft.ui_language == st.live_language {
            draft.ui_language = self.settings.ui_language;
        }
        st.live_theme = self.settings.theme;
        st.live_language = self.settings.ui_language;
        let mut apply = false;
        let mut ok = false;
        let mut cancel = false;
        // 擷取快捷鍵
        if let Some(c) = st.capturing {
            let pressed = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Key { key, pressed: true, modifiers, .. } if !matches!(key, Key::Escape) => Some(egui::KeyboardShortcut::new(*modifiers, *key)),
                    _ => None,
                })
            });
            if ctx.input(|i| i.key_pressed(Key::Escape)) {
                st.capturing = None;
            } else if let Some(mut sc) = pressed.filter(commands::is_bindable) {
                // 只保留邏輯修飾鍵（macOS 的 ⌘ 視為 Ctrl）
                sc.modifiers = egui::Modifiers { mac_cmd: false, ..sc.modifiers };
                let text = commands::shortcut_to_string(&sc);
                let list = draft.key_bindings.entry(c.id().to_string()).or_insert_with(|| c.default_keys().split('|').filter(|k| !k.is_empty()).map(str::to_string).collect());
                if !list.contains(&text) {
                    list.push(text);
                }
                st.capturing = None;
            }
        }
        let lang = self.ui_lang();
        let tabs = [
            self.t("opt.general", "一般"),
            self.t("opt.edit", "編輯"),
            self.t("opt.new", "新文件"),
            self.t("opt.print", "列印"),
            self.t("opt.keys", "按鍵"),
            self.t("opt.spell", "拼字檢查"),
        ];
        let l_open = self.t("opt.open_mode", "開檔模式：");
        let l_auto = self.t("opt.open_auto", "自動（二進位檔用十六進位）");
        let l_text = self.t("opt.open_text", "文字模式");
        let l_column = self.t("opt.open_column", "列塊模式");
        let l_hex = self.t("opt.open_hex", "十六進位模式");
        // "every {n} minutes" around the auto-save interval field, in the language's own order.
        let (every_before, every_after) = self.t("opt.autosave_interval", "每 {n} 分鐘").split_once("{n}").unwrap_or(("", ""));
        let l_remove = self.t("btn.remove", "移除");
        let font_auto = self.tf("opt.font_auto", "自動（{q}）", None, Some(&self.fonts.editor_family));
        // Fixed size: the window used to shrink and grow with each page's content.
        dialog_window(ctx, self.t("dlg.options", "選項")).open(open).resizable(false).show(ctx, |ui| {
            let size = fit_to_window(ui.ctx(), OPTIONS_SIZE, OPTIONS_CHROME_HEIGHT);
            ui.set_width(size.x);
            let nav_width = (size.x * 0.22).clamp(90.0, 150.0);
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(egui::vec2(nav_width, size.y), egui::Layout::top_down(egui::Align::LEFT), |ui| {
                    ui.set_min_size(egui::vec2(nav_width, size.y));
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for (i, name) in tabs.iter().enumerate() {
                        if form::nav_item(ui, st.tab == i, name).clicked() && st.tab != i {
                            st.tab = i;
                            // A pending key capture would silently bind the next shortcut.
                            st.capturing = None;
                        }
                    }
                });
                // Sized separator: a plain one would take all the height on offer and the
                // auto-sized window would grow every frame.
                ui.add_sized([8.0, size.y], egui::Separator::default().vertical());
                let area = egui::ScrollArea::vertical().auto_shrink([false, false]).min_scrolled_height(size.y).max_height(size.y);
                // Right margin keeps the cards clear of the floating scroll bar.
                let page_margin = egui::Margin { left: 4, right: 12, top: 0, bottom: 0 };
                ui.vertical(|ui| area.show(ui, |ui| egui::Frame::new().inner_margin(page_margin).show(ui, |ui| match st.tab {
                    0 => {
                        form::section(ui, self.t("opt.sec.appearance", "外觀"), |s| {
                            s.row(self.t("opt.ui_language", "介面語言："), |ui| {
                                let current = i18n::language_choice_label(lang, draft.ui_language);
                                egui::ComboBox::from_id_salt("ui-lang").selected_text(current).width(200.0).show_ui(ui, |ui| {
                                    for choice in UiLanguage::ALL {
                                        ui.selectable_value(&mut draft.ui_language, choice, i18n::language_choice_label(lang, choice));
                                    }
                                });
                            });
                            let themes = [
                                (ThemeChoice::System, self.t("opt.theme_system", "跟隨系統")),
                                (ThemeChoice::Light, self.t("opt.theme_light", "淺色")),
                                (ThemeChoice::Dark, self.t("opt.theme_dark", "深色")),
                            ];
                            s.row(self.t("opt.theme", "介面主題："), |ui| form::segmented(ui, &mut draft.theme, &themes));
                            let font = if draft.font_family.is_empty() { font_auto.as_str() } else { draft.font_family.as_str() };
                            s.row(self.t("opt.editor_font", "編輯區字型："), |ui| ui.label(RichText::new(font).weak()));
                        });
                        form::section(ui, self.t("opt.sec.open", "開啟檔案"), |s| {
                            s.row(self.t("opt.default_enc", "預設編碼："), |ui| encoding_combo(ui, "def-enc", &mut draft.default_encoding));
                            s.row(self.t("opt.fallback_enc", "備援編碼（非 UTF-8 時）："), |ui| encoding_combo(ui, "fallback-enc", &mut draft.fallback_encoding));
                            s.toggle(self.t("opt.enforce_enc", "一律使用預設編碼開檔（不自動偵測）"), &mut draft.enforce_default_encoding);
                            s.row(l_open, |ui| {
                                let open_label = match draft.open_view_mode {
                                    OpenViewMode::Auto => l_auto,
                                    OpenViewMode::Text => l_text,
                                    OpenViewMode::Column => l_column,
                                    OpenViewMode::Hex => l_hex,
                                };
                                egui::ComboBox::from_id_salt("open-mode").selected_text(open_label).width(200.0).show_ui(ui, |ui| {
                                    ui.selectable_value(&mut draft.open_view_mode, OpenViewMode::Auto, l_auto);
                                    ui.selectable_value(&mut draft.open_view_mode, OpenViewMode::Text, l_text);
                                    ui.selectable_value(&mut draft.open_view_mode, OpenViewMode::Column, l_column);
                                    ui.selectable_value(&mut draft.open_view_mode, OpenViewMode::Hex, l_hex);
                                });
                            });
                            s.row(self.t("opt.max_mb", "文字模式檔案大小上限（MB）："), |ui| ui.add(egui::DragValue::new(&mut draft.max_text_file_mb).range(1..=4096)));
                        });
                        form::section(ui, self.t("opt.sec.session", "工作階段與存檔"), |s| {
                            s.toggle(self.t("opt.reload_startup", "啟動時重新開啟上次的檔案"), &mut draft.reload_files_on_startup);
                            s.toggle(self.t("opt.restore_caret", "開檔時還原上次的游標位置"), &mut draft.restore_caret_position);
                            s.toggle_with(self.t("opt.autosave_label", "自動儲存"), &mut draft.auto_save, |ui| {
                                ui.add(egui::DragValue::new(&mut draft.auto_save_minutes).range(1..=120).prefix(every_before).suffix(every_after));
                            });
                            s.toggle(self.t("opt.backup", "存檔時保留備份（檔名.bak）"), &mut draft.auto_backup);
                            s.toggle(self.t("opt.purge_exit", "結束時清除歷史紀錄"), &mut draft.purge_history_on_exit);
                        });
                        form::section(ui, self.t("opt.sec.search", "搜尋"), |s| {
                            s.row(self.t("opt.search_chars", "搜尋結果每行最多字元："), |ui| ui.add(egui::DragValue::new(&mut draft.max_search_result_chars).range(20..=1000)));
                        });
                    }
                    1 => {
                        form::section(ui, self.t("opt.sec.layout", "排版與縮排"), |s| {
                            s.row(self.t("opt.tab_size", "Tab 欄數："), |ui| ui.add(egui::DragValue::new(&mut draft.tab_size).range(1..=16)));
                            s.row(self.t("opt.indent", "縮排欄數："), |ui| ui.add(egui::DragValue::new(&mut draft.indent_size).range(1..=16)));
                            s.toggle(self.t("opt.spaces_for_tab", "以空白取代 Tab"), &mut draft.insert_spaces_for_tab);
                            s.toggle(self.t("opt.auto_indent", "自動縮排"), &mut draft.auto_indent);
                            s.row(self.t("opt.wrap_col", "依欄數換行的欄數："), |ui| ui.add(egui::DragValue::new(&mut draft.wrap_column).range(10..=1000)));
                            s.row(self.t("opt.guide", "欄位提示線位置："), |ui| ui.add(egui::DragValue::new(&mut draft.col_hint_position).range(1..=1000)));
                            s.row(self.t("opt.spacing", "行距（%）："), |ui| ui.add(egui::DragValue::new(&mut draft.line_spacing).range(100..=250)));
                            s.toggle(self.t("opt.fixed", "等寬模式"), &mut draft.fixed_width);
                            s.toggle(self.t("opt.typewriter", "打字機模式（游標保持在畫面中央）"), &mut draft.typewriter_mode);
                        });
                        form::section(ui, self.t("opt.sec.typing", "輸入"), |s| {
                            s.toggle(self.t("opt.auto_pair", "自動補全成對字元（括號、引號）"), &mut draft.auto_complete_pair);
                            s.toggle(self.t("opt.wrap_pair", "有選取時以成對字元包住選取內容"), &mut draft.wrap_selection_with_pair);
                            s.toggle(self.t("opt.fill_paste", "列塊貼上時以空白補齊短行"), &mut draft.auto_fill_column_paste);
                            s.row(self.t("opt.datetime", "日期時間格式："), |ui| ui.add(egui::TextEdit::singleline(&mut draft.date_time_format).desired_width(200.0)));
                            let preview = chrono::Local::now().format(&draft.date_time_format).to_string();
                            s.note(&self.tf("opt.date_preview", "日期預覽：{q}", None, Some(&preview)));
                        });
                        form::section(ui, self.t("opt.sec.mouse", "滑鼠"), |s| {
                            s.toggle(self.t("opt.auto_copy", "滑鼠選取後自動複製"), &mut draft.auto_copy_mouse_selection);
                            s.toggle(self.t("opt.middle_paste", "滑鼠中鍵貼上"), &mut draft.middle_click_paste);
                            s.toggle(self.t("opt.double_click", "雙擊時標示相同字詞"), &mut draft.double_click_highlight);
                        });
                    }
                    2 => {
                        form::section(ui, "", |s| {
                            s.row(self.t("opt.encoding", "編碼："), |ui| encoding_combo(ui, "new-enc", &mut draft.new_doc_encoding));
                            s.toggle(self.t("opt.add_bom", "加入 BOM"), &mut draft.new_doc_bom);
                            let newlines = ["CRLF", "LF", "CR"].map(|n| (n.to_string(), n));
                            s.row(self.t("opt.newline", "換行字元："), |ui| form::segmented(ui, &mut draft.new_doc_newline, &newlines));
                            s.row(self.t("opt.syntax", "語法："), |ui| {
                                egui::ComboBox::from_id_salt("new-syntax").selected_text(draft.new_doc_syntax.as_str()).width(200.0).show_ui(ui, |ui| {
                                    egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                                        for s in syntax::syntax_names() {
                                            ui.selectable_value(&mut draft.new_doc_syntax, s.clone(), s);
                                        }
                                    });
                                });
                            });
                        });
                    }
                    3 => print_options_ui(ui, &mut draft.print, lang),
                    4 => {
                        let hint = form::bare(self.t("opt.filter", "篩選：")).to_string();
                        ui.add(egui::TextEdit::singleline(&mut st.filter).hint_text(hint).desired_width(f32::INFINITY));
                        ui.add_space(8.0);
                        let list_height = (size.y - 44.0).max(80.0);
                        let list_width = (ui.available_width() * 0.58).floor();
                        let column = egui::Layout::top_down(egui::Align::LEFT);
                        ui.horizontal_top(|ui| {
                            ui.allocate_ui_with_layout(egui::vec2(list_width, list_height), column, |ui| {
                                list_card(ui, "cmd-list", list_height - 8.0, |ui| {
                                    let f = st.filter.to_lowercase();
                                    for &c in commands::ALL {
                                        let label = self.cmd_text(c);
                                        if f.is_empty() || label.to_lowercase().contains(&f) || c.id().to_lowercase().contains(&f) {
                                            let keys = draft.key_bindings.get(c.id()).map(|v| v.join(", ")).unwrap_or_else(|| c.default_keys().replace('|', ", "));
                                            if form::list_row(ui, egui::Role::Button, st.selected == Some(c), &plain_label(&label), &keys).clicked() && st.selected != Some(c) {
                                                st.selected = Some(c);
                                                st.capturing = None;
                                            }
                                        }
                                    }
                                });
                            });
                            ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), list_height), column, |ui| {
                                form::card(ui, egui::Margin::same(12), |ui| {
                                    ui.set_min_height(list_height - 24.0);
                                    let Some(c) = st.selected else {
                                        ui.label(RichText::new(self.t("opt.pick_command", "請在左側選擇命令")).weak());
                                        return;
                                    };
                                    ui.label(RichText::new(plain_label(&self.cmd_text(c))).strong().size(15.0));
                                    ui.label(RichText::new(c.id()).small().weak());
                                    ui.add_space(8.0);
                                    let list: Vec<String> = draft
                                        .key_bindings
                                        .get(c.id())
                                        .cloned()
                                        .unwrap_or_else(|| c.default_keys().split('|').filter(|k| !k.is_empty()).map(str::to_string).collect());
                                    if list.is_empty() {
                                        ui.label(RichText::new(self.t("opt.no_keys", "沒有快捷鍵")).weak());
                                    }
                                    let mut remove = None;
                                    for (i, k) in list.iter().enumerate() {
                                        ui.horizontal(|ui| {
                                            egui::Frame::new().fill(ui.visuals().extreme_bg_color).corner_radius(4.0).inner_margin(egui::Margin::symmetric(6, 2)).show(ui, |ui| ui.monospace(k));
                                            if form::icon_button(ui, egui::Button::new("×").small(), l_remove, &format!("{l_remove} {k}")).clicked() {
                                                remove = Some(i);
                                            }
                                        });
                                    }
                                    if let Some(i) = remove {
                                        let mut l = list.clone();
                                        l.remove(i);
                                        draft.key_bindings.insert(c.id().to_string(), l);
                                    }
                                    ui.add_space(8.0);
                                    if st.capturing == Some(c) {
                                        ui.add(egui::Label::new(RichText::new(self.t("opt.press_key", "請按下新的快捷鍵（需含 Ctrl／⌘／Alt，F 鍵除外；Esc 取消）…")).color(ui.visuals().warn_fg_color)).wrap());
                                    } else {
                                        ui.horizontal_wrapped(|ui| {
                                            if ui.button(self.t("btn.add_shortcut", "新增快捷鍵…")).clicked() {
                                                st.capturing = Some(c);
                                            }
                                            if ui.button(self.t("btn.restore", "還原預設")).clicked() {
                                                draft.key_bindings.remove(c.id());
                                            }
                                        });
                                    }
                                    // 衝突提示。{n} 是另一個命令的名稱，不是數字
                                    for k in &list {
                                        for &other in commands::ALL {
                                            if other == c {
                                                continue;
                                            }
                                            let okeys: Vec<String> = draft.key_bindings.get(other.id()).cloned().unwrap_or_else(|| other.default_keys().split('|').map(str::to_string).collect());
                                            if okeys.iter().any(|o| o == k) {
                                                let name = self.cmd_text(other);
                                                let msg = self.t("opt.conflict", "{q} 也指派給「{n}」").replace("{q}", k).replace("{n}", &name);
                                                ui.add(egui::Label::new(RichText::new(msg).color(ui.visuals().error_fg_color)).wrap());
                                            }
                                        }
                                    }
                                });
                            });
                        });
                    }
                    _ => {
                        form::section(ui, self.t("opt.sec.dictionary", "字典"), |s| {
                            s.row(self.t("opt.spell_lang", "字典語言："), |ui| {
                                egui::ComboBox::from_id_salt("spell-lang").selected_text(draft.spell_language.as_str()).width(200.0).show_ui(ui, |ui| {
                                    for l in spell::available_languages() {
                                        ui.selectable_value(&mut draft.spell_language, l.clone(), l);
                                    }
                                });
                            });
                            if let Some(e) = &self.spell_error {
                                s.body(|ui| ui.add(egui::Label::new(RichText::new(e).color(ui.visuals().error_fg_color)).wrap()));
                            }
                            if let Some(dir) = spell::dictionary_dirs().first() {
                                let path = tilde_path(dir);
                                s.note(&self.tf("opt.dict_dir", "可將其他語言的 Hunspell 字典（.aff 與 .dic）放到：{q}", None, Some(&path)));
                            }
                        });
                        let mut remove = None;
                        form::section(ui, self.t("opt.personal", "個人字典："), |s| {
                            if draft.personal_dictionary.is_empty() {
                                s.body(|ui| ui.label(RichText::new(self.t("opt.personal_empty", "尚未加入任何單字")).weak()));
                            }
                            for (i, w) in draft.personal_dictionary.iter().enumerate() {
                                s.row(w, |ui| {
                                    if form::icon_button(ui, egui::Button::new("×").small(), l_remove, &format!("{l_remove} {w}")).clicked() {
                                        remove = Some(i);
                                    }
                                });
                            }
                        });
                        if let Some(i) = remove {
                            draft.personal_dictionary.remove(i);
                        }
                    }
                })));
            });
            footer(ui, |ui| {
                ok = primary_button(ui, self.t("btn.ok", "確定")).clicked();
                cancel = ui.button(self.t("btn.cancel", "取消")).clicked();
                apply = ui.button(self.t("btn.apply", "套用")).clicked();
            });
        });
        if ok || apply {
            let (Some(base), Some(draft)) = (&st.base, &st.draft) else { return };
            let new = self.settings.with_edits(base, draft);
            let spell_changed = new.spell_language != self.settings.spell_language || new.personal_dictionary != self.settings.personal_dictionary;
            let theme_changed = new.theme != self.settings.theme;
            self.settings = new;
            self.rebuild_bindings();
            if theme_changed {
                self.apply_theme(ctx);
            }
            if spell_changed && self.settings.spell_check {
                self.load_spell();
            }
            let _ = self.settings.save();
            // Later edits are measured from what was just applied.
            st.base = Some(self.settings.clone());
            st.draft = Some(self.settings.clone());
        }
        if ok || cancel {
            *open = false;
        }
    }

    pub fn show_confirm(&mut self, ctx: &egui::Context) {
        let Some(confirm) = self.confirm.take() else {
            self.shown_confirm = None;
            return;
        };
        let kind = std::mem::discriminant(&confirm);
        let opened = self.shown_confirm != Some(kind);
        let mut keep = true;
        let modal = egui::Modal::new(egui::Id::unique("confirm"));
        match &confirm {
            Confirm::Message { title, text } => {
                modal.show(ctx, |ui| {
                    confirm_body(ui, opened, title, &[text]);
                    // Keys are ignored on the first frame: the Enter that opened a missing recent
                    // file would otherwise dismiss its error before it is ever drawn.
                    let key = !opened && ui.input(|i| i.key_pressed(Key::Enter) || i.key_pressed(Key::Escape));
                    footer(ui, |ui| {
                        if primary_button(ui, self.t("btn.ok", "確定")).clicked() || key {
                            keep = false;
                        }
                    });
                });
            }
            Confirm::ProportionalFont => {
                let mut choice = None;
                modal.show(ctx, |ui| {
                    let body = self.t("confirm.proportional_body", "目前字型為比例字型，建議關閉等寬模式。等寬模式下，m、W 等較寬的字母會與相鄰的字重疊。");
                    confirm_body(ui, opened, self.t("confirm.proportional_title", "比例字型"), &[body]);
                    footer(ui, |ui| {
                        if primary_button(ui, self.t("btn.fixed_off", "關閉等寬模式")).clicked() {
                            choice = Some(true);
                        }
                        if ui.button(self.t("btn.keep_fixed", "保持等寬模式")).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                            choice = Some(false);
                        }
                    });
                });
                if let Some(off) = choice {
                    keep = false;
                    if off {
                        self.settings.fixed_width = false;
                        self.save_settings();
                    }
                }
            }
            Confirm::Close { ids, exit } => {
                let id = ids[0];
                let Some(idx) = self.index_of(id) else {
                    let rest: Vec<u64> = ids[1..].to_vec();
                    self.confirm = if rest.is_empty() { None } else { Some(Confirm::Close { ids: rest, exit: *exit }) };
                    if rest_empty_and_exit(&self.confirm, *exit) {
                        self.allow_close = true;
                    }
                    return;
                };
                let name = self.tabs[idx].doc.display_name();
                let mut choice = None;
                let ask = self.tf("confirm.save_ask", "「{q}」已修改，要儲存嗎？", None, Some(&name));
                modal.show(ctx, |ui| {
                    confirm_body(ui, opened, self.t("confirm.unsaved", "尚未儲存"), &[&ask]);
                    footer(ui, |ui| {
                        if primary_button(ui, self.t("btn.save", "儲存")).clicked() {
                            choice = Some(0);
                        }
                        if ui.button(self.t("btn.cancel", "取消")).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                            choice = Some(2);
                        }
                        if danger_button(ui, self.t("btn.dont_save", "不儲存")).clicked() {
                            choice = Some(1);
                        }
                    });
                });
                match choice {
                    Some(0) => {
                        self.activate(idx);
                        self.active = idx;
                        if self.save_tab(idx, false) {
                            if !*exit { self.close_now(id); }
                            self.continue_close(ids, *exit);
                        } else if self.confirm.is_some() {
                            // 存檔需要再確認：記下關閉流程，確認後存檔成功再接續關閉
                            self.pending_close = Some((ids.clone(), *exit));
                        }
                        // 否則為使用者取消另存對話框：中止關閉
                        return;
                    }
                    Some(1) => {
                        if !*exit { self.close_now(id); }
                        self.continue_close(ids, *exit);
                        return;
                    }
                    Some(_) => keep = false,
                    None => {}
                }
            }
            Confirm::SaveLossy { id, path } => {
                let mut choice = None;
                modal.show(ctx, |ui| {
                    let body = self.t("confirm.lossy_body", "這份文件含有無法以目前編碼解碼的位元組（顯示為 �）。直接存檔會以 U+FFFD 取代，原始位元組將永久遺失。");
                    confirm_body(ui, opened, self.t("confirm.lossy_title", "存檔可能遺失資料"), &[body]);
                    hint(ui, self.t("confirm.lossy_hint", "建議先以「檢視 → 編碼」選擇正確的編碼重新解碼。"));
                    footer(ui, |ui| {
                        if ui.button(self.t("btn.cancel", "取消")).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                            choice = Some(false);
                        }
                        if danger_button(ui, self.t("btn.save_anyway", "仍要存檔")).clicked() {
                            choice = Some(true);
                        }
                    });
                });
                if let Some(c) = choice {
                    keep = false;
                    if c {
                        let (id, path) = (*id, path.clone());
                        self.save_and_continue_close(id, path, true);
                        return;
                    }
                }
            }
            Confirm::SaveUnmappable { id, path, message } => {
                let mut choice = None;
                modal.show(ctx, |ui| {
                    confirm_body(ui, opened, self.t("confirm.unmappable_title", "無法以目前編碼存檔"), &[message]);
                    hint(ui, self.t("confirm.unmappable_hint", "可改用 UTF-8 存檔，或取消後修改內容。"));
                    footer(ui, |ui| {
                        if primary_button(ui, self.t("btn.save_utf8", "改用 UTF-8 存檔")).clicked() {
                            choice = Some(true);
                        }
                        if ui.button(self.t("btn.cancel", "取消")).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                            choice = Some(false);
                        }
                    });
                });
                if let Some(c) = choice {
                    keep = false;
                    if c && let Some(i) = self.index_of(*id) {
                        let bom = self.tabs[i].doc.has_bom;
                        if self.tabs[i].doc.convert_encoding(encoding_rs::UTF_8, bom).is_ok() {
                            let (id, path) = (*id, path.clone());
                            self.save_and_continue_close(id, path, false);
                            return;
                        }
                    }
                }
            }
            Confirm::OverwriteExternal { id, path } => {
                let mut choice = None;
                modal.show(ctx, |ui| {
                    let body = self.t("confirm.external_body", "磁碟上的檔案在開啟後已被修改。覆寫會遺失其他程式所做的變更。");
                    confirm_body(ui, opened, self.t("confirm.external_title", "檔案已被其他程式修改"), &[body]);
                    footer(ui, |ui| {
                        if ui.button(self.t("btn.cancel", "取消")).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                            choice = Some(2);
                        }
                        if danger_button(ui, self.t("btn.reload_discard", "重新載入（放棄我的修改）")).clicked() {
                            choice = Some(1);
                        }
                        if danger_button(ui, self.t("btn.overwrite", "覆寫")).clicked() {
                            choice = Some(0);
                        }
                    });
                });
                if let Some(c) = choice {
                    keep = false;
                    let (id, path) = (*id, path.clone());
                    match c {
                        0 => {
                            self.save_and_continue_close(id, path, false);
                            return;
                        }
                        1 => self.reload_tab(id),
                        _ => {}
                    }
                }
            }
            Confirm::ReloadModified { id } => {
                let mut choice = None;
                modal.show(ctx, |ui| {
                    let body = self.t("confirm.reload_body", "重新載入會放棄目前未存檔的修改，確定嗎？");
                    confirm_body(ui, opened, self.t("confirm.reload_title", "重新載入"), &[body]);
                    footer(ui, |ui| {
                        if ui.button(self.t("btn.cancel", "取消")).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                            choice = Some(false);
                        }
                        if danger_button(ui, self.t("btn.reload", "重新載入")).clicked() {
                            choice = Some(true);
                        }
                    });
                });
                if let Some(c) = choice {
                    keep = false;
                    if c {
                        self.reload_tab(*id);
                    }
                }
            }
            Confirm::CreateNew { path } => {
                let mut choice = None;
                let name = path.display().to_string();
                let body = self.tf("confirm.create_body", "「{q}」不存在，是否要建立新的檔案？", None, Some(&name));
                modal.show(ctx, |ui| {
                    confirm_body(ui, opened, self.t("confirm.create_title", "檔案不存在"), &[&body]);
                    footer(ui, |ui| {
                        if primary_button(ui, self.t("btn.create", "建立")).clicked() {
                            choice = Some(true);
                        }
                        if ui.button(self.t("btn.cancel", "取消")).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                            choice = Some(false);
                        }
                    });
                });
                if let Some(c) = choice {
                    keep = false;
                    if c {
                        self.create_new_file(path.clone());
                    }
                    // 已詢問的路徑移出佇列，其餘的稍後繼續詢問
                    if let Some(pos) = self.pending_create.iter().position(|p| p == path) {
                        self.pending_create.remove(pos);
                    }
                }
            }
        }
        if keep && self.confirm.is_none() {
            self.confirm = Some(confirm);
        } else if !keep && self.confirm.is_none() {
            // 確認視窗已關閉且沒有後續確認（取消、重新載入或存檔失敗）：中止暫存的關閉流程
            self.pending_close = None;
            // 還有其他等待詢問是否建立的檔案時，繼續問下一個
            if let Some(next) = self.pending_create.first() {
                self.confirm = Some(Confirm::CreateNew { path: next.clone() });
            }
        }
        self.shown_confirm = self.confirm.as_ref().map(|_| kind);
    }

    /// 存檔；若關閉流程正等待這份文件存檔，成功後接續關閉
    fn save_and_continue_close(&mut self, id: u64, path: Option<std::path::PathBuf>, allow_lossy: bool) {
        if !self.save_with(id, path, allow_lossy) {
            return;
        }
        if let Some((ids, exit)) = self.pending_close.take_if(|(ids, _)| ids[0] == id) {
            if !exit { self.close_now(id); }
            self.continue_close(&ids, exit);
        }
    }

    /// 處理完一份文件後，繼續詢問下一份；全部處理完且為結束程式時允許關閉
    fn continue_close(&mut self, ids: &[u64], exit: bool) {
        let rest: Vec<u64> = ids[1..].to_vec();
        if rest.is_empty() {
            if exit {
                self.allow_close = true;
            }
        } else {
            self.confirm = Some(Confirm::Close { ids: rest, exit });
        }
    }
}

fn rest_empty_and_exit(next: &Option<Confirm>, exit: bool) -> bool {
    next.is_none() && exit
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `COMMAND_SECTIONS` relies on the command table being ordered by menu; a command added
    /// above a section's first entry would silently land in the previous menu.
    #[test]
    fn test_commands_fall_into_their_menu() {
        let sections = command_sections();
        let section_of = |c: C| sections[commands::ALL.iter().position(|&a| a == c).unwrap()];
        let expected = [
            (C::Save, 0),
            (C::Paste, 1),
            (C::Replace, 2),
            (C::HexMode, 3),
            (C::PlayRecMacro, 4),
            (C::WordCount, 5),
            (C::WindowList, 6),
            (C::SwitchLastTab, 6),
            (C::About, 7),
        ];
        for (c, s) in expected {
            assert_eq!(section_of(c), s, "{c:?} is listed under {}", COMMAND_SECTIONS[section_of(c)].2);
        }
        for (i, &(first, _, _)) in COMMAND_SECTIONS.iter().enumerate() {
            assert_eq!(section_of(first), i, "{first:?} must start its own section");
        }
    }

    #[test]
    fn test_column_split_balances_whole_blocks() {
        assert_eq!(column_split(&[10, 10, 10, 10]), 2);
        assert_eq!(column_split(&[30, 5, 5, 5, 5]), 1);
        assert_eq!(column_split(&[5, 5, 5, 5, 30]), 4);
        assert_eq!(column_split(&[50, 1]), 1, "the left column always gets a block");
        assert_eq!(column_split(&[7]), 1);
        assert_eq!(column_split(&[]), 0);
    }

    #[test]
    fn test_tilde_path_hides_the_home_directory() {
        let Some(home) = dirs::home_dir() else { return };
        let shown = tilde_path(&home.join("cfg").join("settings.json"));
        assert!(shown.starts_with('~') && shown.ends_with("settings.json"), "{shown}");
        assert!(!shown.contains(&*home.to_string_lossy()));
        let elsewhere = std::path::Path::new("/opt/madedit/settings.json");
        assert_eq!(tilde_path(elsewhere), elsewhere.display().to_string());
    }
}
