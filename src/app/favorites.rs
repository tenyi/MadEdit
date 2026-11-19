//! 左側「文件視圖」面板：頁籤 → 收藏樹與最近開啟的檔案。
//!
//! The panel is a thin view over `Settings::favorites`; every change is written to the settings
//! file at once, like the other View preferences. Only the 「列表」 tab exists for now; adding
//! another tab (for example the open files) means adding a `FileViewTab` variant, its label and
//! a drawing branch in `file_view_panel`.

use std::path::PathBuf;

use eframe::egui::{self, RichText};

use super::form;
use super::{menus, App};
use crate::core::favorites::FavoriteGroup;

/// Number of recently opened files listed under the favorites tree.
const RECENT_LIMIT: usize = 15;

/// Cap for "add the files in this folder": a folder such as node_modules must not turn into
/// thousands of favorites in one click.
const FOLDER_LIMIT: usize = 500;

pub(crate) const ROW_HEIGHT: f32 = 24.0;
const GROUP_INDENT: f32 = 12.0;
const ITEM_INDENT: f32 = 30.0;
const RECENT_ITEM_INDENT: f32 = 18.0;

/// 文件視圖的頁籤。新增頁籤時在這裡加一個變體，補上 `label` 與 `file_view_panel` 的繪製分支，
/// 並把它加進 `ALL` 即可。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FileViewTab {
    List,
    Open,
    Explorer,
}

impl FileViewTab {
    const ALL: [FileViewTab; 3] = [FileViewTab::List, FileViewTab::Open, FileViewTab::Explorer];

    fn label(self, app: &App) -> &'static str {
        match self {
            FileViewTab::List => app.t("view.tab_list", "列表"),
            FileViewTab::Open => app.t("view.tab_open", "開啟"),
            FileViewTab::Explorer => app.t("view.tab_explorer", "資源管理器"),
        }
    }
}

/// What a rename dialog acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FavTarget {
    NewGroup,
    Group(usize),
    Item(usize, usize),
}

/// A panel interaction, applied after drawing so no settings borrow is held while the lists
/// are painted.
enum FavAction {
    ToggleRoot,
    ToggleGroup(usize),
    OpenGroup(usize),
    OpenFavorite(usize, usize),
    OpenRecent(PathBuf),
    /// Close an editor tab (or its Markdown preview page) listed in the 「開啟」 view.
    CloseView(usize, bool),
    ActivateView(usize, bool),
    NewGroup,
    RenameGroup(usize),
    DeleteGroup(usize),
    AddFiles(Option<usize>),
    AddFolder(Option<usize>),
    AddActive(Option<usize>),
    AddOpen(Option<usize>),
    RenameItem(usize, usize),
    DeleteItem(usize, usize),
    ToggleShowNames,
    ToggleRecent,
}

impl App {
    pub(super) fn file_view_panel(&mut self, ui: &mut egui::Ui) {
        let mut action = None;
        // The panel frame has no margins: the tab strip paints itself edge to edge, and the
        // content below is inset here, so the selected tab can merge into the list.
        let full = ui.max_rect();
        let visuals = ui.visuals().clone();
        ui.painter().rect_filled(full, 0, visuals.panel_fill);
        let strip = egui::Rect::from_min_size(full.min, egui::vec2(full.width(), form::TAB_STRIP_HEIGHT));
        ui.painter().rect_filled(strip, 0, form::tab_strip_bg(ui));

        let tab_rect = egui::Rect::from_min_max(
            egui::pos2(full.left() + 8.0, strip.top() + 2.0),
            egui::pos2(full.right() - 8.0, strip.bottom()),
        );
        let mut tab_ui = ui.new_child(
            egui::UiBuilder::new().id_salt("file-view-tabs").max_rect(tab_rect).layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        tab_ui.spacing_mut().item_spacing.x = 0.0;
        let mut active_rect = None;
        for &tab in &FileViewTab::ALL {
            let selected = self.file_view_tab == tab;
            let item = form::flat_tab(&mut tab_ui, ("file-view-tab", tab as u8), tab.label(self), selected, visuals.panel_fill, None);
            if item.body.clicked() {
                self.file_view_tab = tab;
            }
            if selected {
                active_rect = Some(item.rect);
            }
        }
        form::tab_strip_baseline(ui, strip, active_rect);

        let content = egui::Rect::from_min_max(
            egui::pos2(full.left() + 8.0, strip.bottom() + 2.0),
            egui::pos2(full.right() - 8.0, full.bottom() - 2.0),
        );
        let mut list_ui = ui.new_child(
            egui::UiBuilder::new().id_salt("file-view-content").max_rect(content).layout(egui::Layout::top_down(egui::Align::LEFT)),
        );
        match self.file_view_tab {
            FileViewTab::List => self.file_view_list(&mut list_ui, &mut action),
            FileViewTab::Open => self.file_view_open_tabs(&mut list_ui, &mut action),
            FileViewTab::Explorer => self.explorer_panel(&mut list_ui),
        }
        ui.advance_cursor_after_rect(full);
        if let Some(action) = action {
            self.apply_favorites_action(action);
        }
    }

    fn file_view_list(&mut self, ui: &mut egui::Ui, action: &mut Option<FavAction>) {
        let names_only = self.settings.favorites_show_names_only;
        let selected = self.fav_selected;
        let root_open = self.fav_root_open;
        let recent_open = self.settings.favorites_recent_expanded;
        egui::ScrollArea::vertical().id_salt("file-view-list").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;

            // 一級節點「收藏」；群組是它的子節點。
            let root = self.t("fav.title", "收藏");
            let resp = group_row(ui, root, 0, root_open, false, 0.0, &RowIcon::Folder, None);
            if resp.clicked() {
                *action = Some(FavAction::ToggleRoot);
            }
            let resp = resp.on_hover_text(self.t("fav.root_hint", "收藏群組與檔案；在節點上按右鍵新增"));
            resp.context_menu(|ui| {
                ui.set_min_width(menus::SUBMENU_MIN_WIDTH);
                add_menu_items(ui, self, None, action);
                ui.separator();
                show_names_item(ui, self, action);
            });

            if root_open {
                if self.settings.favorites.is_empty() {
                    ui.add_space(4.0);
                    let hint = self.t("fav.empty", "還沒有收藏。在此按右鍵即可新增群組。");
                    ui.horizontal(|ui| {
                        ui.add_space(GROUP_INDENT + 6.0);
                        ui.add(egui::Label::new(RichText::new(hint).weak()).wrap());
                    });
                }
                for gi in 0..self.settings.favorites.len() {
                    let (name, expanded, count) = {
                        let group = &self.settings.favorites[gi];
                        (group.name.clone(), group.expanded, group.items.len())
                    };
                    let resp = group_row(ui, &name, count, expanded, selected == Some(gi), GROUP_INDENT, &RowIcon::Folder, None);
                    if resp.double_clicked() {
                        *action = Some(FavAction::OpenGroup(gi));
                    } else if resp.clicked() {
                        *action = Some(FavAction::ToggleGroup(gi));
                    }
                    let resp = resp.on_hover_text(self.t("fav.group_hint", "按兩下開啟群組中的所有檔案（取代目前開啟的分頁）"));
                    resp.context_menu(|ui| {
                        ui.set_min_width(menus::SUBMENU_MIN_WIDTH);
                        if ui.button(menus::check_label(false, self.t("fav.open_group", "開啟群組中的所有檔案（取代目前開啟的分頁）"))).clicked() {
                            *action = Some(FavAction::OpenGroup(gi));
                            ui.close();
                        }
                        ui.separator();
                        add_menu_items(ui, self, Some(gi), action);
                        ui.separator();
                        show_names_item(ui, self, action);
                        ui.separator();
                        if ui.button(menus::check_label(false, self.t("fav.rename_group", "重新命名群組…"))).clicked() {
                            *action = Some(FavAction::RenameGroup(gi));
                            ui.close();
                        }
                        if ui.button(menus::check_label(false, self.t("fav.delete_group", "刪除群組"))).clicked() {
                            *action = Some(FavAction::DeleteGroup(gi));
                            ui.close();
                        }
                    });

                    if expanded {
                        for ii in 0..count {
                            let (label, path) = {
                                let item = &self.settings.favorites[gi].items[ii];
                                (item.label(names_only), item.path.clone())
                            };
                            let (resp, _) = item_row(ui, &label, ITEM_INDENT, false, false, "", &RowIcon::File, None);
                            let resp = resp.on_hover_text(path.display().to_string());
                            if resp.clicked() {
                                *action = Some(FavAction::OpenFavorite(gi, ii));
                            }
                            resp.context_menu(|ui| {
                                ui.set_min_width(menus::SUBMENU_MIN_WIDTH);
                                if ui.button(menus::check_label(false, self.t("fav.open", "開啟"))).clicked() {
                                    *action = Some(FavAction::OpenFavorite(gi, ii));
                                    ui.close();
                                }
                                ui.separator();
                                if ui.button(menus::check_label(false, self.t("fav.rename_item", "重新命名檔案…"))).clicked() {
                                    *action = Some(FavAction::RenameItem(gi, ii));
                                    ui.close();
                                }
                                if ui.button(menus::check_label(false, self.t("fav.delete_item", "刪除選定項目"))).clicked() {
                                    *action = Some(FavAction::DeleteItem(gi, ii));
                                    ui.close();
                                }
                            });
                        }
                    }
                }
            }

            // 最近開啟的檔案是與「收藏」並列的另一個一級節點。
            ui.add_space(10.0);
            let recent_len = self.settings.recent_files.len();
            let resp = group_row(ui, self.t("fav.recent", "最近開啟的檔案"), recent_len, recent_open, false, 0.0, &RowIcon::Folder, None);
            if resp.clicked() {
                *action = Some(FavAction::ToggleRecent);
            }
            if recent_open {
                if recent_len == 0 {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.add_space(RECENT_ITEM_INDENT);
                        ui.add(egui::Label::new(RichText::new(self.t("fav.none", "（無）")).weak()).wrap());
                    });
                }
                for path in self.settings.recent_files.iter().take(RECENT_LIMIT) {
                    let label = if names_only {
                        path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
                    } else {
                        path.display().to_string()
                    };
                    let (resp, _) = item_row(ui, &label, RECENT_ITEM_INDENT, false, false, "", &RowIcon::File, None);
                    let resp = resp.on_hover_text(path.display().to_string());
                    if resp.clicked() {
                        *action = Some(FavAction::OpenRecent(path.clone()));
                    }
                }
            }
            ui.add_space(4.0);
        });
    }

    /// 「開啟」頁籤：目前開啟的所有編輯器分頁（含 Markdown 預覽分頁），點一下切換。
    fn file_view_open_tabs(&mut self, ui: &mut egui::Ui, action: &mut Option<FavAction>) {
        let rows = self.tab_pages();
        let active = self.active;
        let preview_active = self.is_preview_tab();
        let unsaved = self.t("st.unsaved", "尚未儲存").to_string();
        let preview_title = self.t("md.title", "Markdown 預覽").to_string();
        let no_files = self.t("st.no_document", "沒有開啟的檔案");
        let close_label = self.t("fav.close_tab", "關閉此分頁").to_string();
        egui::ScrollArea::vertical().id_salt("file-view-open-tabs").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            if rows.is_empty() {
                ui.add_space(8.0);
                ui.add(egui::Label::new(RichText::new(no_files).weak()).wrap());
            }
            for (i, preview) in rows {
                let tab = &self.tabs[i];
                let mut label = if preview {
                    format!("{} — {preview_title}", tab.doc.display_name())
                } else {
                    tab.doc.display_name()
                };
                if !preview {
                    if tab.doc.is_modified() {
                        label.push_str(" *");
                    }
                    if tab.doc.read_only {
                        label.push_str(" 🔒");
                    }
                }
                let tooltip = tab.doc.path.as_ref().map(|p| crate::core::display_path(p)).unwrap_or_else(|| unsaved.clone());
                let selected = i == active && preview == preview_active;
                let (resp, close) = item_row(ui, &label, 6.0, selected, true, &close_label, &RowIcon::None, None);
                let resp = resp.on_hover_text(tooltip);
                if resp.clicked() {
                    *action = Some(FavAction::ActivateView(i, preview));
                }
                // 雙擊等同於雙擊編輯器分頁標題：關閉該分頁（預覽列則關閉預覽）。
                if resp.double_clicked() {
                    *action = Some(FavAction::CloseView(i, preview));
                }
                if close.map_or(false, |c| c.clicked()) {
                    *action = Some(FavAction::CloseView(i, preview));
                }
            }
        });
    }

    fn apply_favorites_action(&mut self, action: FavAction) {
        match action {
            FavAction::ToggleRoot => self.fav_root_open = !self.fav_root_open,
            FavAction::ToggleGroup(gi) => {
                if let Some(group) = self.settings.favorites.get_mut(gi) {
                    group.expanded = !group.expanded;
                    self.fav_selected = Some(gi);
                    self.save_settings();
                }
            }
            FavAction::OpenGroup(gi) => self.open_favorite_group(gi),
            FavAction::OpenFavorite(gi, ii) => {
                let path = self.settings.favorites.get(gi).and_then(|g| g.items.get(ii)).map(|item| item.path.clone());
                if let Some(path) = path {
                    self.fav_selected = Some(gi);
                    self.open_path(&path);
                }
            }
            FavAction::OpenRecent(path) => {
                self.open_path(&path);
            }
            FavAction::CloseView(i, preview) => {
                if i < self.tabs.len() {
                    if preview {
                        let was_active = self.active == i && self.is_preview_tab();
                        self.tabs[i].md_preview = None;
                        if was_active {
                            self.preview_tab_active = false;
                            self.focus_editor = true;
                        }
                    } else {
                        let id = self.tabs[i].doc.id;
                        self.request_close(vec![id], false);
                    }
                }
            }
            FavAction::ActivateView(i, preview) => self.activate_view(i, preview),
            FavAction::NewGroup => self.open_favorite_name_dialog(FavTarget::NewGroup),
            FavAction::RenameGroup(gi) => self.open_favorite_name_dialog(FavTarget::Group(gi)),
            FavAction::RenameItem(gi, ii) => self.open_favorite_name_dialog(FavTarget::Item(gi, ii)),
            FavAction::DeleteGroup(gi) => {
                if gi < self.settings.favorites.len() {
                    self.settings.favorites.remove(gi);
                    self.fav_selected = match self.fav_selected {
                        Some(s) if s > gi => Some(s - 1),
                        Some(s) if s == gi => None,
                        other => other,
                    };
                    self.save_settings();
                }
            }
            FavAction::DeleteItem(gi, ii) => {
                if let Some(group) = self.settings.favorites.get_mut(gi)
                    && group.remove(ii)
                {
                    self.save_settings();
                }
            }
            FavAction::AddActive(target) => self.add_active_to_favorites(target),
            FavAction::AddOpen(target) => {
                let paths: Vec<PathBuf> = self.tabs.iter().filter_map(|tab| tab.doc.path.clone()).collect();
                if paths.is_empty() {
                    self.set_status(self.t("fav.no_paths", "沒有已開啟且已儲存的檔案可供加入"));
                    return;
                }
                let added = self.add_favorite_paths(target, paths);
                self.report_favorites_added(added);
            }
            FavAction::AddFiles(target) => {
                let title = self.t("fav.add_file", "新增檔案…").to_string();
                if let Some(files) = rfd::FileDialog::new().set_title(&title).pick_files() {
                    let added = self.add_favorite_paths(target, files);
                    self.report_favorites_added(added);
                }
            }
            FavAction::AddFolder(target) => self.add_folder_to_favorites(target),
            FavAction::ToggleShowNames => {
                self.settings.favorites_show_names_only = !self.settings.favorites_show_names_only;
                self.save_settings();
            }
            FavAction::ToggleRecent => {
                self.settings.favorites_recent_expanded = !self.settings.favorites_recent_expanded;
                self.save_settings();
            }
        }
    }

    /// Adds the active document (when it has a path) to a favorites group.
    pub(super) fn add_active_to_favorites(&mut self, target: Option<usize>) {
        let Some(path) = self.doc().path.clone() else {
            self.set_status(self.t("fav.no_active", "使用中的檔案尚未儲存，無法加入收藏"));
            return;
        };
        let added = self.add_favorite_paths(target, [path]);
        self.report_favorites_added(added);
    }

    fn report_favorites_added(&mut self, added: usize) {
        let msg = if added == 0 {
            self.t("fav.added_none", "沒有新增任何檔案（可能已在收藏中）").to_string()
        } else {
            self.tf("fav.added", "已加入 {n} 個檔案", Some(added), None)
        };
        self.set_status(msg);
    }

    fn open_favorite_name_dialog(&mut self, target: FavTarget) {
        let value = match target {
            FavTarget::Group(gi) => self.settings.favorites.get(gi).map(|g| g.name.clone()).unwrap_or_default(),
            FavTarget::Item(gi, ii) => self
                .settings
                .favorites
                .get(gi)
                .and_then(|g| g.items.get(ii))
                .and_then(|item| item.name.clone())
                .unwrap_or_default(),
            FavTarget::NewGroup => String::new(),
        };
        self.dialog = Some(super::dialogs::Dialog::FavoriteName { target, value, invalid: false });
    }

    /// Applies a name from the rename dialog. Group names must not be blank (the dialog checks
    /// that); an empty item name restores the default file name / path display.
    pub(super) fn apply_favorite_name(&mut self, target: FavTarget, name: &str) {
        let name = name.trim();
        if name.is_empty() && !matches!(target, FavTarget::Item(..)) {
            return;
        }
        match target {
            FavTarget::NewGroup => {
                self.settings.favorites.push(FavoriteGroup::new(name));
                self.fav_selected = Some(self.settings.favorites.len() - 1);
            }
            FavTarget::Group(gi) => {
                if let Some(group) = self.settings.favorites.get_mut(gi) {
                    group.rename(name);
                }
            }
            FavTarget::Item(gi, ii) => {
                if let Some(item) = self.settings.favorites.get_mut(gi).and_then(|g| g.items.get_mut(ii)) {
                    item.rename(name);
                }
            }
        }
        self.save_settings();
    }

    /// Target group of an "add" action: the clicked group, else the selected one, else a new
    /// default group. Creating it only on demand keeps a first run free of empty groups.
    fn favorite_target_group(&mut self, target: Option<usize>) -> usize {
        if let Some(gi) = target.filter(|&i| i < self.settings.favorites.len()) {
            return gi;
        }
        if let Some(gi) = self.fav_selected.filter(|&i| i < self.settings.favorites.len()) {
            return gi;
        }
        let name = self.t("fav.default_group", "常用").to_string();
        self.settings.favorites.push(FavoriteGroup::new(name));
        self.settings.favorites.len() - 1
    }

    /// Adds `paths` to the target group, skipping duplicates; returns how many were added.
    pub(super) fn add_favorite_paths(&mut self, target: Option<usize>, paths: impl IntoIterator<Item = PathBuf>) -> usize {
        let gi = self.favorite_target_group(target);
        let group = &mut self.settings.favorites[gi];
        group.expanded = true;
        self.fav_root_open = true;
        let mut added = 0;
        for path in paths {
            if group.add(path) {
                added += 1;
            }
        }
        self.fav_selected = Some(gi);
        self.save_settings();
        added
    }

    fn add_folder_to_favorites(&mut self, target: Option<usize>) {
        let title = self.t("fav.add_folder", "新增資料夾中的檔案…").to_string();
        let Some(dir) = rfd::FileDialog::new().set_title(&title).pick_folder() else {
            return;
        };
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) => {
                let title = self.t("fav.folder_failed", "無法讀取資料夾").to_string();
                self.message(&title, e.to_string());
                return;
            }
        };
        let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_file()).collect();
        let total = paths.len();
        paths.sort_by_cached_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default());
        paths.truncate(FOLDER_LIMIT);
        let added = self.add_favorite_paths(target, paths);
        let msg = if total == 0 {
            self.t("fav.added_none", "沒有新增任何檔案（可能已在收藏中）").to_string()
        } else if total > FOLDER_LIMIT {
            self.tf("fav.added_folder_capped", "已加入前 {n} 個檔案（資料夾檔案過多）", Some(added), None)
        } else {
            self.tf("fav.added_folder", "已從資料夾加入 {n} 個檔案", Some(added), None)
        };
        self.set_status(msg);
    }

    pub(super) fn open_favorite_group(&mut self, gi: usize) {
        let paths: Option<Vec<PathBuf>> = self.settings.favorites.get(gi).map(|g| g.items.iter().map(|item| item.path.clone()).collect());
        let Some(paths) = paths else {
            return;
        };
        if paths.is_empty() {
            self.set_status(self.t("fav.empty_group", "群組中沒有檔案"));
            return;
        }
        self.fav_selected = Some(gi);
        // 取代開啟：先關閉目前所有分頁（有未存檔修改走既有的儲存確認流程），
        // 流程結束後由 pending_open_group 接續開啟群組；使用者取消則不開啟。
        self.pending_open_group = Some(gi);
        let ids: Vec<u64> = self.tabs.iter().map(|t| t.doc.id).collect();
        self.request_close(ids, false);
        if self.confirm.is_none() && self.pending_close.is_none() {
            self.pending_open_group = None;
            self.open_group_now(gi);
        }
    }

    /// 實際開啟群組中的所有檔案（取代開啟流程的第二步）
    pub(super) fn open_group_now(&mut self, gi: usize) {
        let paths: Option<Vec<PathBuf>> = self.settings.favorites.get(gi).map(|g| g.items.iter().map(|item| item.path.clone()).collect());
        let Some(paths) = paths else {
            return;
        };
        self.fav_selected = Some(gi);
        let (mut opened, mut missing) = (0, 0);
        for path in paths {
            if !path.exists() {
                missing += 1;
            } else if self.open_path(&path) {
                opened += 1;
            }
        }
        let msg = if missing == 0 {
            self.tf("fav.opened", "已開啟 {n} 個檔案", Some(opened), None)
        } else {
            let missing = missing.to_string();
            self.tf("fav.opened_missing", "已開啟 {n} 個檔案，{q} 個檔案不存在", Some(opened), Some(&missing))
        };
        self.set_status(msg);
    }
}

/// The "add …" entries shared by the favorites root and group context menus.
fn add_menu_items(ui: &mut egui::Ui, app: &App, target: Option<usize>, action: &mut Option<FavAction>) {
    for (label, act) in [
        (app.t("fav.add_group", "新增群組…"), FavAction::NewGroup),
        (app.t("fav.add_folder", "新增資料夾中的檔案…"), FavAction::AddFolder(target)),
        (app.t("fav.add_file", "新增檔案…"), FavAction::AddFiles(target)),
        (app.t("fav.add_active", "新增使用中的檔案"), FavAction::AddActive(target)),
        (app.t("fav.add_open", "新增所有開啟的檔案"), FavAction::AddOpen(target)),
    ] {
        if ui.button(menus::check_label(false, label)).clicked() {
            *action = Some(act);
            ui.close();
        }
    }
}

fn show_names_item(ui: &mut egui::Ui, app: &App, action: &mut Option<FavAction>) {
    if ui.button(menus::check_label(app.settings.favorites_show_names_only, app.t("fav.show_names", "僅顯示名稱"))).clicked() {
        *action = Some(FavAction::ToggleShowNames);
        ui.close();
    }
}

/// 樹／列表列的裝飾圖示。`Tex` 攜帶 Windows Shell 系統圖示貼圖；
/// 其餘為向量繪製的備援（非 Windows 平台或 Shell 取圖失敗時使用）。
#[derive(Clone, Copy)]
pub(crate) enum RowIcon<'a> {
    None,
    Folder,
    File,
    Drive,
    Tex(&'a egui::TextureHandle),
}

/// 在 `center`（圖示中心）繪製圖示，回傳佔用的寬度（含右側間距）。
pub(crate) fn paint_row_icon(ui: &mut egui::Ui, center: egui::Pos2, kind: &RowIcon<'_>) -> f32 {
    const SLOT: f32 = 18.0;
    let painter = ui.painter();
    if let RowIcon::Tex(tex) = kind {
        // Shell 系統圖示貼圖（32px 來源）繪製在 16pt 槽位，高 DPI 清晰。
        let rect = egui::Rect::from_center_size(center, egui::vec2(16.0, 16.0));
        painter.image(
            tex.id(),
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        return SLOT + 2.0;
    }
    if matches!(kind, RowIcon::None) {
        return 0.0;
    }
    let (x, y) = (center.x - SLOT / 2.0, center.y);
    match kind {
        RowIcon::None => unreachable!(),
        RowIcon::Tex(_) => unreachable!(),
        RowIcon::Folder => {
            // 仿 Windows 檔案總管黃色資料夾：深色標籤片 + 淺色本體 + 描邊
            let tab = egui::Rect::from_min_size(egui::pos2(x, y - 5.0), egui::vec2(7.0, 4.0));
            let body = egui::Rect::from_min_size(egui::pos2(x, y - 3.0), egui::vec2(15.0, 10.0));
            painter.rect_filled(tab, 1.5, egui::Color32::from_rgb(224, 164, 20));
            painter.rect_filled(body, 2.0, egui::Color32::from_rgb(255, 213, 79));
            painter.rect_stroke(body, 2.0, egui::Stroke::new(1.0, egui::Color32::from_rgb(197, 144, 12)), egui::StrokeKind::Outside);
        }
        RowIcon::File => {
            // 白色頁面 + 摺角 + 灰描邊
            let sheet = egui::Rect::from_min_size(egui::pos2(x + 2.0, y - 6.0), egui::vec2(11.0, 13.0));
            painter.rect_filled(sheet, 1.0, egui::Color32::from_rgb(252, 252, 252));
            painter.rect_stroke(sheet, 1.0, egui::Stroke::new(1.0, egui::Color32::from_rgb(150, 150, 155)), egui::StrokeKind::Outside);
            let fold = [
                egui::pos2(sheet.right() - 4.0, sheet.top()),
                egui::pos2(sheet.right(), sheet.top() + 4.0),
                egui::pos2(sheet.right() - 4.0, sheet.top() + 4.0),
            ];
            painter.add(egui::Shape::convex_polygon(
                vec![fold[0], fold[1], fold[2]],
                egui::Color32::from_rgb(215, 218, 224),
                egui::Stroke::new(1.0, egui::Color32::from_rgb(150, 150, 155)),
            ));
            for i in 0..3 {
                let ly = sheet.top() + 6.0 + i as f32 * 2.5;
                painter.line_segment([egui::pos2(sheet.left() + 2.5, ly), egui::pos2(sheet.right() - 4.5, ly)], egui::Stroke::new(1.0, egui::Color32::from_rgb(190, 195, 205)));
            }
        }
        RowIcon::Drive => {
            let body = egui::Rect::from_min_size(egui::pos2(x, y - 5.0), egui::vec2(15.0, 10.0));
            painter.rect_filled(body, 2.0, egui::Color32::from_rgb(225, 228, 235));
            painter.rect_stroke(body, 2.0, egui::Stroke::new(1.0, egui::Color32::from_rgb(140, 145, 155)), egui::StrokeKind::Outside);
            painter.line_segment([egui::pos2(body.left() + 2.0, body.bottom() - 2.5), egui::pos2(body.right() - 6.0, body.bottom() - 2.5)], egui::Stroke::new(1.5, egui::Color32::from_rgb(160, 165, 175)));
            painter.circle_filled(egui::pos2(body.right() - 3.0, body.bottom() - 2.5), 1.2, egui::Color32::from_rgb(96, 176, 96));
        }
    }
    SLOT + 2.0
}

/// A tree heading: fold arrow, name, and the number of children dimmed at the right.
/// Reused by the Explorer tab for its directory rows.
/// `width`: 列的最小內容寬度（None = 填滿可用寬度）。橫向捲動的樹／列表傳入自然寬度，
/// 讓長路徑撐開內容並出現橫向捲軸。
pub(crate) fn group_row(ui: &mut egui::Ui, name: &str, count: usize, expanded: bool, selected: bool, indent: f32, icon: &RowIcon<'_>, width: Option<f32>) -> egui::Response {
    let row_w = width.unwrap_or(ui.available_width()).max(ui.available_width());
    let (rect, response) = ui.allocate_exact_size(egui::vec2(row_w, ROW_HEIGHT), egui::Sense::click());
    // 無障礙資訊在裁剪與否都註冊（樹很長時列會被 ScrollArea 裁剪，但節點仍須可查詢）。
    response.widget_info(|| egui::WidgetInfo::selected(egui::accesskit::Role::Button, ui.is_enabled(), selected, name));
    if ui.is_rect_visible(rect) {
        let weak = ui.visuals().weak_text_color();
        let strong = ui.visuals().strong_text_color();
        let hover_fill = ui.visuals().widgets.hovered.weak_bg_fill;
        let sel_fill = ui.visuals().selection.bg_fill.gamma_multiply(0.35);
        let painter = ui.painter().clone();
        if selected {
            painter.rect_filled(rect, 4.0, sel_fill);
        } else if response.hovered() {
            painter.rect_filled(rect, 4.0, hover_fill);
        }
        let y = rect.center().y;
        let arrow = painter.text(
            egui::pos2(rect.left() + indent + 6.0, y),
            egui::Align2::LEFT_CENTER,
            if expanded { "▾" } else { "▸" },
            egui::FontId::proportional(12.0),
            weak,
        );
        // 圖示在箭頭與名稱之間（None 時佔 0 寬，版面與原一致）
        let icon_w = paint_row_icon(ui, egui::pos2(arrow.right() + 11.0, y), icon);
        let x = arrow.right() + 4.0 + icon_w;
        let count_galley = (count > 0).then(|| painter.layout_no_wrap(count.to_string(), egui::FontId::proportional(11.0), weak));
        let right = count_galley.as_ref().map_or(rect.right() - 6.0, |g| rect.right() - 6.0 - g.size().x - 8.0);
        let body = egui::TextStyle::Body.resolve(ui.style());
        let mut job = egui::text::LayoutJob { wrap: egui::text::TextWrapping::truncate_at_width((right - x).max(0.0)), ..Default::default() };
        job.append(name, 0.0, egui::TextFormat::simple(body, strong));
        let galley = painter.layout_job(job);
        painter.galley(egui::pos2(x, y - galley.size().y / 2.0), galley, strong);
        if let Some(count) = count_galley {
            painter.galley(egui::pos2(rect.right() - 6.0 - count.size().x, y - count.size().y / 2.0), count, weak);
        }
    }
    response
}

/// A file row: indented under its group, truncated with the full path on hover.
/// When `closable`, a "×" close button appears at the right end on hover or selection;
/// it owns its own rectangle (never overlapping the row hit rect) with a derived Id
/// so the row click and the close click never fight over the same widget state.
pub(crate) fn item_row(ui: &mut egui::Ui, label: &str, indent: f32, selected: bool, closable: bool, close_label: &str, icon: &RowIcon<'_>, width: Option<f32>) -> (egui::Response, Option<egui::Response>) {
    const CLOSE_WIDTH: f32 = 20.0;
    let row_w = width.unwrap_or(ui.available_width()).max(ui.available_width());
    let (rect, response) = ui.allocate_exact_size(egui::vec2(row_w, ROW_HEIGHT), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::accesskit::Role::Button, ui.is_enabled(), selected, label));
    let mut close = None;
    // Pointer-in-rect instead of response.hovered(): the close button's interact rect sits
    // on top of the row inside it, which would steal "hovered" and make the button flicker.
    let row_hovered = ui.rect_contains_pointer(rect);
    if ui.is_rect_visible(rect) {
        let sel_fill = ui.visuals().selection.bg_fill.gamma_multiply(0.35);
        let hover_fill = ui.visuals().widgets.hovered.weak_bg_fill;
        let weak = ui.visuals().weak_text_color();
        let painter = ui.painter().clone();
        if selected {
            painter.rect_filled(rect, 4.0, sel_fill);
        } else if row_hovered {
            painter.rect_filled(rect, 4.0, hover_fill);
        }
        let show_close = closable && (row_hovered || selected);
        let x_w = if closable { CLOSE_WIDTH } else { 0.0 };
        let body = egui::TextStyle::Body.resolve(ui.style());
        let color = if selected { ui.visuals().strong_text_color() } else { ui.visuals().text_color() };
        // 圖示在文字左側（None 時佔 0 寬）
        let icon_w = paint_row_icon(ui, egui::pos2(rect.left() + indent + 6.0 + 9.0, rect.center().y), icon);
        let x = rect.left() + indent + 6.0 + icon_w;
        let mut layout = egui::text::LayoutJob { wrap: egui::text::TextWrapping::truncate_at_width((rect.right() - 6.0 - x_w - x).max(0.0)), ..Default::default() };
        layout.append(label, 0.0, egui::TextFormat::simple(body, color));
        let galley = painter.layout_job(layout);
        painter.galley(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley, color);
        if show_close {
            // Independent interact rect at the right end; Id derived from the row Id.
            let x_rect = egui::Rect::from_min_size(
                egui::pos2(rect.right() - 4.0 - CLOSE_WIDTH, rect.top()),
                egui::vec2(CLOSE_WIDTH, ROW_HEIGHT),
            );
            let resp = ui.interact(x_rect, response.id.with("close"), egui::Sense::click());
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, close_label));
            resp.clone().on_hover_text(close_label);
            // No button chrome of its own: only the glyph turns red on hover, like the
            // editor tab's close button.
            let tint = if resp.hovered() { egui::Color32::from_rgb(232, 17, 35) } else { weak };
            ui.painter().text(x_rect.center(), egui::Align2::CENTER_CENTER, "×", egui::FontId::proportional(13.0), tint);
            close = Some(resp);
        }
    }
    (response, close)
}
