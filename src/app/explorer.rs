//! 資源管理器頁籤：懶載入的目錄樹與檔名篩選。
//!
//! 只在節點第一次展開時才 `read_dir`，目錄內容按路徑快取，不做全盤掃描，
//! 大目錄也不阻塞界面。篩選語義（仿 UltraEdit，經用戶確認）：
//! - 篩選只對**目前選中資料夾**的檔名生效；子資料夾不受篩選。
//! - 分欄模式未開啟時（純樹形態），選中的樹節點刷新為「子資料夾 + 通過篩選的檔案」，
//!   檔案以葉節點形式出現在樹裡；無篩選時恢復純資料夾樹。
//! - 分欄模式開啟時，下半部列表顯示所選資料夾的內容（見階段 2）。
//!
//! 樹的走訪用顯式堆疊而非遞迴：egui 的 UI 閉包不能遞迴借用 `self`。
//! 子節點以反向壓疊實現，深度優先順序與遞迴樹一致。
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::favorites::{self, ROW_HEIGHT};
use super::App;

/// 每層樹節點的縮排步進。
const INDENT_STEP: f32 = 24.0;
/// 樹中檔案葉節點比同層資料夾再縮一點，視覺歸屬更明確。
const FILE_EXTRA_INDENT: f32 = 12.0;

/// 樹中的一筆目錄項（資料夾或檔案）。
#[derive(Clone)]
struct Entry {
    name: String,
    dir: bool,
}

/// 下半部列表的一筆資料（帶元資料；僅在分欄模式下讀取）。
#[derive(Clone)]
struct FileRow {
    name: String,
    dir: bool,
    /// 修改時間（Unix 秒）。
    modified: Option<i64>,
    /// 檔案大小（位元組；資料夾為 None）。
    size: Option<u64>,
}

/// 一次繪製週期內收集的使用者操作，畫完後統一套用（與 favorites 相同模式）。
enum Action {
    None,
    /// 選中資料夾（單擊，同時切換展開狀態）。
    Select(PathBuf),
    /// 進入資料夾（在下半部列表雙擊資料夾；樹選中同步跳轉並展開）。
    Enter(PathBuf),
    /// 打開檔案（單擊樹中的檔案葉節點，或下半部列表單擊檔案）。
    Open(PathBuf),
    /// 單擊資料夾列：樹選中並展開（不切換列表資料夾）。
    Focus(PathBuf),
    /// 表頭點擊：排序欄/方向已更新，需重繪並保存。
    Resort,
}

pub(crate) struct Explorer {
    /// 篩選輸入框的即時內容；按「>」或 Enter 後才寫入 `filter_applied` 生效。
    pub(crate) filter: String,
    pub(crate) filter_applied: String,
    /// ▣ 分欄模式（樹下方顯示所選資料夾內容列表；跨階段記住）。
    pub(crate) split: bool,
    /// 分欄模式下上半部樹的高度比例（拖拽分隔條後記住）。
    pub(crate) ratio: f32,
    /// 目前選中的資料夾。
    pub(crate) selected: Option<PathBuf>,
    /// 分欄列表排序欄：0=名稱 1=修改日期 2=類型 3=大小。
    pub(crate) sort_column: u8,
    /// 分欄列表是否降冪。
    pub(crate) sort_descending: bool,
    /// 展開中的資料夾。
    pub(crate) expanded: HashSet<PathBuf>,
    /// 讀取過的目錄內容快取；`None` = 讀取失敗（無權限、裝置不存在等）。
    listed: HashMap<PathBuf, Option<Vec<Entry>>>,
    /// 分欄模式下下半部的內容快取（帶元資料）；只在分欄開啟時讀取。
    meta: HashMap<PathBuf, Option<Vec<FileRow>>>,
    /// 磁碟機根清單，第一次繪製時探測一次。
    drives: Option<Vec<PathBuf>>,
}

impl Default for Explorer {
    fn default() -> Self {
        Self {
            filter: String::new(),
            filter_applied: String::new(),
            split: false,
            ratio: 0.5,
            selected: None,
            sort_column: 0,
            sort_descending: false,
            expanded: HashSet::new(),
            listed: HashMap::new(),
            meta: HashMap::new(),
            drives: None,
        }
    }
}

impl Explorer {
    /// 磁碟機（非 Windows 是根 `/`），只探測一次。
    fn roots(&mut self) -> Vec<PathBuf> {
        if self.drives.is_none() {
            #[cfg(windows)]
            {
                self.drives = Some(
                    (b'A'..=b'Z')
                        .filter_map(|c| {
                            let root = format!("{}:\\", char::from(c));
                            Path::new(&root).is_dir().then_some(root.into())
                        })
                        .collect(),
                );
            }
            #[cfg(not(windows))]
            {
                self.drives = Some(vec![PathBuf::from("/")]);
            }
        }
        self.drives.clone().unwrap_or_default()
    }

    /// 讀取並快取一個資料夾的內容（資料夾在前、名稱不分大小寫排序）。
    fn load_entries(&mut self, dir: &Path) {
        if self.listed.contains_key(dir) {
            return;
        }
        let read = std::fs::read_dir(dir).ok().map(|rd| {
            let mut out: Vec<Entry> = rd
                .filter_map(|e| e.ok())
                .map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    let dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    Entry { name, dir }
                })
                .collect();
            out.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
            out
        });
        self.listed.insert(dir.to_path_buf(), read);
    }

    /// 篩選詞目前是否生效。
    fn filtered(&self) -> bool {
        !self.filter_applied.is_empty()
    }

    /// 讀取並快取一個資料夾帶元資料的內容（僅分欄模式使用；資料夾在前、名稱升序）。
    fn load_meta(&mut self, dir: &Path) {
        if self.meta.contains_key(dir) {
            return;
        }
        let read = std::fs::read_dir(dir).ok().map(|rd| {
            let mut out: Vec<FileRow> = rd
                .filter_map(|e| e.ok())
                .map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    let meta = e.metadata().ok();
                    let dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
                    let modified = meta.as_ref().and_then(|m| m.modified().ok()).map(|t| {
                        t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
                    });
                    let size = (!dir).then(|| meta.as_ref().and_then(|m| m.len().into())).flatten();
                    FileRow { name, dir, modified, size }
                })
                .collect();
            out.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
            out
        });
        self.meta.insert(dir.to_path_buf(), read);
    }

    /// 套用篩選：按「>」或輸入框中按 Enter 時呼叫；空字串 = 清除篩選。
    fn apply_filter(&mut self) {
        self.filter_applied = self.filter.trim().to_string();
    }

    /// 把目前狀態寫回設定（跨階段記住）；由 App 在狀態變更後呼叫。
    fn save_to_settings(&self, settings: &mut crate::core::settings::Settings) {
        settings.explorer.split = self.split;
        settings.explorer.ratio = self.ratio;
        settings.explorer.filter = self.filter_applied.clone();
        settings.explorer.selected = self.selected.as_ref().map(|p| p.to_string_lossy().into_owned());
        settings.explorer.expanded = self.expanded.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        settings.explorer.sort_column = self.sort_column;
        settings.explorer.sort_descending = self.sort_descending;
    }

    /// 從設定還原狀態（啟動時呼叫一次）。
    pub(crate) fn restore_from_settings(&mut self, settings: &crate::core::settings::Settings) {
        self.split = settings.explorer.split;
        self.ratio = settings.explorer.ratio.clamp(0.15, 0.85);
        self.filter = settings.explorer.filter.clone();
        self.filter_applied = settings.explorer.filter.clone();
        self.selected = settings.explorer.selected.as_ref().map(PathBuf::from);
        self.expanded = settings.explorer.expanded.iter().map(PathBuf::from).collect();
        self.sort_column = settings.explorer.sort_column.min(3);
        self.sort_descending = settings.explorer.sort_descending;
    }
}

impl App {
    /// 「資源管理器」頁籤：篩選列 + 上半部目錄樹 + 分欄內容列表。
    pub(super) fn explorer_panel(&mut self, ui: &mut egui::Ui) {
        let mut action = Action::None;
        let mut state_changed = false;
        let filter_hint = self.t("ex.filter_hint", "輸入檔名篩選，如 *.txt");
        let apply_tip = self.t("ex.apply", "執行篩選").to_string();

        // 篩選列：文字框 + 「>」執行 + ▣ 分欄開關（仿 UltraEdit；按 > 或 Enter 才生效）。
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            let split_tip = self.t("ex.split", "分欄顯示資料夾內容").to_string();
            let edit = egui::TextEdit::singleline(&mut self.explorer.filter)
                .hint_text(filter_hint)
                .desired_width(ui.available_width() - 92.0);
            let resp = ui.add(edit);
            // 單行 TextEdit 按 Enter 會失去焦點，用 lost_focus 判定。
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let clicked = ui.button(">").on_hover_text(apply_tip.clone()).clicked();
            if enter || clicked {
                self.explorer.apply_filter();
                state_changed = true;
            }
            if ui.add(egui::Checkbox::new(&mut self.explorer.split, "")).on_hover_text(split_tip).changed() {
                state_changed = true;
                // 開啟分欄時確保已有選中資料夾（沒有則退回磁碟機根）。
                if self.explorer.split && self.explorer.selected.is_none() {
                    let roots = self.explorer.roots();
                    if let Some(first) = roots.first() {
                        self.explorer.selected = Some(first.clone());
                    }
                }
            }
        });
        ui.add_space(2.0);

        if !self.explorer.split {
            self.explorer_tree(ui, &mut action, None);
        } else {
            // 分欄：上半部樹 + 拖拽分隔條 + 下半部內容列表。
            let avail = ui.available_rect_before_wrap();
            let total = avail.height().max(240.0);
            let tree_h = (total * self.explorer.ratio).clamp(80.0, total - 110.0);
            self.explorer_tree(ui, &mut action, Some(tree_h));
            let split_tip = self.t("ex.split_tip", "拖曳調整上下高度").to_string();
            let (srect, sresp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 7.0), egui::Sense::click_and_drag());
            // 與左側停靠欄拖拽一致的調整游標風格（垂直方向）
            let sresp = sresp.on_hover_cursor(egui::CursorIcon::ResizeVertical);
            if sresp.dragged() {
                // 拖拽分隔條向下（dy>0）→ 樹變高；拖拽向上 → 樹變矮。
                let dy = sresp.drag_delta().y;
                let r = self.explorer.ratio + dy / total;
                let clamped = r.clamp(0.15, 0.85);
                if clamped != self.explorer.ratio {
                    self.explorer.ratio = clamped;
                    state_changed = true;
                }
            }
            if ui.is_rect_visible(srect) {
                let stroke = if sresp.hovered() || sresp.dragged() {
                    ui.visuals().widgets.hovered.fg_stroke
                } else {
                    ui.visuals().widgets.noninteractive.bg_stroke
                };
                ui.painter().line_segment([egui::pos2(srect.left(), srect.center().y), egui::pos2(srect.right(), srect.center().y)], stroke);
            }
            sresp.on_hover_text(split_tip);
            self.explorer_list(ui, &mut action);
        }

        // 狀態有變（選中位置、展開節點、篩選、分欄、比例、排序）→ 寫回設定即時保存。
        let moved_selection = matches!(action, Action::Select(_) | Action::Enter(_) | Action::Focus(_));
        match action {
            Action::None => {}
            Action::Select(p) => {
                // 單擊 = 選中並切換展開（與收藏樹的互動一致）。
                if self.explorer.expanded.remove(&p) {
                    // 收起
                } else {
                    self.explorer.expanded.insert(p.clone());
                    self.explorer.load_entries(&p);
                }
                self.explorer.selected = Some(p);
            }
            Action::Enter(p) => {
                // 進入資料夾：樹選中跳轉並保持展開。
                self.explorer.expanded.insert(p.clone());
                self.explorer.load_entries(&p);
                self.explorer.selected = Some(p);
            }
            Action::Open(p) => {
                self.open_path(&p);
            }
            Action::Focus(p) => {
                // 單擊列表資料夾：樹選中並展開，列表仍停在原資料夾。
                self.explorer.expanded.insert(p.clone());
                self.explorer.load_entries(&p);
                self.explorer.selected = Some(p);
            }
            Action::Resort => {
                // 排序欄/方向已寫入狀態，只需保存。
                state_changed = true;
            }
        }

        if state_changed || moved_selection {
            self.explorer.save_to_settings(&mut self.settings);
            self.save_settings();
        }
    }

    /// 上半部目錄樹；`max_height` 為 Some 時是分欄模式的上半部高度。
    fn explorer_tree(&mut self, ui: &mut egui::Ui, action: &mut Action, max_height: Option<f32>) {
        let unreadable = self.t("ex.unreadable", "（無法讀取）");
        let mut area = egui::ScrollArea::vertical().id_salt("ex-tree").auto_shrink([false, false]);
        if let Some(h) = max_height {
            area = area.max_height(h);
        }
        area.show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let mut stack: Vec<(PathBuf, f32)> = self.explorer.roots().into_iter().rev().map(|p| (p, 0.0)).collect();
            while let Some((path, indent)) = stack.pop() {
                // 只在展開時讀取目錄內容（懶載入）。
                if self.explorer.expanded.contains(&path) {
                    self.explorer.load_entries(&path);
                }
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string());
                let expanded = self.explorer.expanded.contains(&path);
                let selected = self.explorer.selected.as_deref() == Some(path.as_path());
                // Windows Shell 系統圖示（取得失敗時回退向量資料夾圖示）；根層（磁碟機）用硬碟圖示。
                let path_str = path.to_string_lossy().into_owned();
                let tex = self.shell_icons.get(ui.ctx(), &path_str, true);
                let icon = match &tex {
                    Some(t) => favorites::RowIcon::Tex(t),
                    None if indent == 0.0 => favorites::RowIcon::Drive,
                    None => favorites::RowIcon::Folder,
                };
                let resp = favorites::group_row(ui, &name, 0, expanded, selected, indent, &icon);
                if resp.clicked() {
                    *action = Action::Select(path.clone());
                }
                if !expanded {
                    continue;
                }
                let is_selected = self.explorer.selected.as_deref() == Some(path.as_path());
                let filtered = self.explorer.filtered();
                let mut dirs: Vec<PathBuf> = Vec::new();
                match self.explorer.listed.get(&path).and_then(|e| e.as_ref()) {
                    Some(list) => {
                        for e in list {
                            if e.dir {
                                dirs.push(path.join(&e.name));
                            }
                            // 選中資料夾掛出內容：
                            // - 純樹形態（分欄關閉）：子資料夾 + 全部檔案（有篩選時只顯示匹配項）；
                            // - 分欄形態：樹維持清單，僅在篩選生效時加掛匹配檔案（其餘由下方列表呈現）。
                            else if is_selected
                                && (!self.explorer.split || filtered)
                                && (!filtered || glob_match(&e.name, &self.explorer.filter_applied))
                            {
                                let leaf_indent = indent + INDENT_STEP + FILE_EXTRA_INDENT;
                                let leaf_path = path.join(&e.name);
                                let leaf_str = leaf_path.to_string_lossy().into_owned();
                                let tex = self.shell_icons.get(ui.ctx(), &leaf_str, false);
                                let icon = match &tex {
                                    Some(t) => favorites::RowIcon::Tex(t),
                                    None => favorites::RowIcon::File,
                                };
                                let (fresp, _) = favorites::item_row(ui, &e.name, leaf_indent, false, false, "", &icon);
                                if fresp.clicked() {
                                    *action = Action::Open(leaf_path);
                                }
                            }
                        }
                    }
                    None => {
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_HEIGHT), egui::Sense::hover());
                        ui.painter().text(
                            egui::pos2(rect.left() + indent + 18.0, rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            unreadable,
                            egui::FontId::proportional(12.0),
                            ui.visuals().weak_text_color(),
                        );
                    }
                }
                // 反向壓疊：pop 出來時保持原本的排序（資料夾在前、名稱升序）。
                for d in dirs.into_iter().rev() {
                    stack.push((d, indent + INDENT_STEP));
                }
            }
        });
    }

    /// 分欄模式的下半部：所選資料夾的內容列表（名稱／修改日期／類型／大小）。
    /// 單擊檔案開啟；雙擊資料夾進入（樹選中同步跳轉）。篩選只作用於檔名，子資料夾不受篩選。
    fn explorer_list(&mut self, ui: &mut egui::Ui, action: &mut Action) {
        let name_header = self.t("ex.name", "名稱");
        let date_header = self.t("ex.date", "修改日期");
        let type_header = self.t("ex.type", "類型");
        let size_header = self.t("ex.size", "大小");
        let folder_type = self.t("ex.folder", "資料夾");
        let file_type = self.t("ex.file", "檔案");
        let empty_dir = self.t("ex.empty", "（空）");
        let unreadable = self.t("ex.unreadable", "（無法讀取）");
        let Some(selected) = self.explorer.selected.clone() else {
            return;
        };
        if !self.explorer.meta.contains_key(&selected) {
            self.explorer.load_meta(&selected);
        }

        // 欄位寬度與表頭：表頭固定在 ScrollArea 之外，列表滾動時不隨之捲動。
        let full_w = ui.available_width();
        // 自適應欄位：面板窄時依次隱藏 大小 → 類型 → 修改日期，名稱欄永遠保留。
        let mut date_w = 115.0;
        let mut type_w = 70.0;
        let mut size_w = 70.0;
        let show_size = full_w >= 340.0;
        if !show_size {
            size_w = 0.0;
        }
        let show_type = show_size || full_w >= 280.0;
        if !show_type {
            type_w = 0.0;
        }
        let show_date = show_type || full_w >= 200.0;
        if !show_date {
            date_w = 0.0;
        }
        let name_w = (full_w - date_w - type_w - size_w).max(60.0);
        let weak = ui.visuals().weak_text_color();
        // 表頭：可點擊排序（四選一；同欄再點切換升降序），排序中的欄位畫 ▲/▼ 指示。
        let columns = [(0u8, name_w, name_header), (1, date_w, date_header), (2, type_w, type_header), (3, size_w, size_header)];
        let (hrect, hresp) = ui.allocate_exact_size(egui::vec2(full_w, ROW_HEIGHT), egui::Sense::click());
        hresp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, format!("{name_header}|{date_header}|{type_header}|{size_header}")));
        if hresp.clicked() {
            if let Some(px) = hresp.interact_pointer_pos() {
                let mut x = hrect.left() + 6.0;
                for (idx, w, _) in columns {
                    if w <= 0.0 {
                        continue;
                    }
                    if px.x >= x && px.x < x + w {
                        if self.explorer.sort_column == idx {
                            self.explorer.sort_descending = !self.explorer.sort_descending;
                        } else {
                            self.explorer.sort_column = idx;
                            self.explorer.sort_descending = false;
                        }
                        *action = Action::Resort;
                        break;
                    }
                    x += w;
                }
            }
        }
        let sort_color = ui.visuals().strong_text_color();
        let mut x = hrect.left() + 6.0;
        for (idx, w, title) in columns {
            if w <= 0.0 {
                continue;
            }
            let sorted = self.explorer.sort_column == idx;
            let tint = if sorted { sort_color } else { weak };
            let arrow = if sorted { if self.explorer.sort_descending { " ▼" } else { " ▲" } } else { "" };
            ui.painter().text(egui::pos2(x + 2.0, hrect.center().y), egui::Align2::LEFT_CENTER, format!("{title}{arrow}"), egui::FontId::proportional(12.0), tint);
            x += w;
        }

        egui::ScrollArea::vertical().id_salt("ex-list").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let rows = self.explorer.meta.get(&selected).and_then(|e| e.as_ref());
            // 排序：資料夾永遠在前；欄內依 sort_column 升降冪。副本排序，不動快取。
            let mut sorted: Vec<&FileRow> = rows.into_iter().flatten().collect();
            sorted.sort_by(|a, b| {
                let cmp = match self.explorer.sort_column {
                    1 => a.modified.cmp(&b.modified),
                    2 => {
                        let ka = if a.dir { folder_type.to_string() } else { std::path::Path::new(&a.name).extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_default() };
                        let kb = if b.dir { folder_type.to_string() } else { std::path::Path::new(&b.name).extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_default() };
                        ka.cmp(&kb)
                    }
                    3 => a.size.cmp(&b.size),
                    _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                };
                let primary = if self.explorer.sort_descending { cmp.reverse() } else { cmp };
                // 資料夾恆在檔案前（升降冪只影響各群組內的順序）。
                b.dir.cmp(&a.dir).then_with(|| primary)
            });
            // 「…」上層資料夾列：放在所有排序列之前，點擊回上一層。
            let parent = selected.parent().filter(|p| p != &selected).map(PathBuf::from);
            if parent.is_some() {
                let full_rect = egui::Rect::from_min_size(
                    egui::pos2(ui.max_rect().left(), ui.cursor().top()),
                    egui::vec2(full_w, ROW_HEIGHT),
                );
                if ui.is_rect_visible(full_rect) {
                    let (row_rect, resp) = ui.allocate_exact_size(egui::vec2(full_w, ROW_HEIGHT), egui::Sense::click());
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, ".."));
                    let visuals = ui.visuals();
                    let hover_fill = visuals.widgets.hovered.weak_bg_fill;
                    let text_color = visuals.text_color();
                    if resp.hovered() {
                        ui.painter().rect_filled(row_rect, 2.0, hover_fill);
                    }
                    // 「…」固定用向量資料夾圖示（上層可能是磁碟機根，無 Shell 圖示場景）。
                    let icon_slot = favorites::paint_row_icon(ui, egui::pos2(row_rect.left() + 14.0, row_rect.center().y), &favorites::RowIcon::Folder);
                    ui.painter().text(
                        egui::pos2(row_rect.left() + 6.0 + icon_slot, row_rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        "..",
                        egui::FontId::proportional(12.0),
                        text_color,
                    );
                    if resp.clicked() {
                        if let Some(p) = parent {
                            *action = Action::Enter(p);
                        }
                    }
                } else {
                    ui.allocate_rect(full_rect, egui::Sense::hover());
                }
            }
            match rows {
                None => {
                    ui.painter().text(
                        egui::pos2(hrect.left() + 8.0, hrect.max.y + ROW_HEIGHT / 2.0),
                        egui::Align2::LEFT_CENTER,
                        unreadable,
                        egui::FontId::proportional(12.0),
                        weak,
                    );
                }
                Some(_) => {
                    if sorted.is_empty() {
                        ui.painter().text(
                            egui::pos2(hrect.left() + 8.0, hrect.max.y + ROW_HEIGHT / 2.0),
                            egui::Align2::LEFT_CENTER,
                            empty_dir,
                            egui::FontId::proportional(12.0),
                            weak,
                        );
                    }
                    for row in &sorted {
                        let filtered = self.explorer.filtered();
                        if !row.dir && filtered && !glob_match(&row.name, &self.explorer.filter_applied) {
                            continue;
                        }
                        let full_rect = egui::Rect::from_min_size(
                            egui::pos2(ui.max_rect().left(), ui.cursor().top()),
                            egui::vec2(full_w, ROW_HEIGHT),
                        );
                        if !ui.is_rect_visible(full_rect) {
                            // 仍需推進版面游標，否則捲動計算錯亂。
                            ui.allocate_rect(full_rect, egui::Sense::hover());
                            continue;
                        }
                        let (row_rect, resp) = ui.allocate_exact_size(egui::vec2(full_w, ROW_HEIGHT), egui::Sense::click());
                        resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, &row.name));
                        let visuals = ui.visuals();
                        if resp.hovered() {
                            ui.painter().rect_filled(row_rect, 2.0, visuals.widgets.hovered.weak_bg_fill);
                        }
                        let color = visuals.text_color();
                        let x0 = row_rect.left() + 6.0;
                        let y = row_rect.center().y;
                        // 名稱欄位含系統圖示
                        let row_path = selected.join(&row.name);
                        let row_str = row_path.to_string_lossy().into_owned();
                        let tex = self.shell_icons.get(ui.ctx(), &row_str, row.dir);
                        let icon = match &tex {
                            Some(t) => favorites::RowIcon::Tex(t),
                            None => if row.dir { favorites::RowIcon::Folder } else { favorites::RowIcon::File },
                        };
                        let icon_slot = favorites::paint_row_icon(ui, egui::pos2(x0 + 9.0, y), &icon);
                        // 名稱（截斷至欄寬）
                        let mut job = egui::text::LayoutJob { wrap: egui::text::TextWrapping::truncate_at_width(name_w - 12.0 - icon_slot), ..Default::default() };
                        job.append(&row.name, 0.0, egui::TextFormat::simple(egui::TextStyle::Body.resolve(ui.style()), color));
                        let galley = ui.painter().layout_job(job);
                        ui.painter().galley(egui::pos2(x0 + icon_slot, y - galley.size().y / 2.0), galley, color);
                        // 修改日期
                        if show_date {
                            let date = row.modified.and_then(|s| chrono::DateTime::from_timestamp(s, 0)).map(|dt| dt.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string()).unwrap_or_else(|| "-".into());
                            ui.painter().text(egui::pos2(x0 + name_w, y), egui::Align2::LEFT_CENTER, date, egui::FontId::proportional(12.0), color);
                        }
                        // 類型
                        if show_type {
                            let ext = std::path::Path::new(&row.name).extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_default();
                            let kind = if row.dir { folder_type.to_string() } else if ext.is_empty() { file_type.to_string() } else { ext };
                            ui.painter().text(egui::pos2(x0 + name_w + date_w, y), egui::Align2::LEFT_CENTER, kind, egui::FontId::proportional(12.0), color);
                        }
                        // 大小（人類可讀，右對齊）
                        if show_size {
                            let size = row.size.map(|b| human_size(b)).unwrap_or_else(|| "-".into());
                            ui.painter().text(egui::pos2(row_rect.right() - 6.0, y), egui::Align2::RIGHT_CENTER, size, egui::FontId::proportional(12.0), color);
                        }

                        let path = selected.join(&row.name);
                        if resp.clicked() && !row.dir {
                            *action = Action::Open(path.clone());
                        }
                        // 單擊資料夾：同步樹選中並展開（不切換列表所在資料夾）——這樣關閉分欄後
                        // 樹能立即顯示該資料夾的內容。
                        if resp.clicked() && row.dir {
                            *action = Action::Focus(path.clone());
                        }
                        if resp.double_clicked() && row.dir {
                            *action = Action::Enter(path);
                        }
                    }
                }
            }
        });
    }
}

/// 位元組數的人類可讀表示（B／KB／MB／GB，一位小數）。
fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// 對檔名做 `*` / `?` 萬用字元匹配（不分大小寫；`;` 或空格分隔多個模式）。
/// 不引入新依賴；行為有測試覆蓋。
pub(crate) fn glob_match(name: &str, patterns: &str) -> bool {
    let lower = name.to_lowercase();
    patterns
        .split([';', ' '])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .any(|p| match_one(lower.as_bytes(), p.to_lowercase().as_bytes()))
}

/// 經典兩指針 + 回溯的單模式匹配（`*` 貪婪回溯）。
fn match_one(name: &[u8], pat: &[u8]) -> bool {
    let (mut i, mut j) = (0usize, 0usize);
    let mut star = usize::MAX;
    let mut mark = 0usize;
    while i < name.len() {
        if j < pat.len() && (pat[j] == b'?' || pat[j] == name[i]) {
            i += 1;
            j += 1;
        } else if j < pat.len() && pat[j] == b'*' {
            star = j;
            mark = i;
            j += 1;
        } else if star != usize::MAX {
            j = star + 1;
            mark += 1;
            i = mark;
        } else {
            return false;
        }
    }
    while j < pat.len() && pat[j] == b'*' {
        j += 1;
    }
    j == pat.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_drives_exist() {
        let found: Vec<String> = (b'A'..=b'Z')
            .filter_map(|c| {
                let root = format!("{}:\\", char::from(c));
                Path::new(&root).is_dir().then_some(root)
            })
            .collect();
        println!("[probe] drives = {found:?}");
        assert!(!found.is_empty(), "至少應探測到 C:");
    }

    #[test]
    fn glob_matches_wildcards_case_insensitively() {
        assert!(glob_match("Report.TXT", "*.txt"));
        assert!(glob_match("data2026.log", "data????.log"));
        assert!(!glob_match("data26.log", "data????.log"));
        assert!(glob_match("src", "src"));
        assert!(glob_match("README.md", "*.md;*.txt"));
        assert!(glob_match("notes.txt", "*.md *.txt"));
        assert!(!glob_match("notes.txt", "*.md;*.rs"));
        assert!(glob_match("anything", "*"));
        assert!(glob_match("a.b.c", "*.*"));
        assert!(!glob_match("abc", "abd"));
    }
}
