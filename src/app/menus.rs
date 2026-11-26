//! 選單列與工具列（結構對應原版 MadEdit 的 CommandTable）

use eframe::egui::{self, Button};

use super::commands::{plain_label, C};
use super::{form, App};
use crate::core::encoding::{self, ALL_ENCODINGS, COMMON_ENCODINGS};
use crate::core::model::{EditMode, NewLineFormat};
use crate::core::settings::{Settings, ThemeChoice, UiLanguage, WrapMode};
use crate::core::syntax;

/// Room the zoom / theme / quick-search controls need at the right of the menu bar
/// (widest language included); with less free space they stay in the toolbar.
pub(super) const QUICK_CONTROLS_WIDTH: f32 = 340.0;

/// Minimum widths of top-level menus and of submenus / context menus. egui sizes every popup
/// to its widest item, so without a floor Help (one item) is a sliver next to Search and
/// widths jump around.
const MENU_MIN_WIDTH: f32 = 240.0;
pub(super) const SUBMENU_MIN_WIDTH: f32 = 180.0;

/// `ui.menu_button` with a minimum width, whose items scroll when they do not fit between the
/// popup's top and the window bottom: egui menus never scroll on their own, so long menus
/// (View, font sizes, all encodings) used to run off the window.
fn menu<'a>(ui: &mut egui::Ui, title: impl egui::IntoAtoms<'a>, add: impl FnOnce(&mut egui::Ui)) {
    let submenu = egui::containers::menu::is_in_menu(ui);
    let min_width = if submenu { SUBMENU_MIN_WIDTH } else { MENU_MIN_WIDTH };
    // Submenus may move above their parent item to fit the screen. Deriving their height
    // from that moving popup position makes height and placement oscillate every frame.
    // Give them the viewport height; top-level menus keep the room below the menu bar.
    let top = if submenu { ui.ctx().content_rect().top() } else {
        ui.next_widget_position().y + ui.spacing().interact_size.y
    };
    let room = (ui.ctx().content_rect().bottom() - top - 12.0).max(160.0);
    ui.menu_button(title, |ui| {
        ui.set_min_width(min_width);
        // A popup's first (sizing) pass is capped at 400 pt and later passes at the last frame's
        // size, so `max_height` alone would freeze long menus there; `min_scrolled_height` lets
        // them grow to the room below (short menus still shrink to their content).
        egui::ScrollArea::vertical().max_height(room).min_scrolled_height(room).show(ui, |ui| {
            ui.set_min_width(min_width);
            add(ui);
        });
    });
}

/// 選單項目的勾選標記
pub(super) fn check_label(checked: bool, text: &str) -> String {
    format!("{} {}", if checked { "✔" } else { "　" }, text)
}

/// Draws the theme toggle icon with shapes rather than glyphs, so it looks the same in every
/// font: a sun for the light theme and a crescent moon for the dark one.
fn paint_theme_icon(painter: &egui::Painter, c: egui::Pos2, r: f32, color: egui::Color32, dark: bool) {
    use std::f32::consts::{PI, TAU};
    let at = |center: egui::Pos2, radius: f32, angle: f32| center + radius * egui::Vec2::angled(angle);
    if dark {
        // Outer disc minus a smaller disc shifted right (then the whole shape tilted).
        // Both arcs run from the upper tip around the left side to the lower tip, so
        // matching samples form a triangle strip that fills the (non-convex) crescent.
        let (d, r1) = (0.55 * r, 0.8 * r);
        let x = (d * d + r * r - r1 * r1) / (2.0 * d);
        let y = (r * r - x * x).sqrt();
        let (outer0, inner0) = (y.atan2(x), y.atan2(x - d));
        let tilt = -PI / 5.0;
        let rot = egui::Vec2::angled(tilt);
        let inner_c = c + d * rot;
        let n = 24;
        let mut mesh = egui::Mesh::default();
        let mut outline = Vec::with_capacity(2 * n + 2);
        for i in 0..=n {
            let t = i as f32 / n as f32;
            let o = at(c, r, tilt + outer0 + t * (TAU - 2.0 * outer0));
            let p = at(inner_c, r1, tilt + inner0 + t * (TAU - 2.0 * inner0));
            mesh.colored_vertex(o, color);
            mesh.colored_vertex(p, color);
            if i > 0 {
                let k = 2 * i as u32;
                mesh.add_triangle(k - 2, k - 1, k);
                mesh.add_triangle(k - 1, k + 1, k);
            }
            outline.push(o);
        }
        for i in (0..=n).rev() {
            let t = i as f32 / n as f32;
            outline.push(at(inner_c, r1, tilt + inner0 + t * (TAU - 2.0 * inner0)));
        }
        painter.add(egui::Shape::mesh(mesh));
        // A thin outline anti-aliases the mesh edges.
        painter.add(egui::Shape::closed_line(outline, egui::Stroke::new(1.0, color)));
    } else {
        painter.circle_filled(c, r * 0.5, color);
        for i in 0..8 {
            let a = TAU * i as f32 / 8.0;
            painter.line_segment([at(c, r * 0.75, a), at(c, r * 1.1, a)], egui::Stroke::new(1.5, color));
        }
    }
}

impl App {
    /// 一般命令項目
    fn item(&mut self, ui: &mut egui::Ui, c: C) {
        self.item_enabled(ui, c, true);
    }

    fn item_enabled(&mut self, ui: &mut egui::Ui, c: C, enabled: bool) {
        let sc = self.shortcut_text(c, ui.ctx());
        if ui.add_enabled(enabled, Button::new(check_label(false, &self.cmd_text(c))).shortcut_text(sc)).clicked() {
            self.pending.push(c);
            ui.close();
        }
    }

    /// 可勾選的命令項目
    fn check(&mut self, ui: &mut egui::Ui, c: C, checked: bool) {
        let sc = self.shortcut_text(c, ui.ctx());
        if ui.add(Button::new(check_label(checked, &self.cmd_text(c))).shortcut_text(sc)).clicked() {
            self.pending.push(c);
            ui.close();
        }
    }

    /// 原生選單列（非 Windows 平台仍在用；Windows 上選單已移進自繪標題欄）。
    #[cfg_attr(windows, allow(dead_code))]
    pub fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            self.menus(ui);
            // Zoom, theme and quick search sit at the right end of the menu bar, so the toolbar
            // stays a single row of icons. Below this width they would overlap the menus. They do
            // not depend on the toolbar being shown, or hiding it would remove the theme toggle.
            if ui.available_width() >= QUICK_CONTROLS_WIDTH {
                self.quick_in_menubar = true;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| self.quick_controls(ui, true));
            }
        });
    }

    /// 八個下拉選單本体（不含容器與右側快捷控制項），供原生選單列與 Windows 自繪標題欄共用。
    pub fn menus(&mut self, ui: &mut egui::Ui) {
        menu(ui, self.t("menu.file", "檔案(F)"), |ui| self.file_menu(ui));
        menu(ui, self.t("menu.edit", "編輯(E)"), |ui| self.edit_menu(ui));
        menu(ui, self.t("menu.search", "搜尋(S)"), |ui| self.search_menu(ui));
        menu(ui, self.t("menu.view", "檢視(V)"), |ui| self.view_menu(ui));
        menu(ui, self.t("menu.macro", "巨集(M)"), |ui| self.macro_menu(ui));
        menu(ui, self.t("menu.tools", "工具(T)"), |ui| self.tools_menu(ui));
        menu(ui, self.t("menu.window", "視窗(W)"), |ui| self.window_menu(ui));
        menu(ui, self.t("menu.help", "說明(H)"), |ui| {
            self.item(ui, C::KeyboardShortcuts);
            ui.separator();
            if ui.add_enabled(!self.updates.busy(), Button::new(self.t("updates.check", "檢查更新"))).clicked() {
                self.start_update_check(ui.ctx(), true);
                ui.close();
            }
            let label = self.t("updates.auto", "自動檢查更新（每日）");
            if ui.checkbox(&mut self.settings.auto_check_updates, label).changed() {
                self.save_settings();
            }
            self.item(ui, C::About);
        });
    }

    /// Font zoom, theme switch and quick search. `rtl` is set inside a right-to-left layout,
    /// where widgets are placed from the right edge and must therefore be added in reverse.
    pub(super) fn quick_controls(&mut self, ui: &mut egui::Ui, rtl: bool) {
        let zoom_out = ("A−", self.t("zoom.out", "縮小字型"), C::ZoomOut);
        let zoom_in = ("A+", self.t("zoom.in", "放大字型"), C::ZoomIn);
        let size = self.settings.font_size.to_string();
        let mut zooms = [zoom_out, zoom_in];
        let mut parts = [0, 1, 2];
        if rtl {
            zooms.reverse();
            parts.reverse();
        }
        let quick = self.t("find.quick", "快速查找");
        for part in parts.into_iter() {
            match part {
                0 => {
                    for (i, (label, tip, c)) in zooms.into_iter().enumerate() {
                        if i == 1 {
                            ui.label(&size);
                        }
                        // 閒置時底色透明（深淺主題都與標題欄底色融為一體），保留邊框與懸停高亮。
                        ui.scope(|ui| {
                            ui.style_mut().visuals.widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
                            if form::icon_button(ui, Button::new(label), tip, tip).clicked() {
                                self.pending.push(c);
                            }
                        });
                    }
                }
                1 => self.theme_toggle(ui),
                _ => {
                    let resp = ui.add(egui::TextEdit::singleline(&mut self.find.query).desired_width(140.0).hint_text(quick));
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        let back = ui.input(|i| i.modifiers.shift);
                        self.find_next(back);
                        resp.request_focus();
                    }
                    // 與查找／取代浮窗同一套歷史下拉（chevron 疊在框內右端）
                    let tip = self.t("find.history", "查找歷史");
                    let remove = self.t("find.history_remove", "移除此紀錄");
                    let clear = self.t("find.history_clear", "清空搜尋紀錄");
                    match super::find::history_chevron_dropdown(ui, "quick-history", resp.rect, &mut self.find.quick_history_open, &self.settings.recent_finds, &tip, &remove, &clear) {
                        super::find::HistAction::Pick(h) => self.find.query = h,
                        super::find::HistAction::Remove(h) => self.settings.recent_finds.retain(|x| *x != h),
                        super::find::HistAction::Clear => self.settings.recent_finds.clear(),
                        super::find::HistAction::None => {}
                    }
                }
            }
        }
    }

    fn file_menu(&mut self, ui: &mut egui::Ui) {
        let has_path = self.doc().path.is_some();
        self.item(ui, C::New);
        self.item(ui, C::Open);
        ui.separator();
        self.item(ui, C::Save);
        self.item(ui, C::SaveAs);
        self.item(ui, C::SaveACopy);
        self.item(ui, C::SaveAll);
        ui.separator();
        self.item_enabled(ui, C::Reload, has_path);
        ui.separator();
        self.item(ui, C::Close);
        self.item(ui, C::CloseAll);
        self.item(ui, C::CloseOthers);
        self.item(ui, C::CloseLeft);
        self.item(ui, C::CloseRight);
        ui.separator();
        let ro = self.doc().read_only;
        self.check(ui, C::ToggleReadOnly, ro);
        ui.separator();
        self.item(ui, C::PageSetup);
        self.item(ui, C::PrintPreview);
        self.item(ui, C::Print);
        ui.separator();
        menu(ui, check_label(false, self.t("menu.recent", "最近使用的檔案")), |ui| {
            let recent = self.settings.recent_files.clone();
            if recent.is_empty() {
                ui.label(self.t("menu.none", "（無）"));
            }
            for (i, p) in recent.iter().enumerate() {
                if ui.button(format!("{} {}", i + 1, p.display())).clicked() {
                    self.open_path(p);
                    ui.close();
                }
            }
        });
        self.item(ui, C::RecentFilesList);
        ui.separator();
        self.item(ui, C::Exit);
    }

    fn edit_menu(&mut self, ui: &mut egui::Ui) {
        let can_undo = self.doc().can_undo();
        let can_redo = self.doc().can_redo();
        self.item_enabled(ui, C::Undo, can_undo);
        self.item_enabled(ui, C::Redo, can_redo);
        ui.separator();
        self.item(ui, C::Cut);
        self.item(ui, C::Copy);
        self.item(ui, C::Paste);
        self.item(ui, C::Delete);
        ui.separator();
        self.item(ui, C::CutLine);
        self.item(ui, C::DeleteLine);
        self.item(ui, C::DuplicateLine);
        self.item(ui, C::MoveLinesUp);
        self.item(ui, C::MoveLinesDown);
        self.item(ui, C::JoinLines);
        self.item(ui, C::DeleteEmptyLines);
        self.item(ui, C::DeleteEmptyLinesWithSpaces);
        self.item(ui, C::TrimTrailingSpaces);
        self.item(ui, C::TrimLeadingSpaces);
        ui.separator();
        self.item(ui, C::SelectAll);
        let sel_mode = self.doc().selecting_mode;
        self.check(ui, C::StartEndSelecting, sel_mode);
        ui.separator();
        self.item(ui, C::InsertTabChar);
        self.item(ui, C::InsertDateTime);
        ui.separator();
        menu(ui, check_label(false, self.t("menu.bookmarks", "書籤")), |ui| {
            for c in [C::BookmarkCopy, C::BookmarkCut, C::BookmarkDel, C::BookmarkCopyUnmarked, C::BookmarkCutUnmarked, C::BookmarkDelUnmarked, C::BookmarkReplace] {
                self.item(ui, c);
            }
        });
        ui.separator();
        menu(ui, check_label(false, self.t("menu.advanced", "進階")), |ui| {
            self.item(ui, C::CopyAsHexString);
            self.item(ui, C::CopyAsHexStringWithSpace);
            self.item(ui, C::CopyRevertHex);
            ui.separator();
            self.item(ui, C::IncreaseIndent);
            self.item(ui, C::DecreaseIndent);
            ui.separator();
            self.item(ui, C::Comment);
            self.item(ui, C::Uncomment);
            ui.separator();
            self.item(ui, C::WordWrapToNewLine);
            self.item(ui, C::NewLineToWordWrap);
            ui.separator();
            self.item(ui, C::ToUpperCase);
            self.item(ui, C::ToLowerCase);
            self.item(ui, C::InvertCase);
            self.item(ui, C::Capitalize);
            ui.separator();
            self.item(ui, C::ToHalfWidth);
            self.item(ui, C::ToHalfWidthByOptions);
            self.item(ui, C::ToFullWidth);
            self.item(ui, C::ToFullWidthByOptions);
            ui.separator();
            self.item(ui, C::TabToSpace);
            self.item(ui, C::SpaceToTab);
            ui.separator();
            self.item(ui, C::InsertNumbers);
            self.item(ui, C::ColumnAlignLeft);
            self.item(ui, C::ColumnAlignRight);
        });
        ui.separator();
        menu(ui, check_label(false, self.t("menu.sort", "排序")), |ui| {
            self.item(ui, C::SortAscending);
            self.item(ui, C::SortDescending);
            ui.separator();
            self.item(ui, C::SortAscendingCase);
            self.item(ui, C::SortDescendingCase);
            ui.separator();
            self.item(ui, C::SortByOptions);
            self.item(ui, C::SortOptions);
        });
    }

    fn search_menu(&mut self, ui: &mut egui::Ui) {
        self.item(ui, C::Find);
        self.item(ui, C::FindNext);
        self.item(ui, C::FindPrevious);
        self.item(ui, C::QuickFind);
        ui.separator();
        self.item(ui, C::Replace);
        ui.separator();
        self.item(ui, C::FindInFiles);
        let shown = self.fif.show_results;
        self.check(ui, C::ShowFindInFilesResults, shown);
        let collapsed = self.fif.collapsed;
        self.check(ui, C::CollapseAllResults, collapsed);
        ui.separator();
        self.item(ui, C::GoToLine);
        self.item(ui, C::GoToPosition);
        ui.separator();
        self.item(ui, C::GoBack);
        self.item(ui, C::GoForward);
        ui.separator();
        self.item(ui, C::LeftBrace);
        self.item(ui, C::RightBrace);
        ui.separator();
        self.item(ui, C::ToggleBookmark);
        self.item(ui, C::NextBookmark);
        self.item(ui, C::PreviousBookmark);
        self.item(ui, C::ClearAllBookmarks);
    }

    /// 檢視選單最上方：立刻切換介面語言並寫入設定
    fn language_menu(&mut self, ui: &mut egui::Ui) {
        let current = self.settings.ui_language;
        let lang = self.ui_lang();
        menu(ui, check_label(false, self.t("menu.language", "介面語言")), |ui| {
            for choice in UiLanguage::ALL {
                let label = super::i18n::language_choice_label(lang, choice);
                if ui.button(check_label(current == choice, label)).clicked() {
                    self.settings.ui_language = choice;
                    let _ = self.settings.save();
                    ui.close();
                }
            }
        });
    }

    /// Theme choices with their translated labels for the View menu.
    fn theme_choices(&self) -> [(ThemeChoice, &'static str); 3] {
        [
            (ThemeChoice::System, self.t("opt.theme_system", "跟隨系統")),
            (ThemeChoice::Light, self.t("opt.theme_light", "淺色")),
            (ThemeChoice::Dark, self.t("opt.theme_dark", "深色")),
        ]
    }

    /// One icon button that switches between the light and dark themes. The icon shows the
    /// theme in use (also when following the system), and the tooltip / accessible label name
    /// it and the theme a click switches to.
    fn theme_toggle(&mut self, ui: &mut egui::Ui) {
        let dark = App::is_dark(ui.ctx());
        let (light_name, dark_name) = (self.t("opt.theme_light", "淺色"), self.t("opt.theme_dark", "深色"));
        let (current, next) = if dark { (dark_name, light_name) } else { (light_name, dark_name) };
        let mut label = self.tf("tb.theme_toggle", "介面主題：{q}", None, Some(current));
        // A click ends following the system, so say when that is what happens now.
        if self.settings.theme == ThemeChoice::System {
            label = format!("{label} · {}", self.t("opt.theme_system", "跟隨系統"));
        }
        let tip = format!("{label}\n{}", self.tf("tb.theme_next", "按一下切換為「{q}」", None, Some(next)));
        let side = ui.spacing().interact_size.y;
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
        if ui.is_rect_visible(rect) {
            let visuals = ui.style().interact(&resp);
            if resp.hovered() || resp.has_focus() {
                ui.painter().rect(rect, visuals.corner_radius, visuals.bg_fill, visuals.bg_stroke, egui::StrokeKind::Inside);
            }
            paint_theme_icon(ui.painter(), rect.center(), side * 0.32, visuals.fg_stroke.color, dark);
        }
        if resp.on_hover_text(tip).clicked() {
            self.toggle_theme(ui.ctx());
        }
    }

    /// View > Theme: System / Light / Dark, applied and saved immediately.
    fn theme_menu(&mut self, ui: &mut egui::Ui) {
        let current = self.settings.theme;
        let choices = self.theme_choices();
        menu(ui, check_label(false, self.t("menu.theme", "介面主題")), |ui| {
            for (choice, label) in choices {
                if ui.button(check_label(current == choice, label)).clicked() {
                    self.set_theme(ui.ctx(), choice);
                    ui.close();
                }
            }
        });
    }

    /// The View menu is grouped so its top level stays short: edit modes, document type,
    /// editor appearance (font / layout / show submenus), preview, then window chrome.
    /// Toggles with well-known shortcuts (full screen, always on top) stay at the top level.
    fn view_menu(&mut self, ui: &mut egui::Ui) {
        let s = self.settings.clone();
        self.edit_mode_items(ui);
        ui.separator();
        self.encoding_menu(ui);
        self.syntax_menu(ui);
        ui.separator();
        menu(ui, check_label(false, self.t("menu.font", "字型")), |ui| self.font_menu(ui, &s));
        menu(ui, check_label(false, self.t("menu.layout", "排版")), |ui| self.layout_menu(ui, &s));
        menu(ui, check_label(false, self.t("menu.show", "顯示")), |ui| self.show_menu(ui, &s));
        ui.separator();
        // 勾選式總開關：依副檔名自動分發到 Markdown／HTML／圖形預覽
        self.check(ui, C::PreviewAuto, s.auto_preview);
        ui.separator();
        menu(ui, check_label(false, self.t("menu.fold", "代碼折疊")), |ui| {
            self.check(ui, C::ToggleCodeFolding, s.code_folding);
            ui.separator();
            self.item(ui, C::FoldToggle);
            self.item(ui, C::FoldAll);
            self.item(ui, C::UnfoldAll);
        });
        self.language_menu(ui);
        self.theme_menu(ui);
        menu(ui, check_label(false, self.t("menu.toolbars", "工具列")), |ui| {
            self.check(ui, C::ToggleToolbar, s.show_toolbar);
            self.check(ui, C::ToggleMenubar, s.show_menubar);
            self.check(ui, C::ToggleStatusbar, s.show_statusbar);
        });
        self.check(ui, C::ToggleFileView, s.show_file_view);
        // 運行輸出面板也能像文件視圖一樣手動顯示/隱藏；執行腳本時會自動打開。
        let script_output_shown = self.runner.show;
        self.check(ui, C::ToggleScriptOutput, script_output_shown);
        self.check(ui, C::FullScreen, self.fullscreen);
        self.check(ui, C::AlwaysOnTop, s.always_on_top);
        self.check(ui, C::PostIt, self.post_it);
    }

    /// Text / Column / Hex mode; text and column are unavailable for hex-only (binary) files.
    fn edit_mode_items(&mut self, ui: &mut egui::Ui) {
        let mode = self.doc().mode;
        let hex_only = self.doc().hex_only;
        for (c, m) in [(C::TextMode, EditMode::Text), (C::ColumnMode, EditMode::Column)] {
            let sc = self.shortcut_text(c, ui.ctx());
            if ui.add_enabled(!hex_only, Button::new(check_label(mode == m, &self.cmd_text(c))).shortcut_text(sc)).clicked() {
                self.pending.push(c);
                ui.close();
            }
        }
        self.check(ui, C::HexMode, mode == EditMode::Hex);
    }

    fn encoding_menu(&mut self, ui: &mut egui::Ui) {
        let enc_name = self.doc().encoding.name().to_string();
        menu(ui, check_label(false, &self.tf("menu.encoding", "編碼：{q}", None, Some(&enc_name))), |ui| {
            let mut chosen = None;
            for &e in COMMON_ENCODINGS {
                if ui.button(check_label(e == enc_name, e)).clicked() {
                    chosen = Some(e);
                }
            }
            ui.separator();
            menu(ui, check_label(false, self.t("menu.all_encodings", "所有編碼")), |ui| {
                for &e in ALL_ENCODINGS {
                    if ui.button(check_label(e == enc_name, e)).clicked() {
                        chosen = Some(e);
                    }
                }
            });
            if let Some(e) = chosen {
                ui.close();
                self.reinterpret(e);
            }
        });
    }

    fn syntax_menu(&mut self, ui: &mut egui::Ui) {
        let syn = self.doc().syntax.clone();
        menu(ui, check_label(false, &self.tf("menu.syntax", "語法類型：{q}", None, Some(&syn))), |ui| {
            for name in syntax::syntax_names() {
                if ui.button(check_label(name == syn, &name)).clicked() {
                    let doc = self.doc_mut();
                    doc.syntax = name.clone();
                    doc.syntax_cache.clear();
                    ui.close();
                }
            }
        });
    }

    /// View > Font: family, size, zoom and fixed-width mode.
    fn font_menu(&mut self, ui: &mut egui::Ui, s: &Settings) {
        let fam = if s.font_family.is_empty() { self.fonts.editor_family.clone() } else { s.font_family.clone() };
        menu(ui, check_label(false, &self.tf("menu.font_name", "字型名稱：{q}", None, Some(&fam))), |ui| {
            type Group = (&'static str, fn(char) -> bool);
            let groups: [Group; 7] = [
                ("[@]", |c| c == '@'),
                ("[ABCDE]", |c| ('A'..='E').contains(&c)),
                ("[FGHIJ]", |c| ('F'..='J').contains(&c)),
                ("[KLMNO]", |c| ('K'..='O').contains(&c)),
                ("[PQRST]", |c| ('P'..='T').contains(&c)),
                ("[UVWXYZ]", |c| ('U'..='Z').contains(&c)),
                (self.t("menu.other", "[其他]"), |c| !c.is_ascii_alphabetic() && c != '@'),
            ];
            for (label, pred) in groups {
                menu(ui, label, |ui| {
                    for f in self.fonts.families.clone() {
                        let first = f.chars().next().unwrap_or(' ').to_ascii_uppercase();
                        if pred(first) && ui.button(check_label(f == fam, &f)).clicked() {
                            self.settings.font_family = f.clone();
                            self.save_settings();
                            ui.close();
                        }
                    }
                });
            }
        });
        let size_label = self.tf("menu.font_size", "字型大小：{q}", None, Some(&s.font_size.to_string()));
        menu(ui, check_label(false, &size_label), |ui| {
            for n in 1..=72 {
                if ui.button(check_label(s.font_size as i32 == n, &n.to_string())).clicked() {
                    self.settings.font_size = n as f32;
                    self.save_settings();
                    ui.close();
                }
            }
        });
        self.item(ui, C::SetFont);
        ui.separator();
        self.item(ui, C::ZoomIn);
        self.item(ui, C::ZoomOut);
        ui.separator();
        self.check(ui, C::FixedWidthMode, s.fixed_width);
    }

    /// View > Layout: word wrap, tab width, line spacing and text direction.
    fn layout_menu(&mut self, ui: &mut egui::Ui, s: &Settings) {
        self.check(ui, C::NoWrap, s.wrap_mode == WrapMode::NoWrap);
        self.check(ui, C::WrapByWindow, s.wrap_mode == WrapMode::ByWindow);
        self.check(ui, C::WrapByColumn, s.wrap_mode == WrapMode::ByColumn);
        ui.separator();
        let tab_label = self.tf("menu.tab_width", "Tab 欄數：{q}", None, Some(&s.tab_size.to_string()));
        menu(ui, check_label(false, &tab_label), |ui| {
            for n in 1..=16 {
                if ui.button(check_label(s.tab_size == n, &n.to_string())).clicked() {
                    self.settings.tab_size = n;
                    self.save_settings();
                    ui.close();
                }
            }
        });
        let spacing_label = self.tf("menu.line_spacing", "行距：{q}%", None, Some(&s.line_spacing.to_string()));
        menu(ui, check_label(false, &spacing_label), |ui| {
            for n in (100..=250).step_by(5) {
                if ui.button(check_label(s.line_spacing == n, &format!("{n}%"))).clicked() {
                    self.settings.line_spacing = n;
                    self.save_settings();
                    ui.close();
                }
            }
        });
        ui.separator();
        self.check(ui, C::RightToLeft, s.right_to_left);
        self.check(ui, C::TypewriterMode, s.typewriter_mode);
    }

    /// View > Show: gutter and guides, invisible characters, highlights and spell check.
    fn show_menu(&mut self, ui: &mut egui::Ui, s: &Settings) {
        self.check(ui, C::DisplayLineNumber, s.show_line_numbers);
        self.check(ui, C::DisplayBookmark, s.show_bookmarks);
        self.check(ui, C::Display80ColHint, s.show_col_hint);
        self.check(ui, C::ShowRuler, s.show_ruler);
        ui.separator();
        self.check(ui, C::ShowEndOfLine, s.show_eol);
        self.check(ui, C::ShowTabChar, s.show_tab);
        self.check(ui, C::ShowSpaceChar, s.show_space);
        self.check(ui, C::ShowAllChar, s.show_eol && s.show_tab && s.show_space);
        ui.separator();
        self.check(ui, C::MarkActiveLine, s.mark_active_line);
        self.check(ui, C::MarkBracePair, s.mark_brace_pair);
        self.check(ui, C::SpellCheck, s.spell_check);
    }

    fn macro_menu(&mut self, ui: &mut egui::Ui) {
        let rec = self.macros.recording;
        let has = !self.macros.steps.is_empty();
        self.item(ui, C::RunTempMacro);
        self.item(ui, C::LoadMacro);
        ui.separator();
        self.item_enabled(ui, C::StartRecMacro, !rec);
        self.item_enabled(ui, C::StopRecMacro, rec);
        self.item_enabled(ui, C::PlayRecMacro, !rec && has);
        self.item_enabled(ui, C::SaveRecMacro, !rec && has);
        ui.separator();
        menu(ui, check_label(false, self.t("menu.local_macros", "本機巨集清單")), |ui| {
            self.item(ui, C::EditMacroFile);
            ui.separator();
            let list = super::macros::list_macros();
            if list.is_empty() {
                ui.label(self.t("menu.no_macros", "（尚無已儲存的巨集）"));
            }
            for p in list {
                let name = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                if ui.button(name).clicked() {
                    self.load_macro_file(&p);
                    ui.close();
                }
            }
        });
    }

    fn tools_menu(&mut self, ui: &mut egui::Ui) {
        self.item(ui, C::Options);
        ui.separator();
        self.item(ui, C::RunScript);
        ui.separator();
        self.item(ui, C::Highlighting);
        self.item(ui, C::FileAssociation);
        self.item(ui, C::SourceFormatter);
        self.item(ui, C::PurgeHistories);
        ui.separator();
        let bom = self.doc().has_bom;
        let can_bom = encoding::supports_bom(self.doc().encoding);
        let bom_state = if bom { self.t("menu.bom_yes", "有") } else { self.t("menu.bom_no", "無") };
        menu(ui, check_label(false, &self.tf("menu.bom", "Unicode BOM：{q}", None, Some(bom_state))), |ui| {
            self.item_enabled(ui, C::ToggleBOM, can_bom);
        });
        ui.separator();
        let nl = self.doc().newline;
        let nl_name = match nl {
            NewLineFormat::Dos => "CRLF",
            NewLineFormat::Unix => "LF",
            NewLineFormat::Mac => "CR",
        };
        menu(ui, check_label(false, &self.tf("menu.newline", "換行字元（檔案格式）：{q}", None, Some(nl_name))), |ui| {
            self.check(ui, C::ConvertToDOS, nl == NewLineFormat::Dos);
            self.check(ui, C::ConvertToMac, nl == NewLineFormat::Mac);
            self.check(ui, C::ConvertToUNIX, nl == NewLineFormat::Unix);
        });
        menu(ui, check_label(false, &self.tf("menu.newline_enter", "按 Enter 插入的換行字元：{q}", None, Some(nl_name))), |ui| {
            ui.label(self.t("menu.newline_same", "本程式整份文件使用同一種換行字元，此設定與上方的檔案格式相同。"));
            self.check(ui, C::InsertDOS, nl == NewLineFormat::Dos);
            self.check(ui, C::InsertMac, nl == NewLineFormat::Mac);
            self.check(ui, C::InsertUNIX, nl == NewLineFormat::Unix);
        });
        ui.separator();
        self.item(ui, C::ConvertEncoding);
        ui.separator();
        menu(ui, check_label(false, self.t("menu.chinese", "簡繁轉換")), |ui| {
            let safe = self.chinese_safe;
            self.check(ui, C::ChineseSafeMode, safe);
            ui.separator();
            for c in [C::Simp2TradChinese, C::Trad2SimpChinese, C::Kanji2TradChinese, C::Kanji2SimpChinese, C::Chinese2Kanji] {
                self.item(ui, c);
            }
            ui.separator();
            for c in [C::Simp2TradClipboard, C::Trad2SimpClipboard, C::Kanji2TradClipboard, C::Kanji2SimpClipboard, C::Chinese2KanjiClipboard] {
                self.item(ui, c);
            }
        });
        ui.separator();
        menu(ui, check_label(false, self.t("menu.convert", "文字轉換／格式化")), |ui| {
            self.item(ui, C::Markdown2Html);
            self.item(ui, C::Html2PlainText);
            ui.separator();
            self.item(ui, C::AstyleFormat);
            self.item(ui, C::XmlFormat);
            self.item(ui, C::JsonFormat);
            self.item(ui, C::DprintFormat);
        });
        ui.separator();
        self.item(ui, C::WordCount);
    }

    fn window_menu(&mut self, ui: &mut egui::Ui) {
        self.item(ui, C::ToggleWindow);
        self.item(ui, C::PreviousWindow);
        self.item(ui, C::NextWindow);
        self.item(ui, C::WindowList);
        ui.separator();
        let mut go = None;
        for (i, (index, preview)) in self.tab_pages().into_iter().enumerate() {
            let doc = &self.tabs[index].doc;
            let suffix = if preview { format!(" — {}", self.t("md.title", "Markdown 預覽")) } else { String::new() };
            let name = format!("{} {}{}{}", i + 1, doc.display_name(), if doc.is_modified() { " *" } else { "" }, suffix);
            if ui.button(check_label(index == self.active && preview == self.is_preview_tab(), &name)).clicked() {
                go = Some(i);
                ui.close();
            }
        }
        if let Some(i) = go {
            self.activate_page(i);
        }
    }

    /// 以指定編碼重新解碼目前文件
    pub fn reinterpret(&mut self, name: &str) {
        let Some(enc) = encoding::encoding_for_name(name) else { return };
        if self.doc().hex_only {
            self.doc_mut().encoding = enc;
            return;
        }
        if self.doc().is_modified() && self.doc().has_decode_error {
            self.message("無法重新解碼", "文件已修改且原本解碼有誤，無法取得原始位元組；請先重新載入檔案。");
            return;
        }
        let was_hex = self.doc().mode == EditMode::Hex;
        if was_hex {
            self.doc_mut().encoding = enc;
            return;
        }
        match self.doc_mut().reinterpret(enc) {
            Ok(()) => {
                let msg = if self.doc().has_decode_error { format!("以 {name} 解碼時有無法辨識的位元組") } else { format!("已改用 {name} 解碼") };
                self.set_status(msg);
            }
            Err(e) => self.message("無法重新解碼", e),
        }
    }

    pub fn toolbar(&mut self, ui: &mut egui::Ui) {
        let resp = ui
            .horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let wrap = self.settings.wrap_mode;
                self.icon_group(ui, &[("new", C::New, None), ("fileopen", C::Open, None), ("filesave", C::Save, None), ("saveall", C::SaveAll, None), ("fileclose", C::Close, None), ("closeall", C::CloseAll, None)]);
                ui.separator();
                self.icon_group(ui, &[("print", C::Print, None)]);
                ui.separator();
                self.icon_group(ui, &[("undo", C::Undo, None), ("redo", C::Redo, None)]);
                ui.separator();
                self.icon_group(ui, &[("cut", C::Cut, None), ("copy", C::Copy, None), ("paste", C::Paste, None)]);
                ui.separator();
                self.icon_group(ui, &[("indent", C::IncreaseIndent, None), ("unindent", C::DecreaseIndent, None), ("comment", C::Comment, None), ("uncomment", C::Uncomment, None)]);
                ui.separator();
                self.icon_group(ui, &[("find", C::Find, None), ("findnext", C::FindNext, None), ("findprev", C::FindPrevious, None), ("replace", C::Replace, None), ("goposition", C::GoToLine, None)]);
                ui.separator();
                self.icon_group(ui, &[
                    ("nowrap", C::NoWrap, Some(wrap == WrapMode::NoWrap)),
                    ("wrapbywin", C::WrapByWindow, Some(wrap == WrapMode::ByWindow)),
                    ("wrapbycol", C::WrapByColumn, Some(wrap == WrapMode::ByColumn)),
                ]);
                ui.separator();
                self.edit_mode_group(ui);
                ui.separator();
                self.icon_group(ui, &[("bookmark_toggle", C::ToggleBookmark, None), ("bookmark_next", C::NextBookmark, None), ("bookmark_prev", C::PreviousBookmark, None), ("bookmark_clear", C::ClearAllBookmarks, None)]);
                ui.separator();
                self.icon_group(ui, &[("preview", C::PreviewMarkdown, None)]);
                if !self.quick_in_menubar {
                    ui.separator();
                    self.quick_controls(ui, false);
                }
            })
            .response;
        resp.context_menu(|ui| {
            ui.set_min_width(SUBMENU_MIN_WIDTH);
            let s = self.settings.clone();
            self.check(ui, C::ToggleToolbar, s.show_toolbar);
            self.check(ui, C::ToggleMenubar, s.show_menubar);
            self.check(ui, C::ToggleStatusbar, s.show_statusbar);
        });
    }

    /// One group of toolbar icons, separated from its neighbours by a divider.
    fn icon_group(&mut self, ui: &mut egui::Ui, group: &[(&str, C, Option<bool>)]) {
        let can_undo = self.doc().can_undo();
        let can_redo = self.doc().can_redo();
        let is_md = self.doc().is_markdown();
        for &(icon, c, selected) in group {
            let Some((light, dark)) = self.icons.get(icon) else { continue };
            let tex = if ui.visuals().dark_mode { dark } else { light };
            let img = egui::Image::new((tex.id(), egui::vec2(16.0, 16.0)));
            let mut btn = Button::image(img).frame_when_inactive(false);
            if let Some(sel) = selected {
                btn = btn.selected(sel);
            }
            let enabled = match c {
                C::Undo => can_undo,
                C::Redo => can_redo,
                C::PreviewMarkdown => is_md,
                _ => true,
            };
            let sc = self.shortcut_text(c, ui.ctx());
            let tip = if sc.is_empty() { self.cmd_text(c) } else { format!("{} ({sc})", self.cmd_text(c)) };
            // Announced without the menu's "(&F)" mnemonic and trailing "...".
            let name = plain_label(&self.cmd_text(c));
            if ui.add_enabled_ui(enabled, |ui| form::icon_button(ui, btn, &tip, &name)).inner.clicked() {
                self.pending.push(c);
            }
        }
    }

    /// The text / column / hex edit modes as one three-way choice, so the toolbar shows which
    /// one is in use instead of three unrelated buttons.
    fn edit_mode_group(&mut self, ui: &mut egui::Ui) {
        let mode = self.doc().mode;
        let choices = [(C::TextMode, EditMode::Text, "textmode"), (C::ColumnMode, EditMode::Column, "columnmode"), (C::HexMode, EditMode::Hex, "hexmode")];
        let mut items = Vec::with_capacity(choices.len());
        for &(c, m, icon) in &choices {
            let Some((light, dark)) = self.icons.get(icon) else { return };
            let tex = if ui.visuals().dark_mode { dark } else { light };
            let sc = self.shortcut_text(c, ui.ctx());
            let tip = if sc.is_empty() { self.cmd_text(c) } else { format!("{} ({sc})", self.cmd_text(c)) };
            items.push(form::IconChoice {
                image: egui::Image::new((tex.id(), egui::vec2(16.0, 16.0))),
                tip,
                name: plain_label(&self.cmd_text(c)),
                selected: mode == m,
            });
        }
        if let Some(index) = form::icon_choices(ui, &items) {
            self.pending.push(choices[index].0);
        }
    }
}
