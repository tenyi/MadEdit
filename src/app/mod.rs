//! egui 主程式：多文件分頁、選單、工具列、狀態列、對話框與快捷鍵派送

mod actions;
mod updates;
pub mod commands;
mod i18n;
mod icons;
mod dialogs;
mod explorer;
mod shell_icons;mod favorites;
mod form;
mod find;
mod findfiles;
pub mod fonts;
pub mod hexview;
mod macros;
mod menus;
mod mdmath;
mod mdpreview;
mod preview;
mod print;
mod svgpreview;
mod imgpreview;
mod runner;
#[cfg(windows)]
mod titlebar;

/// Windows 自繪標題欄的應用程式圖示（egui 貼圖，從內嵌 PNG 載入一次後快取）。
#[cfg(windows)]
fn load_app_icon(ctx: &egui::Context) -> TextureHandle {
    let img = image::load_from_memory(include_bytes!("../../assets/madedit-icon.png")).expect("app icon decodes");
    let size = [img.width() as usize, img.height() as usize];
    let image = egui::ColorImage::from_rgba_unmultiplied(size, &img.to_rgba8());
    ctx.load_texture("madedit-app-icon", image, egui::TextureOptions::default())
}
pub mod view;
#[cfg(test)]
mod visual_tests;
#[cfg(test)]
mod ui_tests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui::{self, Color32, Event, KeyboardShortcut, TextureHandle};

use crate::core::document::{Document, OpenOptions, SaveError};
use crate::core::editor;
use crate::core::encoding;
use crate::core::model::{EditMode, NewLineFormat};
use crate::core::settings::{SessionFile, Settings, SettingsLoadError, ThemeChoice};
use crate::core::spell::SpellChecker;

use commands::C;
use dialogs::Dialog;
use find::{FindBar, GotoBar};
use findfiles::FindInFiles;
use fonts::FontManager;
use hexview::HexViewState;
use macros::MacroState;
use view::{CharWidths, ViewEnv, ViewOutput, ViewState};

/// 一個分頁：文件與其檢視狀態
pub struct Tab {
    pub doc: Document,
    pub view: ViewState,
    pub hex: HexViewState,
    /// 磁碟上的檔案已被其他程式修改（只有使用者選擇重新載入或忽略時才清除）
    pub disk_changed: bool,
    /// 此分頁的 Markdown 預覽；切換分頁時跟著切換
    pub md_preview: Option<mdpreview::MarkdownPreview>,
    /// 此分頁的 SVG 圖形預覽（檢視 > 預覽為 > 圖形）
    pub svg_preview: Option<svgpreview::SvgPreview>,
    /// 此分頁的點陣圖片預覽（檢視 > 預覽為 > 圖形（圖片））
    pub img_preview: Option<imgpreview::ImagePreview>,
}

impl Tab {
    fn new(doc: Document) -> Self {
        Self { doc, view: ViewState::default(), hex: HexViewState::default(), disk_changed: false, md_preview: None, svg_preview: None, img_preview: None }
    }
}

/// 比較兩個路徑是否指向同一個檔案；檔案尚未存在（無法 canonicalize）時改用絕對路徑比較，
/// 避免「建立新檔但尚未存檔」時再開同一路徑而多出一個分頁
fn same_target(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// 路徑是否為點陣圖片（可自動以圖形預覽）：bmp／jpg／jpeg／png／gif／webp。
pub(crate) fn is_raster_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| matches!(e.to_ascii_lowercase().as_str(), "bmp" | "jpg" | "jpeg" | "png" | "gif" | "webp"))
        .unwrap_or(false)
}

/// 需要使用者確認的動作
pub enum Confirm {
    /// 關閉有未存檔修改的文件：依序詢問 `ids` 中每份文件；`exit` 為真時全部處理完後結束程式
    Close { ids: Vec<u64>, exit: bool },
    /// 解碼有錯的文件仍要存檔
    SaveLossy { id: u64, path: Option<PathBuf> },
    /// 目前編碼無法表示部分字元：可改以 UTF-8 存檔
    SaveUnmappable { id: u64, path: Option<PathBuf>, message: String },
    /// 磁碟上的檔案在開啟後被其他程式修改，仍要覆寫
    OverwriteExternal { id: u64, path: Option<PathBuf> },
    /// 重新載入會遺失未存檔修改
    ReloadModified { id: u64 },
    /// 檔案不存在，詢問是否建立新檔案（比照 Notepad）
    CreateNew { path: PathBuf },
    /// 一般訊息
    Message { title: String, text: String },
    /// A proportional editor font was picked while fixed-width mode is on: offer to turn it off.
    ProportionalFont,
}

pub struct App {
    updates: updates::UpdateState,
    pub instance: Option<crate::single_instance::Instance>,
    pub settings: Settings,
    pub tabs: Vec<Tab>,
    pub active: usize,
    /// 沒有開啟任何分頁時，文件相關查詢（選單、工具列、對話框）使用的空白替代文件。
    /// 它不會顯示、不會存檔，也不會進入工作階段；所有分頁關閉後視窗仍保持開啟。
    /// 延後建立，才不會佔用第一個「未命名-N」的文件編號。
    empty_tab: std::sync::OnceLock<Tab>,
    /// 預覽與來源共用文件，只另外記錄目前選取的是哪一種檢視。
    preview_tab_active: bool,
    /// 目前選取的是 SVG 圖形預覽分頁（與 Markdown 預覽分頁互斥）
    svg_preview_tab_active: bool,
    /// 目前選取的是點陣圖片預覽分頁
    img_preview_tab_active: bool,
    prev_active: usize,
    prev_preview_tab: bool,
    fonts: FontManager,
    /// 已套用的（編輯區字型、介面字型）設定值
    font_applied: Option<(String, String)>,
    /// The editor font was replaced this frame; checked next frame, once the fonts are active.
    fonts_changed: bool,
    widths: CharWidths,
    /// Toolbar icon textures: (light theme, dark theme). See `icons.rs`.
    icons: HashMap<&'static str, (TextureHandle, TextureHandle)>,
    /// Windows 自繪標題欄：應用程式圖示貼圖與子類安裝標記
    #[cfg(windows)]
    app_icon: Option<TextureHandle>,
    #[cfg(windows)]
    titlebar_installed: bool,
    status: Option<(String, f64)>,
    find: FindBar,
    goto: GotoBar,
    dialog: Option<Dialog>,
    confirm: Option<Confirm>,
    /// Kind of dialog and confirmation shown last frame, to tell when one has just opened
    /// (its first field takes focus and its content starts scrolled to the top). Reopening
    /// the same kind over itself (F9 while Recent Files is open) does not count as opening.
    shown_dialog: Option<std::mem::Discriminant<Dialog>>,
    shown_confirm: Option<std::mem::Discriminant<Confirm>>,
    /// 關閉流程中「儲存」需要再確認（外部修改、解碼錯誤等）時暫存的待關閉分頁與是否結束程式，
    /// 存檔成功後由此接續關閉；再確認被取消時清除
    pending_close: Option<(Vec<u64>, bool)>,
    /// 收藏群組「取代開啟」：關閉流程（含儲存確認）全部結束後接續開啟的群組索引；
    /// 使用者取消關閉時一併清除，不會開啟群組
    pending_open_group: Option<usize>,
    /// 等待使用者確認是否建立的檔案（開啟不存在的檔案時排隊）
    pending_create: Vec<PathBuf>,
    macros: MacroState,
    spell: Option<SpellChecker>,
    spell_error: Option<String>,
    fif: FindInFiles,
    /// 腳本運行器（工具 → 運行腳本，輸出到底部面板）
    runner: runner::ScriptRunner,
    bindings: Vec<(KeyboardShortcut, C)>,
    post_it: bool,
    /// Set each frame by the menu bar when it had room for the zoom / theme / quick-search
    /// controls; otherwise the toolbar shows them.
    quick_in_menubar: bool,
    fullscreen: bool,
    last_autosave: f64,
    last_disk_check: f64,
    /// 上一幀視窗是否有焦點；重新取得焦點時立即檢查外部修改，不必等下一個 1.5 秒週期。
    was_focused: bool,
    /// 上一幀「自動重新載入」是否已開啟；剛開啟時也處理已經顯示提示的分頁。
    auto_reload_was_on: bool,
    allow_close: bool,
    clipboard: Option<arboard::Clipboard>,
    /// 測試用：啟動後截圖存檔並結束
    screenshot: Option<(PathBuf, u32)>,
    /// 截圖回呼寫入的結果（egui 0.37 起截圖改為回呼，不再是事件）
    shot_slot: std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<egui::ColorImage>>>>,
    pending: Vec<C>,
    /// 下一幀將焦點交給編輯區
    focus_editor: bool,
    /// 上次在分頁列自動捲動過的（索引、是否預覽分頁）；僅在激活分頁變化時觸發捲動
    tab_scroll_done: Option<(usize, bool)>,
    /// 簡繁轉換的安全模式（不轉換易誤轉的字）
    chinese_safe: bool,
    /// 收藏面板中選取的群組（右鍵與「新增…」以此為目標）
    fav_selected: Option<usize>,
    /// 文件視圖目前顯示的頁籤；預留給「開啟」等新頁籤
    file_view_tab: favorites::FileViewTab,
    /// 資源管理器頁籤的狀態（樹、篩選、分欄）
    explorer: explorer::Explorer,
    /// Windows 系統檔案圖示快取（資源管理器用）
    shell_icons: shell_icons::IconCache,
    /// 收藏樹根節點是否展開（只在本次執行期間記住）
    fav_root_open: bool,
    /// 開啟中的預覽列印視窗
    preview: Option<preview::PrintPreview>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext, files: Vec<PathBuf>, screenshot: Option<PathBuf>) -> Self {
        // 自動化截圖以預設設定啟動，不還原或覆寫使用者的工作階段。
        let (settings, settings_error) = if screenshot.is_some() {
            (Settings::default(), None)
        } else {
            Settings::load_checked()
        };
        Self::with_settings(cc, files, screenshot, settings, settings_error)
    }

    /// Builds the app from settings already loaded; tests start it from chosen settings here,
    /// since `Settings::load_checked` never reads a file under `cfg(test)`.
    fn with_settings(cc: &eframe::CreationContext, files: Vec<PathBuf>, screenshot: Option<PathBuf>, settings: Settings, settings_error: Option<SettingsLoadError>) -> Self {
        #[cfg(target_os = "macos")]
        crate::macos::attach_context(&cc.egui_ctx);
        let auto_reload_was_on = settings.auto_reload_external;
        // 「執行輸出」面板沿用上次結束時的顯示狀態（檢視 → 運行輸出）。
        let mut runner = runner::ScriptRunner::default();
        runner.show = settings.show_script_output;
        let mut app = Self {
            updates: updates::UpdateState::default(),
            instance: None,
            settings,
            tabs: Vec::new(),
            active: 0,
            empty_tab: std::sync::OnceLock::new(),
            preview_tab_active: false,
            svg_preview_tab_active: false,
            img_preview_tab_active: false,
            prev_active: 0,
            prev_preview_tab: false,
            fonts: FontManager::new(),
            font_applied: None,
            fonts_changed: false,
            widths: CharWidths::default(),
            icons: HashMap::new(),
            #[cfg(windows)]
            app_icon: None,
            #[cfg(windows)]
            titlebar_installed: false,
            status: None,
            find: FindBar::default(),
            goto: GotoBar::default(),
            dialog: None,
            confirm: None,
            shown_dialog: None,
            shown_confirm: None,
            pending_close: None,
            pending_open_group: None,
            pending_create: Vec::new(),
            macros: MacroState::default(),
            spell: None,
            spell_error: None,
            fif: FindInFiles::default(),
            runner,
            bindings: Vec::new(),
            post_it: false,
            quick_in_menubar: false,
            fullscreen: false,
            last_autosave: 0.0,
            last_disk_check: 0.0,
            was_focused: true,
            auto_reload_was_on,
            allow_close: false,
            // 測試時不連接系統剪貼簿，避免結果受使用者剪貼簿內容影響或覆寫它
            clipboard: if cfg!(test) { None } else { arboard::Clipboard::new().ok() },
            screenshot: screenshot.map(|p| (p, 0)),
            shot_slot: Default::default(),
            pending: Vec::new(),
            focus_editor: true,
            tab_scroll_done: None,
            chinese_safe: false,
            fav_selected: None,
            file_view_tab: favorites::FileViewTab::List,
            explorer: explorer::Explorer::default(),
            shell_icons: shell_icons::IconCache::default(),
            fav_root_open: true,
            preview: None,
        };
        app.load_icons(&cc.egui_ctx);
        app.rebuild_bindings();
        app.apply_theme(&cc.egui_ctx);
        // 資源管理器頁籤：從設定還原分欄/篩選/位置
        app.explorer.restore_from_settings(&app.settings);
        // 第一幀繪製前就註冊編輯區字型家族（set_fonts 於下一幀才生效，延後註冊會 panic）
        let fam = app.settings.font_family.clone();
        let ui_fam = app.settings.ui_font_family.clone();
        app.fonts.apply(&cc.egui_ctx, &fam, &ui_fam);
        app.font_applied = Some((fam, ui_fam));
        // 沿用上次的「輸出到檔案」勾選；以續寫模式開檔（失敗就靜默停用，不打擾啟動）
        if app.settings.output_log_to_file
            && app.runner.set_log_enabled_append(true).is_err()
        {
            app.runner.disable_log();
            app.settings.output_log_to_file = false;
        }

        if files.is_empty() && app.settings.reload_files_on_startup {
            let session = app.settings.session_files.clone();
            for f in session {
                if f.path.exists() && app.open_path(&f.path) {
                    let doc = &mut app.tabs[app.active].doc;
                    doc.editor.set_caret(f.line, f.col);
                }
            }
            if !app.tabs.is_empty() {
                app.active = app.settings.session_active.min(app.tabs.len() - 1);
            }
        }
        for f in files {
            app.open_path(&f);
        }
        if app.tabs.is_empty() {
            app.new_document();
        }
        if app.settings.spell_check {
            app.load_spell();
        }
        if let Some(err) = settings_error {
            let text = app.settings_error_text(&err);
            if app.confirm.is_none() {
                app.confirm = Some(Confirm::Message { title: app.t("msg.settings_bad", "設定檔損毀").to_string(), text });
            } else {
                app.set_status(text);
            }
        }
        app
    }

    // ── 文件與分頁 ──────────────────────────────────────────────

    fn open_external_files(&mut self, ctx: &egui::Context, files: Vec<PathBuf>) {
        for path in files {
            self.open_path(&path);
        }
        self.focus_editor = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    /// 空白替代文件；第一次需要時才建立，避免佔用第一個「未命名-N」的編號。
    fn empty_tab(&self) -> &Tab {
        self.empty_tab.get_or_init(|| Tab::new(Document::new_untitled(encoding_rs::UTF_8, false, NewLineFormat::Unix, "Plain Text")))
    }

    fn empty_tab_mut(&mut self) -> &mut Tab {
        if self.empty_tab.get().is_none() {
            let _ = self.empty_tab.set(Tab::new(Document::new_untitled(encoding_rs::UTF_8, false, NewLineFormat::Unix, "Plain Text")));
        }
        self.empty_tab.get_mut().expect("empty_tab initialized above")
    }

    pub fn doc(&self) -> &Document {
        self.tabs.get(self.active).map(|tab| &tab.doc).unwrap_or(&self.empty_tab().doc)
    }

    pub fn doc_mut(&mut self) -> &mut Document {
        let active = self.active;
        if active < self.tabs.len() { &mut self.tabs[active].doc } else { &mut self.empty_tab_mut().doc }
    }

    pub fn tab_mut(&mut self) -> &mut Tab {
        let active = self.active;
        if active < self.tabs.len() { &mut self.tabs[active] } else { self.empty_tab_mut() }
    }

    fn index_of(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.doc.id == id)
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), 0.0));
    }

    /// 設定檔損毀提示。譯文裡的 `\n` 是跳脫的換行
    fn settings_error_text(&self, err: &SettingsLoadError) -> String {
        let raw = match &err.backup {
            Ok(path) => self
                .t("msg.settings_reset", "設定檔無法讀取，已改用預設設定。\n原設定檔已備份為 {q}\n\n錯誤：{n}")
                .replace("{q}", &path.display().to_string())
                .replace("{n}", &err.detail),
            Err(e) => self
                .t("msg.settings_reset_nobak", "設定檔無法讀取，已改用預設設定（備份失敗：{q}）。\n\n錯誤：{n}")
                .replace("{q}", e)
                .replace("{n}", &err.detail),
        };
        raw.replace("\\n", "\n")
    }

    /// 目前介面實際使用的語言（「跟隨系統」已解析成具體語言）
    pub fn ui_lang(&self) -> i18n::Lang {
        i18n::resolve(self.settings.ui_language)
    }

    /// 介面字串。`zh_tw` 是繁體中文原文。
    pub fn t<'a>(&self, key: &str, zh_tw: &'a str) -> &'a str {
        i18n::translate(self.ui_lang(), key, zh_tw)
    }

    /// 帶 `{n}` 或 `{q}` 的介面字串
    pub fn tf(&self, key: &str, zh_tw: &str, n: Option<usize>, q: Option<&str>) -> String {
        i18n::fill(self.t(key, zh_tw), n, q)
    }

    /// 命令在選單與工具列提示上的文字
    pub fn cmd_text(&self, c: C) -> String {
        i18n::command_text(self.ui_lang(), c)
    }

    pub fn open_options(&self) -> OpenOptions {
        OpenOptions {
            default_encoding: encoding::encoding_for_name(&self.settings.default_encoding).unwrap_or(encoding_rs::UTF_8),
            fallback_encoding: encoding::encoding_for_name(&self.settings.fallback_encoding).unwrap_or(encoding_rs::BIG5),
            enforce_default_encoding: self.settings.enforce_default_encoding,
            max_text_size: self.settings.max_text_file_mb.max(1) * 1024 * 1024,
        }
    }

    fn activate(&mut self, idx: usize) {
        self.activate_view(idx, false);
    }

    fn activate_view(&mut self, idx: usize, preview: bool) {
        if idx >= self.tabs.len() {
            return;
        }
        let preview = preview && self.tabs[idx].md_preview.as_ref().is_some_and(|pv| pv.placement == mdpreview::Placement::Tab);
        if idx != self.active || preview != self.is_preview_tab() {
            self.prev_active = self.active;
            self.prev_preview_tab = self.is_preview_tab();
        }
        self.active = idx;
        self.preview_tab_active = preview;
        self.focus_editor = !preview;
        // 切換到這個分頁時立即檢查外部修改，背景分頁不必等下一個輪詢週期。
        self.check_disk_changes();
    }

    fn is_preview_tab(&self) -> bool {
        self.preview_tab_active
            && self.tabs.get(self.active).and_then(|tab| tab.md_preview.as_ref()).is_some_and(|pv| pv.placement == mdpreview::Placement::Tab)
    }

    /// 目前是否顯示 SVG 圖形預覽分頁
    pub(crate) fn is_svg_preview_tab(&self) -> bool {
        self.svg_preview_tab_active && self.tabs.get(self.active).is_some_and(|tab| tab.svg_preview.is_some())
    }

    /// 目前是否顯示點陣圖片預覽分頁
    pub(crate) fn is_img_preview_tab(&self) -> bool {
        self.img_preview_tab_active && self.tabs.get(self.active).is_some_and(|tab| tab.img_preview.is_some())
    }

    fn tab_pages(&self) -> Vec<(usize, bool)> {
        self.tabs.iter().enumerate().flat_map(|(i, tab)| {
            let preview = tab.md_preview.as_ref().is_some_and(|pv| pv.placement == mdpreview::Placement::Tab);
            let svg = tab.svg_preview.is_some();
            let img = tab.img_preview.is_some();
            std::iter::once((i, false))
                .chain(preview.then_some((i, true)))
                .chain(svg.then_some((i, false)))
                .chain(img.then_some((i, false)))
        }).collect()
    }

    fn activate_page(&mut self, page: usize) {
        if let Some((index, preview)) = self.tab_pages().get(page).copied() {
            self.activate_view(index, preview);
        }
    }

    pub fn new_document(&mut self) {
        let s = &self.settings;
        let enc = encoding::encoding_for_name(&s.new_doc_encoding).unwrap_or(encoding_rs::UTF_8);
        let nl = match s.new_doc_newline.as_str() {
            "CRLF" => NewLineFormat::Dos,
            "CR" => NewLineFormat::Mac,
            _ => NewLineFormat::Unix,
        };
        let doc = Document::new_untitled(enc, s.new_doc_bom, nl, &s.new_doc_syntax);
        self.tabs.push(Tab::new(doc));
        let idx = self.tabs.len() - 1;
        self.activate(idx);
        self.active = idx;
    }

    /// 開啟檔案（已開啟則切換到該分頁）；成功回傳 true
    pub fn open_path(&mut self, path: &Path) -> bool {
        // 檔案不存在時無法 canonicalize，改用絕對路徑，讓後續顯示與建立檔案的位置一致
        let canon = match path.canonicalize() {
            Ok(p) => p,
            Err(_) => {
                if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    std::env::current_dir().map(|d| d.join(path)).unwrap_or_else(|_| path.to_path_buf())
                }
            }
        };
        if let Some(i) = self.tabs.iter().position(|t| t.doc.path.as_deref().is_some_and(|p| same_target(p, &canon))) {
            self.activate(i);
            // 勾選「預覽」總開關時，重開已開啟的檔案同樣自動分發到對應的預覽
            if self.settings.auto_preview {
                self.dispatch_auto_preview();
            }
            return true;
        }
        // 先登記再讀檔：執行中的背景取代不會再寫入這個檔案，正在寫入時則等寫完才讀
        self.fif.mark_opened(&canon);
        match Document::open(&canon, &self.open_options()) {
            Ok(mut doc) => {
                use crate::core::settings::OpenViewMode;
                match self.settings.open_view_mode {
                    OpenViewMode::Text => {
                        let _ = doc.set_mode(EditMode::Text);
                    }
                    OpenViewMode::Column => {
                        let _ = doc.set_mode(EditMode::Column);
                    }
                    OpenViewMode::Hex => {
                        let _ = doc.set_mode(EditMode::Hex);
                    }
                    OpenViewMode::Auto => {}
                }
                if self.settings.restore_caret_position
                    && let Some(&(l, c)) = self.settings.caret_positions.get(&canon.to_string_lossy().to_string())
                {
                    doc.editor.set_caret(l, c);
                }
                let mut notes = Vec::new();
                if doc.hex_only {
                    notes.push(format!("檔案大於 {} MB，以十六進位模式開啟", self.settings.max_text_file_mb));
                }
                if doc.has_decode_error {
                    notes.push(format!("無法以 {} 正確解碼，請以「檢視 → 編碼」重新解碼", doc.encoding.name()));
                }
                if doc.mixed_newlines {
                    notes.push("檔案混用多種換行字元，存檔時會統一".to_string());
                }
                self.push_tab(doc);
                self.settings.add_recent_file(canon);
                let shown = crate::core::display_path(&path);
                self.set_status(if notes.is_empty() { self.tf("msg.opened", "已開啟 {q}", None, Some(&shown)) } else { notes.join("；") });
                // 勾選「預覽」總開關時，依副檔名自動分發到對應的預覽
                if self.settings.auto_preview {
                    self.dispatch_auto_preview();
                }
                true
            }
            Err(e) => {
                if e.kind() == std::io::ErrorKind::NotFound {
                    // 檔案不存在：比照 Notepad 詢問是否建立新檔案
                    self.pending_create.push(canon.clone());
                    if self.confirm.is_none() {
                        self.confirm = Some(Confirm::CreateNew { path: canon });
                    }
                } else {
                    self.confirm = Some(Confirm::Message { title: self.t("msg.open_fail", "開啟失敗").to_string(), text: format!("{}\n{e}", crate::core::display_path(&path)) });
                }
                false
            }
        }
    }

    /// 建立指定路徑的新檔案（用於「開啟不存在的檔案」時使用者選擇建立）
    fn create_new_file(&mut self, path: PathBuf) {
        let s = &self.settings;
        let enc = encoding::encoding_for_name(&s.new_doc_encoding).unwrap_or(encoding_rs::UTF_8);
        let nl = match s.new_doc_newline.as_str() {
            "CRLF" => NewLineFormat::Dos,
            "CR" => NewLineFormat::Mac,
            _ => NewLineFormat::Unix,
        };
        // 副檔名決定語法，與開啟既有檔案的行為一致
        let syntax = crate::core::syntax::detect_syntax(&path, "");
        let mut doc = Document::new_untitled(enc, s.new_doc_bom, nl, &syntax);
        // 直接綁定路徑，存檔時寫回同一個檔案，不再詢問另存新檔
        doc.path = Some(path.clone());
        self.push_tab(doc);
        let shown = crate::core::display_path(&path);
        self.set_status(self.tf("msg.created", "已建立 {q}", None, Some(&shown)));
    }

    /// 新增分頁；只剩一個未修改的空白未命名文件時直接取代
    fn push_tab(&mut self, doc: Document) {
        let replace_blank = self.tabs.len() == 1
            && self.tabs[0].doc.path.is_none()
            && !self.tabs[0].doc.is_modified()
            && self.tabs[0].doc.editor.lines.len() == 1
            && self.tabs[0].doc.editor.lines[0].is_empty();
        if replace_blank {
            self.tabs[0] = Tab::new(doc);
            self.active = 0;
        } else {
            self.tabs.push(Tab::new(doc));
            let idx = self.tabs.len() - 1;
            self.activate(idx);
        }
    }

    pub fn open_dialog(&mut self) {
        let mut dlg = rfd::FileDialog::new().set_title(self.t("msg.open_file", "開啟舊檔"));
        if let Some(dir) = self.doc().path.as_ref().and_then(|p| p.parent()) {
            dlg = dlg.set_directory(dir);
        }
        if let Some(files) = dlg.pick_files() {
            for f in files {
                self.open_path(&f);
            }
        }
    }

    /// 清理另存對話框的檔名：去除前後空白，以及結尾多餘的 `.`（目錄部分不動）。
    /// macOS 連按兩下空白鍵會自動輸入「. 」；Windows 會悄悄剝掉結尾的點與空白，
    /// 造成實際檔名與畫面不一致，因此統一在此清掉。清理後若為空則維持原樣。
    pub fn trim_file_name(p: PathBuf) -> PathBuf {
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else { return p };
        let clean = name.trim().trim_end_matches(|c: char| c == '.' || c.is_whitespace());
        if clean.is_empty() || clean == name {
            return p;
        }
        p.with_file_name(clean)
    }

    /// 存檔（必要時詢問路徑或確認）；`as_new` 為真時一律詢問路徑
    pub fn save_tab(&mut self, idx: usize, as_new: bool) -> bool {
        let id = self.tabs[idx].doc.id;
        let path = if as_new || self.tabs[idx].doc.path.is_none() {
            let name = self.tabs[idx].doc.display_name();
            let mut dlg = rfd::FileDialog::new().set_title(self.t("msg.save_as", "另存新檔")).set_file_name(&name);
            if let Some(dir) = self.tabs[idx].doc.path.as_ref().and_then(|p| p.parent()) {
                dlg = dlg.set_directory(dir);
            }
            match dlg.save_file() {
                Some(p) => Some(Self::trim_file_name(p)),
                None => return false,
            }
        } else {
            None
        };
        // 存檔前檢查檔案是否在開啟後被外部修改（P3-4）
        if path.is_none() && self.tabs[idx].doc.changed_on_disk() {
            self.confirm = Some(Confirm::OverwriteExternal { id, path });
            return false;
        }
        self.save_with(id, path, false)
    }

    /// 實際存檔並處理錯誤
    fn save_with(&mut self, id: u64, path: Option<PathBuf>, allow_lossy: bool) -> bool {
        let Some(idx) = self.index_of(id) else { return false };
        let target = path.clone().or_else(|| self.tabs[idx].doc.path.clone());
        if self.settings.auto_backup
            && let Some(t) = &target
            && t.exists()
        {
            let mut bak = t.as_os_str().to_owned();
            bak.push(".bak");
            let _ = std::fs::copy(t, PathBuf::from(bak));
        }
        let result = self.tabs[idx].doc.save(path.as_deref(), allow_lossy);
        match result {
            Ok(()) => {
                self.tabs[idx].disk_changed = false;
                if let Some(p) = self.tabs[idx].doc.path.clone() {
                    self.settings.add_recent_file(p.clone());
                    let shown = p.display().to_string();
                    self.set_status(self.tf("msg.saved", "已儲存 {q}", None, Some(&shown)));
                }
                true
            }
            Err(SaveError::DecodeErrors) => {
                self.confirm = Some(Confirm::SaveLossy { id, path });
                false
            }
            Err(SaveError::Unmappable(u)) => {
                let samples: String = u.samples.iter().collect();
                let message = self
                    .t("msg.unmappable", "目前編碼無法表示 {n} 個字元（例如「{q}」）")
                    .replace("{n}", &u.count.to_string())
                    .replace("{q}", &samples);
                self.confirm = Some(Confirm::SaveUnmappable { id, path, message });
                false
            }
            Err(e) => {
                let text = match &e {
                    SaveError::Io(err) => self.tf("msg.write_failed", "寫入檔案失敗：{q}", None, Some(&err.to_string())),
                    SaveError::NoPath => self.t("msg.no_path", "尚未指定檔案路徑").to_string(),
                    SaveError::ReadOnly => self.t("msg.readonly", "文件為唯讀").to_string(),
                    _ => e.to_string(),
                };
                self.confirm = Some(Confirm::Message { title: self.t("msg.save_fail", "存檔失敗").to_string(), text });
                false
            }
        }
    }

    /// 要求關閉分頁：有未存檔修改的先詢問
    pub fn request_close(&mut self, ids: Vec<u64>, exit: bool) {
        let mut clean = Vec::new();
        let mut dirty = Vec::new();
        for id in ids {
            match self.index_of(id) {
                Some(i) if self.tabs[i].doc.is_modified() => dirty.push(id),
                Some(_) => clean.push(id),
                None => {}
            }
        }
        // 結束時保留分頁直到視窗真的關閉，才能保存完整工作階段；
        // 若後續取消未存檔確認，乾淨的分頁也仍留在原處。
        if !exit {
            for id in clean {
                self.close_now(id);
            }
        }
        if dirty.is_empty() {
            if exit {
                self.allow_close = true;
            }
        } else {
            self.confirm = Some(Confirm::Close { ids: dirty, exit });
        }
    }

    fn close_now(&mut self, id: u64) {
        let Some(i) = self.index_of(id) else { return };
        if i == self.active {
            self.preview_tab_active = false;
        }
        self.remember_caret(i);
        self.tabs.remove(i);
        if self.tabs.is_empty() {
            // 最後一個分頁關閉後進入空白工作區：不自動建立「未命名」，也不結束程式。
            self.active = 0;
            self.prev_active = 0;
            self.preview_tab_active = false;
            self.focus_editor = false;
            self.find.visible = false;
            self.goto.visible = false;
        } else {
            if self.active >= self.tabs.len() || self.active > i {
                self.active = self.active.saturating_sub(1).min(self.tabs.len() - 1);
            }
            self.prev_active = self.prev_active.min(self.tabs.len() - 1);
            self.focus_editor = true;
        }
    }

    fn remember_caret(&mut self, i: usize) {
        if let Some(p) = self.tabs[i].doc.path.clone() {
            let h = self.tabs[i].doc.editor.selection.head;
            self.settings.remember_caret(&p, h.line, h.col);
        }
    }

    fn reload_tab(&mut self, id: u64) {
        let Some(i) = self.index_of(id) else { return };
        let opts = self.open_options();
        // 重新載入後保留目前可見的列，不跳回游標或文件開頭。
        let top_row = self.tabs[i].view.top_row;
        match self.tabs[i].doc.reload(&opts) {
            Ok(()) => {
                self.tabs[i].disk_changed = false;
                self.tabs[i].view.top_row = top_row;
                self.tabs[i].view.keep_scroll_on_reload();
                self.set_status(self.t("msg.reloaded", "已重新載入"));
            }
            Err(e) => self.confirm = Some(Confirm::Message { title: self.t("msg.reload_fail", "重新載入失敗").to_string(), text: e.to_string() }),
        }
    }

    // ── 設定、主題、字型、快捷鍵 ────────────────────────────────

    pub fn rebuild_bindings(&mut self) {
        let mut b = Vec::new();
        for &c in commands::ALL {
            let keys: Vec<String> = match self.settings.key_bindings.get(c.id()) {
                Some(list) => list.clone(),
                None => c.default_keys().split('|').filter(|k| !k.is_empty()).map(str::to_string).collect(),
            };
            for k in keys {
                if let Some(sc) = commands::parse_shortcut(&k) {
                    b.push((sc, c));
                }
            }
        }
        self.bindings = b;
    }

    pub fn shortcut_text(&self, c: C, ctx: &egui::Context) -> String {
        self.bindings.iter().find(|(_, cc)| *cc == c).map(|(sc, _)| ctx.format_shortcut(sc)).unwrap_or_default()
    }

    pub fn apply_theme(&mut self, ctx: &egui::Context) {
        ctx.set_theme(match self.settings.theme {
            ThemeChoice::System => egui::ThemePreference::System,
            ThemeChoice::Light => egui::ThemePreference::Light,
            ThemeChoice::Dark => egui::ThemePreference::Dark,
        });
        // set_theme only stores the preference; the shared shape and accent settings must be
        // written into both the light and dark styles so a later system theme switch keeps them.
        install_style(ctx);
    }

    /// Theme picked from the View menu or the toolbar: takes effect at once and is saved
    /// right away, unlike the Options dialog, which only applies on OK / Apply.
    pub fn set_theme(&mut self, ctx: &egui::Context, choice: ThemeChoice) {
        self.settings.theme = choice;
        self.apply_theme(ctx);
        let _ = self.settings.save();
    }

    /// Writes the settings file now, for preferences changed outside the Options dialog
    /// (View menu, toolbar, shortcuts). Screenshot runs use default settings and never save.
    pub fn save_settings(&self) {
        if self.screenshot.is_some() {
            return;
        }
        if let Err(e) = self.settings.save() {
            eprintln!("無法儲存設定：{e}");
        }
    }

    /// The theme icon and View > Toggle Theme: switch to the opposite of the theme shown now
    /// (for System, whatever the OS uses). The result is always Light or Dark and is saved, so
    /// the next launch starts in it; only a fresh install, without settings, follows the system.
    pub fn toggle_theme(&mut self, ctx: &egui::Context) {
        let next = if App::is_dark(ctx) { ThemeChoice::Light } else { ThemeChoice::Dark };
        self.set_theme(ctx, next);
    }

    pub fn is_dark(ctx: &egui::Context) -> bool {
        ctx.global_style().visuals.dark_mode
    }

    pub fn syntax_theme(&self, dark: bool) -> String {
        if dark { self.settings.syntax_theme_dark.clone() } else { self.settings.syntax_theme_light.clone() }
    }

    pub fn load_spell(&mut self) {
        match SpellChecker::load(&self.settings.spell_language, &self.settings.personal_dictionary) {
            Ok(s) => {
                self.spell = Some(s);
                self.spell_error = None;
            }
            Err(e) => {
                self.spell = None;
                self.spell_error = Some(e.clone());
                self.set_status(e);
            }
        }
    }

    fn load_icons(&mut self, ctx: &egui::Context) {
        let texture = |name: String, img: &image::RgbaImage, options: egui::TextureOptions| {
            let size = [img.width() as usize, img.height() as usize];
            let ci = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
            ctx.load_texture(name, ci, options)
        };
        // Pixel art stays crisp when enlarged, but the 32 px art is drawn at 16 pt, i.e. shrunk
        // on 1x displays, where nearest sampling drops every other row and column and with
        // them thin outlines; linear minification averages them instead.
        let pixel_art = egui::TextureOptions { minification: egui::TextureFilter::Linear, ..egui::TextureOptions::NEAREST };
        for &(name, bytes) in icons::ICONS {
            if let Some(img) = icons::decode(bytes) {
                let dark = icons::dark_variant(&img);
                self.icons.insert(name, (texture(name.to_string(), &img, pixel_art), texture(format!("{name}-dark"), &dark, pixel_art)));
            }
        }
        // Application icon for the About dialog; it is colorful, so both themes share it. It is
        // enlarged by a non-integer factor (48 px to 56 pt), which nearest sampling makes blocky.
        if let Some(img) = icons::decode(include_bytes!("../../assets/madedit-icon.png")) {
            let tex = texture("app".into(), &img, egui::TextureOptions::LINEAR);
            self.icons.insert("app", (tex.clone(), tex));
        }
    }

    // ── 快捷鍵 ──────────────────────────────────────────────────

    /// 只在文字欄位有焦點時也允許的命令（不會與文字欄位自身的編輯快捷鍵衝突）
    fn allowed_in_text_field(c: C) -> bool {
        matches!(
            c,
            C::New | C::Open | C::Save | C::SaveAs | C::SaveAll | C::Close | C::Exit | C::Find | C::FindNext | C::FindPrevious
                | C::Replace | C::QuickFind | C::FindInFiles | C::GoToLine | C::GoToPosition | C::NextWindow
                | C::PreviousWindow | C::ToggleWindow | C::FullScreen | C::Print | C::RecentFilesList | C::RunScript | C::SwitchTab1
                | C::SwitchTab2 | C::SwitchTab3 | C::SwitchTab4 | C::SwitchTab5 | C::SwitchTab6 | C::SwitchTab7
                | C::SwitchTab8 | C::SwitchLastTab
        )
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let focused = ctx.memory(|m| m.focused());
        let editor_focus = !self.is_preview_tab() && (focused.is_none() || self.editor_has_focus(ctx));
        let mut fired: Vec<C> = Vec::new();
        let mut alt_fired = false;
        let mut pasted: Option<String> = None;
        let bindings = self.bindings.clone();
        let capturing = matches!(self.dialog, Some(Dialog::Options(ref o)) if o.capturing.is_some());
        if capturing || self.confirm.is_some() {
            return;
        }
        ctx.input_mut(|i| {
            i.events.retain(|ev| match ev {
                Event::Key { key, pressed: true, modifiers, .. } => {
                    let hit = bindings
                        .iter()
                        .find(|(sc, _)| sc.logical_key == *key && commands::modifiers_match(*modifiers, sc.modifiers));
                    match hit {
                        Some((sc, c)) if editor_focus || Self::allowed_in_text_field(*c) => {
                            fired.push(*c);
                            alt_fired |= sc.modifiers.alt;
                            false
                        }
                        _ => true,
                    }
                }
                Event::Copy if editor_focus => {
                    fired.push(C::Copy);
                    false
                }
                Event::Cut if editor_focus => {
                    fired.push(C::Cut);
                    false
                }
                Event::Paste(s) if editor_focus => {
                    pasted = Some(s.clone());
                    fired.push(C::Paste);
                    false
                }
                _ => true,
            });
            if alt_fired || !fired.is_empty() {
                // macOS 上 Option+按鍵會另外產生特殊字元的文字事件；Windows 上 Ctrl+Shift+[ 等
                // 組合鍵也會產生「{」的文字事件。快捷鍵觸發時一併丟棄同幀的文字輸入。
                i.events.retain(|ev| !matches!(ev, Event::Text(_)));
            }
        });
        if let Some(s) = pasted {
            self.set_editor_clipboard(&s);
        }
        for c in fired {
            self.run(ctx, c);
        }
    }

    fn editor_has_focus(&self, ctx: &egui::Context) -> bool {
        let Some(f) = ctx.memory(|m| m.focused()) else { return false };
        let doc_id = self.doc().id;
        f == view::editor_id(doc_id) || f == hexview::hex_id(doc_id)
    }

    // ── 剪貼簿 ──────────────────────────────────────────────────

    pub fn copy_to_system(&mut self, text: &str) {
        if let Some(cb) = self.clipboard.as_mut() {
            let _ = cb.set_text(text.to_string());
        }
    }

    pub fn system_clipboard(&mut self) -> Option<String> {
        self.clipboard.as_mut().and_then(|cb| cb.get_text().ok())
    }

    /// 以系統剪貼簿內容更新編輯器剪貼簿（內容相同時保留列塊旗標）
    fn set_editor_clipboard(&mut self, text: &str) {
        let ed = &mut self.doc_mut().editor;
        let normalized = text.replace("\r\n", "\n");
        if ed.clipboard != normalized {
            ed.clipboard = normalized;
            ed.clipboard_is_block = false;
        }
    }

    fn sync_clipboard_from_system(&mut self) {
        if let Some(t) = self.system_clipboard() {
            self.set_editor_clipboard(&t);
        }
    }

    // ── 定時工作 ────────────────────────────────────────────────

    /// 檢查分頁在磁碟上是否被其他程式修改；開啟「自動重新載入」時，沒有未存檔修改的文件
    /// 直接重讀。有未存檔修改或檔案已不存在時仍保留提示，避免默默丟失編輯內容。
    fn check_disk_changes(&mut self) {
        let just_enabled = self.settings.auto_reload_external && !self.auto_reload_was_on;
        self.auto_reload_was_on = self.settings.auto_reload_external;
        let mut changed = Vec::new();
        for tab in &mut self.tabs {
            if !tab.disk_changed && tab.doc.changed_on_disk() {
                tab.disk_changed = true;
                changed.push(tab.doc.id);
            }
        }
        if !self.settings.auto_reload_external {
            return;
        }
        if just_enabled {
            // 剛開啟選項：也處理已經顯示提示、且沒有未存檔修改的分頁。
            changed.extend(self.tabs.iter().filter(|tab| tab.disk_changed).map(|tab| tab.doc.id));
            changed.sort_unstable();
            changed.dedup();
        }
        for id in changed {
            let Some(i) = self.index_of(id) else { continue };
            let exists = self.tabs[i].doc.path.as_ref().is_some_and(|p| p.exists());
            if exists && !self.tabs[i].doc.is_modified() {
                self.reload_tab(id);
            }
        }
    }

    fn periodic(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        if now - self.last_disk_check > 1.5 {
            self.last_disk_check = now;
            self.check_disk_changes();
        }
        if self.settings.auto_save && now - self.last_autosave > self.settings.auto_save_minutes.max(1) as f64 * 60.0 {
            self.last_autosave = now;
            let ids: Vec<u64> = self.tabs.iter().filter(|t| t.doc.path.is_some() && t.doc.is_modified() && !t.disk_changed).map(|t| t.doc.id).collect();
            for id in ids {
                self.save_with(id, None, false);
            }
        }
        if let Some((_, t)) = &mut self.status {
            if *t == 0.0 {
                *t = now;
            } else if now - *t > 8.0 {
                self.status = None;
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(1500));
    }

    fn save_session(&mut self) {
        if self.screenshot.is_some() {
            return;
        }
        for i in 0..self.tabs.len() {
            self.remember_caret(i);
        }
        self.settings.session_files = self
            .tabs
            .iter()
            .filter_map(|t| {
                let p = t.doc.path.clone()?;
                let h = t.doc.editor.selection.head;
                Some(SessionFile { path: p, line: h.line, col: h.col })
            })
            .collect();
        self.settings.session_active = self.active;
        if self.settings.purge_history_on_exit {
            self.settings.purge_history();
        }
        if let Err(e) = self.settings.save() {
            eprintln!("無法儲存設定：{e}");
        }
    }

    // ── 介面元件 ────────────────────────────────────────────────

    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        // 沒有分頁時仍保留標籤列高度，避免面板在開啟檔案前後跳動。
        ui.set_min_height(form::TAB_HEIGHT);
        let mut activate = None;
        let mut close = None;
        let mut close_preview = None;
        let mut menu_cmd: Option<(usize, C)> = None;
        let unsaved = self.t("st.unsaved", "尚未儲存").to_string();
        let preview_title = self.t("md.title", "Markdown 預覽").to_string();
        let preview_active = self.is_preview_tab();
        let close_tip = self.t("btn.close", "關閉").to_string();
        let close_preview_tip = self.t("md.close", "關閉預覽").to_string();
        let copy_path = self.t("tab.copy_path", "複製完整路徑").to_string();
        // 選取的分頁和下面的編輯區同色才會連成一體：暗色下編輯區用統一的面板底色（見
        // view::paint 的背景規則），淺色沿用語法主題底色；十六進位模式用 Hex 檢視的底色。
        let dark = Self::is_dark(ui.ctx());
        let content_bg = match self.doc().mode {
            EditMode::Hex => ui.visuals().extreme_bg_color,
            _ if dark => ui.visuals().panel_fill,
            _ => {
                let c = crate::core::syntax::theme_colors(&self.syntax_theme(dark), dark).background;
                Color32::from_rgb(c.0, c.1, c.2)
            }
        };
        let strip = ui.max_rect();
        let mut active_rect = None;
        // 激活分頁變化時（例如從「開啟」清單點選），若該分頁超出可見範圍則自動捲動到可見。
        let want_tab_scroll = self.tab_scroll_done != Some((self.active, preview_active));
        // 拖動分頁調整順序（MTT 風格）：拖動期間不改變順序，只做純繪製預覽——
        // 來源分頁浮起跟隨指針（帶陰影、置頂），鄰居動畫滑開一個空位；鬆手才提交搬移。
        let mut drag_stopped_flag = false;
        // 每個分頁的 (索引, 矩形, 繪製形狀區間起訖)，供幀末平移形狀實現拖動預覽
        let mut tab_shapes: Vec<(usize, egui::Rect, egui::layers::ShapeIdx, egui::layers::ShapeIdx)> = Vec::new();
        egui::ScrollArea::horizontal().id_salt("tabs").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(8.0);
                for (i, t) in self.tabs.iter().enumerate() {
                    let mut name = t.doc.display_name();
                    if t.doc.is_modified() {
                        name.push_str(" *");
                    }
                    if t.doc.read_only {
                        name.push_str(" 🔒");
                    }
                    let selected = i == self.active && !preview_active;
                    let tooltip = t.doc.path.as_ref().map(|p| crate::core::display_path(p)).unwrap_or_else(|| unsaved.clone());
                    let close_name = format!("{close_tip} {}", t.doc.display_name());
                    let shape_from = next_shape_idx(ui);
                    let form::FlatTab { rect, body, close: close_btn } =
                        form::flat_tab(ui, ("doc-tab", i), &name, selected, content_bg, Some(&close_name));
                    let shape_to = next_shape_idx(ui);
                    tab_shapes.push((i, rect, shape_from, shape_to));
                    if selected {
                        active_rect = Some(rect);
                    }
                    let body = body.on_hover_text(tooltip);
                    if body.clicked() {
                        activate = Some((i, false));
                    }
                    // 拖動排序：按下即記錄來源分頁；繪製預覽在 for 迴圈後統一處理
                    if body.drag_started() {
                        egui::DragAndDrop::set_payload(ui.ctx(), i);
                    }
                    if body.drag_stopped() {
                        drag_stopped_flag = true;
                    }
                    // 左鍵雙擊分頁標題即關閉這個分頁（未儲存的變更仍會先詢問）。
                    if body.middle_clicked() || body.double_clicked() {
                        close = Some(t.doc.id);
                    }
                    body.context_menu(|ui| {
                        ui.set_min_width(menus::SUBMENU_MIN_WIDTH);
                        for c in [C::Save, C::SaveAs, C::Reload, C::Close, C::CloseOthers, C::CloseLeft, C::CloseRight, C::CloseAll] {
                            if ui.button(menus::check_label(false, &self.cmd_text(c))).clicked() {
                                menu_cmd = Some((i, c));
                                ui.close();
                            }
                        }
                        // 開啟檔案所在資料夾（未存檔的文件沒有資料夾可開）
                        if ui
                            .add_enabled(t.doc.path.is_some(), egui::Button::new(menus::check_label(false, &self.cmd_text(C::OpenFolder))))
                            .clicked()
                        {
                            menu_cmd = Some((i, C::OpenFolder));
                            ui.close();
                        }
                        // 複製完整路徑，緊接在「開啟資料夾」之後（Windows 上不含 `\\?\` 前綴）
                        if let Some(p) = &t.doc.path
                            && ui.button(menus::check_label(false, &copy_path)).clicked()
                        {
                            ui.ctx().copy_text(crate::core::display_path(p));
                            ui.close();
                        }
                        // 預覽：勾選式總開關，依副檔名自動分發到 Markdown／HTML／圖形預覽
                        ui.separator();
                        if ui
                            .add(egui::Button::new(menus::check_label(self.settings.auto_preview, &self.cmd_text(C::PreviewAuto))))
                            .clicked()
                        {
                            menu_cmd = Some((i, C::PreviewAuto));
                            ui.close();
                        }
                    });
                    if close_btn.is_some_and(|btn| btn.clicked()) {
                        close = Some(t.doc.id);
                    }
                    // A document's Markdown preview tab follows it, in the same flat style.
                    if t.md_preview.as_ref().is_some_and(|pv| pv.placement == mdpreview::Placement::Tab) {
                        let name = format!("{} — {preview_title}", t.doc.display_name());
                        let selected = i == self.active && preview_active;
                        let close_name = format!("{close_preview_tip} {}", t.doc.display_name());
                        let shape_from = next_shape_idx(ui);
                        let form::FlatTab { rect, body, close: close_btn } =
                            form::flat_tab(ui, ("preview-tab", i), &name, selected, content_bg, Some(&close_name));
                        let shape_to = next_shape_idx(ui);
                        tab_shapes.push((i, rect, shape_from, shape_to));
                        if selected {
                            active_rect = Some(rect);
                        }
                        if body.clicked() {
                            activate = Some((i, true));
                        }
                        // 預覽分頁同樣可拖動（與其文件分頁一起移動）
                        if body.drag_started() {
                            egui::DragAndDrop::set_payload(ui.ctx(), i);
                        }
                        if body.drag_stopped() {
                            drag_stopped_flag = true;
                        }
                        // 雙擊預覽分頁標題同樣關閉它。
                        if body.middle_clicked() || body.double_clicked() || close_btn.is_some_and(|btn| btn.clicked()) {
                            close_preview = Some(i);
                        }
                    }
                }
                ui.add_space(8.0);
                // 拖動預覽：來源分頁浮起跟隨指針，鄰居滑開空位（不改變實際順序）
                paint_tab_drag(ui, &tab_shapes, strip);
                // 激活分頁變化的那一幀，把它捲動到可見（帶 [usize; bool] 之外再取一次 rect 以免借用衝突）
                if want_tab_scroll
                    && let Some(rect) = active_rect
                {
                    // 僅當橫向不完全可見時才捲動，避免每次切換都強制移動使用者的捲動位置
                    let clip = ui.clip_rect();
                    if rect.left() < clip.left() || rect.right() > clip.right() {
                        ui.scroll_to_rect(rect, None);
                    }
                }
            });
        });
        if want_tab_scroll {
            self.tab_scroll_done = Some((self.active, preview_active));
        }
        form::tab_strip_baseline(ui, strip, active_rect);
        // ── 拖動結束：鬆手才提交搬移（繪製預覽不改變順序）──
        let drag_payload = egui::DragAndDrop::payload::<usize>(ui.ctx()).map(|p| *p);
        if let Some(src) = drag_payload
            && drag_stopped_flag
            && let Some(from) = tab_shapes.iter().position(|&(i, ..)| i == src)
            && let Some(pos) = ui.ctx().input(|i| i.pointer.interact_pos())
            && strip.contains(pos)
        {
            // 指針所在位置 = 插入空位（以分頁中點為界）
            let to = tab_shapes
                .iter()
                .position(|&(_, r, ..)| pos.x < r.center().x)
                .unwrap_or(self.tabs.len());
            // 移除來源後再插入：gap 在來源右側時索引要減一
            let dst = if to > from { to - 1 } else { to };
            if dst != from {
                let tab = self.tabs.remove(from);
                self.tabs.insert(dst, tab);
                // 激活索引跟著搬移的分頁走
                let a = self.active;
                self.active = if a == from {
                    dst
                } else if from < a && a <= dst {
                    a - 1
                } else if dst <= a && a < from {
                    a + 1
                } else {
                    a
                };
                // 搬移後本幀繪製時記錄的 activate／close 索引已過期，丟棄以免誤觸
                activate = None;
                close = None;
                close_preview = None;
                menu_cmd = None;
            }
        }
        // 拖動結束清掉 payload；拖動期間游標保持預設（分頁本身在跟隨指針）
        if drag_stopped_flag {
            egui::DragAndDrop::clear_payload(ui.ctx());
        }
        if let Some((i, preview)) = activate {
            self.activate_view(i, preview);
        }
        if let Some(i) = close_preview {
            self.tabs[i].md_preview = None;
            if i == self.active {
                self.preview_tab_active = false;
                self.focus_editor = true;
            }
        }
        if let Some(id) = close {
            self.request_close(vec![id], false);
        }
        if let Some((i, c)) = menu_cmd {
            self.activate(i);
            self.active = i;
            self.pending.push(c);
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {        if self.tabs.is_empty() {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(self.t("st.no_document", "沒有開啟的檔案")).weak());
                if let Some((msg, _)) = &self.status {
                    ui.separator();
                    ui.label(msg);
                }
            });
            return;
        }
        // 先把文件狀態抄出來，避免翻譯時同時借用文件
        let snap = {
            let doc = self.doc_mut();
            let position = if doc.mode == EditMode::Hex {
                doc.hex.as_ref().map(|hb| {
                    let sel = hb.selection().map(|(s, e)| e - s).unwrap_or(0);
                    (format!("{:X}h", hb.caret), format!("{:X}h", hb.len()), sel)
                })
            } else {
                None
            };
            let text = if doc.mode != EditMode::Hex {
                let ed = &mut doc.editor;
                let h = ed.selection.head;
                let block = if ed.selection.is_block() {
                    let b = ed.selection.visual_bounds(ed);
                    Some(((b.max_line - b.min_line + 1).to_string(), (b.max_vcol - b.min_vcol).to_string()))
                } else {
                    None
                };
                let (vcol, char_pos, sel_chars) = ed.status_info(doc.version);
                Some((h.line + 1, ed.lines.len(), vcol + 1, char_pos, sel_chars, block))
            } else {
                None
            };
            (
                position,
                text,
                doc.has_bom,
                doc.newline,
                doc.mixed_newlines,
                doc.encoding.name().to_string(),
                doc.syntax.clone(),
                doc.mode,
                doc.read_only,
                if doc.mode == EditMode::Hex { doc.hex.as_ref().is_some_and(|h| h.overwrite) } else { doc.editor.overwrite_mode },
                doc.selecting_mode,
            )
        };
        let (position, text, has_bom, newline, mixed_newlines, encoding, syntax, mode, read_only, overwrite, selecting) = snap;
        let recording = self.macros.recording;
        let status = self.status.clone();
        ui.horizontal(|ui| {
            if let Some((caret, len, sel)) = position {
                ui.monospace(self.t("st.offset", "位移 {n} / {q}").replace("{n}", &caret).replace("{q}", &len));
                ui.separator();
                ui.monospace(self.tf("st.sel", "選取 {n}", Some(sel), None));
            } else if let Some((line, total, col, chars, sel, block)) = text {
                let total = total.to_string();
                ui.monospace(format!(
                    "{}  {}",
                    self.tf("st.line", "行 {n}/{q}", Some(line), Some(&total)),
                    self.tf("st.col", "欄 {n}", Some(col), None)
                ));
                ui.separator();
                ui.monospace(self.tf("st.char", "字元 {n}", Some(chars), None));
                ui.separator();
                if let Some((rows, cols)) = block {
                    ui.monospace(self.t("st.block", "列塊 {n}×{q}").replace("{n}", &rows).replace("{q}", &cols));
                } else {
                    ui.monospace(self.tf("st.sel", "選取 {n}", Some(sel), None));
                }
            }
            ui.separator();
            let bom = if has_bom { ".BOM" } else { "" };
            let nl = match newline {
                NewLineFormat::Dos => "CRLF",
                NewLineFormat::Unix => "LF",
                NewLineFormat::Mac => "CR",
            };
            let mixed = if mixed_newlines { self.t("st.mixed", "(混合)") } else { "" };
            ui.monospace(format!("{encoding}{bom} {nl}{mixed}"));
            ui.separator();
            ui.monospace(syntax);
            ui.separator();
            ui.monospace(match mode {
                EditMode::Text => self.t("st.text", "文字"),
                EditMode::Column => self.t("st.column", "列塊"),
                EditMode::Hex => self.t("st.hex", "十六進位"),
            });
            ui.separator();
            ui.monospace(if read_only { self.t("st.readonly", "唯讀") } else { self.t("st.writable", "可寫") });
            ui.separator();
            ui.monospace(if overwrite { "OVR" } else { "INS" });
            if selecting {
                ui.separator();
                ui.colored_label(ui.visuals().warn_fg_color, self.t("st.selecting", "選取中"));
            }
            if recording {
                ui.separator();
                ui.colored_label(ui.visuals().error_fg_color, self.t("st.recording", "● 錄製巨集"));
            }
            if let Some((msg, _)) = &status {
                ui.separator();
                ui.label(msg);
            }
        });
    }

    /// 外部修改通知橫幅（P1-10：只在使用者選擇重新載入或忽略時消失）
    fn disk_banner(&mut self, ui: &mut egui::Ui) {
        let t = &self.tabs[self.active];
        if !t.disk_changed {
            return;
        }
        let id = t.doc.id;
        let modified = t.doc.is_modified();
        let exists = t.doc.path.as_ref().is_some_and(|p| p.exists());
        // Amber warning banner in both themes: pale amber with dark text in light mode,
        // deep amber with light text in dark mode, so the message stays readable either way.
        let (banner_bg, banner_fg, banner_danger) = if ui.visuals().dark_mode {
            (Color32::from_rgb(74, 52, 16), Color32::from_rgb(255, 228, 176), Color32::from_rgb(255, 160, 140))
        } else {
            (Color32::from_rgb(255, 244, 214), Color32::from_rgb(72, 48, 8), Color32::from_rgb(168, 32, 24))
        };
        let banner = egui::Frame::new().fill(banner_bg).inner_margin(egui::Margin::symmetric(10, 6)).corner_radius(8.0);
        banner.show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.colored_label(
                    banner_fg,
                    format!(
                        "⚠ {}",
                        if exists {
                            self.t("banner.modified", "檔案已被其他程式修改。")
                        } else {
                            self.t("banner.deleted", "檔案已被其他程式刪除或移動。")
                        }
                    ),
                );
                if modified {
                    ui.colored_label(banner_danger, self.t("banner.lose", "重新載入會遺失目前未存檔的修改。"));
                }
                if exists && ui.button(self.t("btn.reload", "重新載入")).clicked() {
                    if modified {
                        self.confirm = Some(Confirm::ReloadModified { id });
                    } else {
                        self.reload_tab(id);
                    }
                }
                if ui.button(self.t("btn.ignore", "忽略")).clicked() {
                    let t = &mut self.tabs[self.active];
                    t.doc.ignore_disk_change();
                    t.disk_changed = false;
                }
            });
        });
    }

    /// 所有分頁關閉後的空白工作區：不自動建立新檔、不結束程式，只提示如何開始。
    fn empty_workspace(&mut self, ui: &mut egui::Ui) {
        let hint = self.t("empty.hint", "使用「檔案 → 新增」或「開啟舊檔」開始，也可以從左側「文件視圖」開啟檔案。");
        let new_label = commands::plain_label(&self.cmd_text(C::New));
        let open_label = commands::plain_label(&self.cmd_text(C::Open));
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.28).max(24.0));
            ui.label(egui::RichText::new(self.t("st.no_document", "沒有開啟的檔案")).size(18.0).strong());
            ui.add_space(6.0);
            ui.add(egui::Label::new(egui::RichText::new(hint).weak()).wrap());
            ui.add_space(14.0);
            let button_pad = ui.spacing().button_padding.x * 2.0;
            let spacing = ui.spacing().item_spacing.x;
            let measure = |text: &str| {
                ui.painter().layout_no_wrap(text.to_string(), egui::TextStyle::Button.resolve(ui.style()), ui.visuals().text_color()).size().x + button_pad
            };
            let row_width = measure(&new_label) + measure(&open_label) + spacing;
            let left = ((ui.available_width() - row_width) / 2.0).max(0.0);
            ui.horizontal(|ui| {
                ui.add_space(left);
                if ui.button(new_label).clicked() {
                    self.pending.push(C::New);
                }
                if ui.button(open_label).clicked() {
                    self.pending.push(C::Open);
                }
            });
        });
    }

    fn editor_area(&mut self, ui: &mut egui::Ui) {
        let dark = Self::is_dark(ui.ctx());
        let theme = self.syntax_theme(dark);
        editor::set_tab_size(self.settings.tab_size);
        let fold_label = self.t("fold.collapsed", "⋯ {n} 行");
        let idx = self.active;
        let mut out = ViewOutput::default();
        let tab = &mut self.tabs[idx];
        let family = self.fonts.editor_family.clone();
        let focus_now = std::mem::take(&mut self.focus_editor);
        let child = ui;
        if tab.doc.mode == EditMode::Hex {
            let focused = hexview::show(child, &mut tab.doc, &mut tab.hex, &self.settings, &mut self.widths, &family, dark);
            out.focused = focused;
            if focus_now {
                child.memory_mut(|m| m.request_focus(hexview::hex_id(tab.doc.id)));
            }
        } else {
            let mut env = ViewEnv {
                settings: &self.settings,
                widths: &mut self.widths,
                spell: self.spell.as_ref(),
                theme_name: &theme,
                is_dark: dark,
                editor_family: &family,
                fold_label: &fold_label,
            };
            view::show(child, &mut tab.doc, &mut tab.view, &mut env, &mut out);
            if focus_now {
                child.memory_mut(|m| m.request_focus(view::editor_id(tab.doc.id)));
            }
        }

        if self.macros.recording {
            for a in &out.actions {
                self.macros.record_action(a.clone());
            }
        }
        if let Some(t) = out.copy {
            self.copy_to_system(&t);
        }
        if out.paste {
            self.sync_clipboard_from_system();
            let fill = self.settings.auto_fill_column_paste;
            let doc = self.doc_mut();
            let hint = doc.to_end_hint();
            doc.edit(crate::core::document::EditKind::Other, hint, |ed| ed.paste_from_clipboard(fill));
        }
        if let Some(w) = out.add_word {
            if !self.settings.personal_dictionary.contains(&w) {
                self.settings.personal_dictionary.push(w.clone());
            }
            if let Some(s) = self.spell.as_mut() {
                s.add_word(&w);
            }
        }
        self.pending.extend(out.commands);
    }
}

/// Modern look shared by the light and dark styles: rounded widgets and windows, softer
/// shadows and one accent blue. Background colors stay on egui's own light / dark palette
/// so dialogs, menus and panels all match.
///
/// Vertical spacing keeps the old compact values (padding 2, spacing 3): the Edit menu has
/// dozens of rows, and although long menus scroll (`menus::menu`), taller rows would make
/// them scroll on an ordinary laptop screen.
fn install_style(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        style.spacing.button_padding = egui::vec2(6.0, 2.0);
        style.spacing.item_spacing = egui::vec2(6.0, 3.0);
        style.spacing.window_margin = egui::Margin::symmetric(12, 10);
        let visuals = &mut style.visuals;
        let radius = egui::CornerRadius::same(6);
        visuals.widgets.inactive.corner_radius = radius;
        visuals.widgets.hovered.corner_radius = radius;
        visuals.widgets.active.corner_radius = radius;
        visuals.widgets.open.corner_radius = radius;
        visuals.window_corner_radius = egui::CornerRadius::same(10);
        visuals.menu_corner_radius = egui::CornerRadius::same(8);
        let shadow = if visuals.dark_mode { 96 } else { 32 };
        visuals.window_shadow = egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(shadow) };
        visuals.popup_shadow = egui::Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(shadow) };
        // Warning / error text (status bar, dialogs, find bar) must stay readable as small text:
        // egui's default light-mode orange is only about 3:1 against the panel; these reach 4.5:1+.
        if visuals.dark_mode {
            visuals.warn_fg_color = Color32::from_rgb(255, 170, 60);
            visuals.error_fg_color = Color32::from_rgb(255, 110, 100);
            visuals.selection.bg_fill = Color32::from_rgb(46, 92, 160);
            visuals.selection.stroke.color = Color32::from_rgb(226, 236, 255);
            visuals.hyperlink_color = Color32::from_rgb(140, 186, 255);
        } else {
            visuals.warn_fg_color = Color32::from_rgb(160, 80, 0);
            visuals.error_fg_color = Color32::from_rgb(190, 30, 30);
            // The hex view and the editor fallback paint selections at 60% of this color on
            // white, so it must be strong enough to stay visible after that fade.
            visuals.selection.bg_fill = Color32::from_rgb(160, 200, 255);
            visuals.selection.stroke.color = Color32::from_rgb(12, 52, 120);
            visuals.hyperlink_color = Color32::from_rgb(16, 90, 180);
        }
    });
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 最小化時 eframe 不呼叫 ui；開檔與還原必須在仍會執行的 logic 處理。
        let requests = self.instance.as_ref().map(|instance| instance.requests()).unwrap_or_default();
        for files in requests {
            self.open_external_files(ctx, files);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        #[cfg(target_os = "macos")]
        for path in crate::macos::take_open_files() {
            self.open_path(&path);
        }
        #[cfg(target_os = "macos")]
        if crate::macos::take_quit_requested() {
            // 系統選單與 Dock 的結束請求也必須經過未儲存確認。
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.pending.push(C::Exit);
        }

        // The fonts set last frame are active now. Widths measured in that frame came from the
        // old font, so measure again; and a proportional font in fixed-width mode overlaps
        // wide letters such as "m", so suggest turning the mode off.
        if self.fonts_changed {
            self.fonts_changed = false;
            self.widths.invalidate();
            if self.settings.fixed_width && self.confirm.is_none() && fonts::editor_is_proportional(&ctx) {
                self.confirm = Some(Confirm::ProportionalFont);
            }
        }
        // 字型：編輯區或介面字型設定改變時重建
        let wanted_fonts = (self.settings.font_family.clone(), self.settings.ui_font_family.clone());
        if self.font_applied.as_ref() != Some(&wanted_fonts) {
            self.fonts.apply(&ctx, &wanted_fonts.0, &wanted_fonts.1);
            self.font_applied = Some(wanted_fonts);
            self.fonts_changed = true;
        }

        // 拖放開檔
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        for p in dropped {
            self.open_path(&p);
        }

        // 關閉視窗：有未存檔修改時先詢問
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            let ids = self.tabs.iter().map(|t| t.doc.id).collect();
            self.request_close(ids, true);
        }
        if self.allow_close {
            self.save_session();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // 記錄視窗尺寸（全螢幕時不記錄），下次啟動沿用
        if !self.fullscreen
            && let Some(r) = ctx.input(|i| i.viewport().inner_rect)
        {
            self.settings.window_size = Some((r.width(), r.height()));
        }

        // Ctrl/⌘ + 滾輪縮放字型
        let zoom = ctx.input(|i| if i.modifiers.command { i.smooth_scroll_delta().y } else { 0.0 });
        // 滑鼠在並排的 Markdown 預覽上時，Ctrl+滾輪交給預覽自己縮放
        let over_preview = self.is_preview_tab()
            || self.tabs.get(self.active).and_then(|tab| tab.md_preview.as_ref()).is_some_and(|p| p.docked() && ui.rect_contains_pointer(p.rect));
        if zoom != 0.0 && ui.rect_contains_pointer(ui.max_rect()) && !over_preview {
            self.settings.font_size = (self.settings.font_size + zoom.signum()).clamp(6.0, 72.0);
            self.save_settings();
        }

        // 視窗重新取得焦點時立即檢查外部修改：剛在其他程式存檔後切回來不必等下一個週期。
        let focused = ctx.input(|i| i.focused);
        if focused && !self.was_focused {
            self.last_disk_check = ctx.input(|i| i.time);
            self.check_disk_changes();
        }
        self.was_focused = focused;

        self.handle_shortcuts(&ctx);
        self.poll_macro(&ctx);
        let pending = std::mem::take(&mut self.pending);
        for c in pending {
            self.run(&ctx, c);
        }
        self.periodic(&ctx);
        self.poll_updates(&ctx);

        let show_bars = !self.post_it;
        self.quick_in_menubar = false;
        #[cfg(windows)]
        {
            // Windows 自繪標題欄：圖示 + 選單列 + 快捷控制 + 視窗按鈕
            //（便利貼模式只保留圖示與視窗按鈕）。面板由 `title_bar` 自行建立並給定高度。
            self.title_bar(ui, _frame);
        }
        #[cfg(not(windows))]
        if self.settings.show_menubar && show_bars {
            egui::Panel::top("menubar").show(ui, |ui| self.menu_bar(ui));
        }
        if self.settings.show_toolbar && show_bars {
            // 不畫面板分隔線：標題欄（含選單列）與工具列之間不需要一條橫線。
            egui::Panel::top("toolbar").show_separator_line(false).show(ui, |ui| self.toolbar(ui));
        }
        // 狀態列先配置，才會橫跨整個視窗；文件視圖在其上方、緊接工具列底部，
        // 接下來的分頁列與搜尋結果都只佔編輯區寬度，與編輯器左右並排。
        if self.settings.show_statusbar && show_bars {
            egui::Panel::bottom("statusbar").show(ui, |ui| self.status_bar(ui));
        }
        self.update_banner(ui);
        if show_bars && self.settings.show_file_view {
            // 寬度依視窗大小收斂，避免擠掉編輯區；上限放寬到 800（深目錄可見更多層級）。
            let max_w = (ui.available_width() - 180.0).max(160.0);
            let want = self.settings.file_view_width.clamp(160.0, 800.0).min(max_w);
            let shown = egui::Panel::left("file-view")
                .frame(egui::Frame::new())
                .resizable(true)
                .default_size(want)
                .min_size(160.0)
                .max_size(max_w.min(800.0))
                .show(ui, |ui| self.file_view_panel(ui));
            let width = shown.response.rect.width();
            if (width - self.settings.file_view_width).abs() > 1.0 {
                self.settings.file_view_width = width;
                // 拖曳結束才寫回設定檔，不要每一幀都寫入。
                if ui.ctx().input(|i| i.pointer.any_released()) {
                    self.save_settings();
                }
            }
        }
        if show_bars {
            let strip = form::tab_strip_bg(ui);
            let frame = egui::Frame::new().fill(strip).inner_margin(egui::Margin { left: 0, right: 0, top: 2, bottom: 0 });
            egui::Panel::top("tabs").show_separator_line(false).frame(frame).show(ui, |ui| self.tab_bar(ui));
        }
        if self.fif.show_results {
            egui::Panel::bottom("fif-results").resizable(true).default_size(180.0).show(ui, |ui| self.find_results_panel(ui));
        }
        if self.runner.show {
            egui::Panel::bottom("script-output").resizable(true).default_size(180.0).show(ui, |ui| self.script_output_panel(ui));
        }
        // 並排面板先分配空間；獨立預覽分頁則在中央區域繪製。
        if self.tabs.get(self.active).and_then(|tab| tab.md_preview.as_ref()).is_some_and(|pv| pv.docked()) {
            self.show_markdown_preview(ui);
        }
        if self.tabs.get(self.active).and_then(|tab| tab.svg_preview.as_ref()).is_some_and(|pv| pv.docked()) {
            self.show_svg_preview(ui);
        }
        if self.tabs.get(self.active).and_then(|tab| tab.img_preview.as_ref()).is_some_and(|pv| pv.docked()) {
            self.show_img_preview(ui);
        }
        // 分隔線的拖曳範圍會跨入編輯區；留出抓取空間，避免稍後建立的捲軸搶走事件。
        let mut editor_margin = egui::Margin::ZERO;
        if let Some(pv) = self.tabs.get(self.active).and_then(|tab| tab.md_preview.as_ref()) {
            let gap = ui.style().interaction.resize_grab_radius_side.ceil() as i8 + 1;
            match pv.placement {
                mdpreview::Placement::Right => editor_margin.right = gap,
                mdpreview::Placement::Bottom => editor_margin.bottom = gap,
                mdpreview::Placement::Tab => {}
            }
        }
        if let Some(pv) = self.tabs.get(self.active).and_then(|tab| tab.svg_preview.as_ref()) {
            let gap = ui.style().interaction.resize_grab_radius_side.ceil() as i8 + 1;
            match pv.placement {
                mdpreview::Placement::Right => editor_margin.right = gap,
                mdpreview::Placement::Bottom => editor_margin.bottom = gap,
                mdpreview::Placement::Tab => {}
            }
        }
        if let Some(pv) = self.tabs.get(self.active).and_then(|tab| tab.img_preview.as_ref()) {
            let gap = ui.style().interaction.resize_grab_radius_side.ceil() as i8 + 1;
            match pv.placement {
                mdpreview::Placement::Right => editor_margin.right = gap,
                mdpreview::Placement::Bottom => editor_margin.bottom = gap,
                mdpreview::Placement::Tab => {}
            }
        }
        // Opaque panel color: the find / go-to bars and the gaps around them must not show the
        // window's clear color, which is dark even in light mode.
        let editor_frame = egui::Frame::new().inner_margin(editor_margin).fill(ui.visuals().panel_fill);
        egui::CentralPanel::default().frame(editor_frame).show(ui, |ui| {
            if self.tabs.is_empty() {
                self.empty_workspace(ui);
            } else if self.is_preview_tab() {
                self.show_markdown_preview(ui);
            } else if self.is_svg_preview_tab() {
                self.show_svg_preview(ui);
            } else if self.is_img_preview_tab() {
                self.show_img_preview(ui);
            } else {
                self.disk_banner(ui);
                self.find_bar(ui);
                self.goto_bar(ui);
                self.editor_area(ui);
            }
        });

        self.show_dialogs(&ctx);
        self.show_print_preview(&ctx);
        self.show_confirm(&ctx);
        // 收藏群組「取代開啟」：關閉流程（含儲存確認）全部結束後接續開啟群組
        if self.confirm.is_none()
            && self.pending_close.is_none()
            && let Some(gi) = self.pending_open_group.take()
        {
            self.open_group_now(gi);
        }
        self.fif.poll(&ctx);
        if let Some(err) = self.runner.poll(&ctx) {
            self.set_status(format!("{} {err}", self.t("run.input_fail", "無法傳送到程序（可能已結束）：")));
        }
        // 「執行輸出」面板也會被面板上的 × 與執行腳本（自動開啟）改變顯示狀態，
        // 這裡統一同步到設定，下次啟動沿用最後的顯示/隱藏。
        if self.runner.show != self.settings.show_script_output {
            self.settings.show_script_output = self.runner.show;
            self.save_settings();
        }

        // 測試用截圖
        if let Some((path, frames)) = self.screenshot.as_mut() {
            *frames += 1;
            // 視窗尚未可見時截圖請求會被丟棄，因此每 30 幀重試一次
            if *frames % 30 == 8 {
                let slot = self.shot_slot.clone();
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
                    if let Ok(mut g) = slot.lock() {
                        *g = Some(img);
                    }
                })));
            }
            let shot = self.shot_slot.lock().ok().and_then(|mut g| g.take());
            if let Some(img) = shot {
                let [w, h] = img.size;
                let raw: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                if let Some(buf) = image::RgbaImage::from_raw(w as u32, h as u32, raw)
                    && let Err(e) = buf.save(path.as_path()) {
                        eprintln!("無法儲存截圖：{e}");
                    }
                self.allow_close = true;
            }
            ctx.request_repaint();
        }
    }

    fn on_exit(&mut self) {
        self.fif.stop_worker();
        self.save_session();
    }
}

impl App {
    /// Windows 自繪標題欄：圖示 + 選單列 + 快捷控制 + 視窗按鈕。
    /// 高度由選單自然高推導（× 1.5），拖動、雙擊最大化與按鈕全部由 egui 處理；
    /// 原生子類（`titlebar` 模組）只負責客戶區邊緣縮放，因此懸停與點擊不會互相搶事件。
    #[cfg(windows)]
    fn title_bar(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        use raw_window_handle::HasWindowHandle;
        if !self.titlebar_installed
            && let Ok(handle) = frame.window_handle()
            && let raw_window_handle::RawWindowHandle::Win32(win) = handle.as_raw()
        {
            titlebar::install(win.hwnd.get() as isize);
            self.titlebar_installed = true;
        }

        let maximized = ui.ctx().input(|i| i.viewport().maximized).unwrap_or(false);
        // 快捷控制項跟著選單列走：隱藏選單列時移交給工具列（與非 Windows 行為一致）。
        let quick_shown = self.settings.show_menubar && !self.post_it && ui.available_width() >= menus::QUICK_CONTROLS_WIDTH;
        let menus_shown = self.settings.show_menubar && !self.post_it;
        let icon = self.app_icon.get_or_insert_with(|| load_app_icon(ui.ctx())).clone();
        // 欄高 = 選單按鈕自然高 × 1.5。自然高由字型與樣式推導（字高 + 上下內距，且不小於
        // 互動尺寸），不測量已佈局的結果，因此不會出現逐幀回饋放大。
        let text_h = ui.text_style_height(&egui::TextStyle::Button);
        let menu_h = (text_h + 2.0 * ui.spacing().button_padding.y).max(ui.spacing().interact_size.y);
        // 欄（容器）高度 = 選單自然高 × 1.5：固定給面板，避免 `ui.set_height` 只撐大內容
        // 卻不改變面板保留高度，導致選單列下緣被工具列蓋住。
        let bar_height = (menu_h * 1.5).max(24.0);
        // 吸附到整數物理像素：×1.5 的高度在小數點後留下碎片時（某些縮放比例下），標題欄
        // 與工具列面板的接縫會露出一條 wgpu 清除色（黑線，深淺主題都出現）。對齊後嚴絲合縫。
        let ppp = ui.ctx().pixels_per_point();
        let bar_height = ((bar_height * ppp).ceil() / ppp).max(24.0);
        // 圖示邊長（與欄高等比縮放，保持視覺平衡）
        let icon_size = (bar_height * 0.5).clamp(16.0, 32.0);

        egui::Panel::top("titlebar").exact_size(bar_height).show_separator_line(false).show(ui, |ui| {
            // 拖動區：`interact` 只註冊互動矩形、不佔用版面（`allocate_rect` 會推進游標，
            // 把選單列推出面板高度，導致點擊落空）。先註冊者在下，後面的按鈕優先取得點擊。
            let drag = ui.interact(ui.max_rect(), ui.scope_id().with("titlebar-drag"), egui::Sense::click_and_drag());
            if drag.drag_started() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            if drag.double_clicked() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            }
            // 內容：右側叢集釘右緣，左側圖示 + 選單列自然由左至右。
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // 視窗按鈕：Win11 風格的整欄平面按鈕（`form::caption_button` 以線段畫字形，
                // 懸停整格變色、關閉鍵變紅），相鄰不留間距，與原生標題欄一致。
                let click = |ui: &mut egui::Ui, icon: form::CaptionIcon, name: &str| {
                    form::caption_button(ui, icon, name, name).clicked()
                };
                let spacing_x = ui.spacing().item_spacing.x;
                ui.spacing_mut().item_spacing.x = 0.0;
                if click(ui, form::CaptionIcon::Close, &self.t("btn.close", "關閉")) {
                    // 與原生關閉一致：走 close_requested 流程（未存檔先詢問）。
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if click(ui, if maximized { form::CaptionIcon::Restore } else { form::CaptionIcon::Maximize }, &self.t("btn.maximize", "最大化／還原")) {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                }
                if click(ui, form::CaptionIcon::Minimize, &self.t("btn.minimize", "最小化")) {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
                ui.spacing_mut().item_spacing.x = spacing_x;
                if quick_shown {
                    ui.scope(|ui| {
                        self.quick_in_menubar = true;
                        self.quick_controls(ui, true);
                    });
                }
                // 圖示與選單佔用剩餘寬度，並明確由左至右排列；
                // `MenuBar` 提供「點開一個後懸停自動切換」。
                // 便利貼模式與「隱藏選單列」時只保留圖示。
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let (icon_rect, _) = ui.allocate_exact_size(egui::vec2(icon_size, icon_size), egui::Sense::hover());
                    ui.painter().image(
                        icon.id(),
                        icon_rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                    if menus_shown {
                        egui::MenuBar::new().ui(ui, |ui| self.menus(ui));
                    }
                });
            });
        });
    }
}

/// 目前圖層的下一個形狀索引（記錄每個分頁的繪製區間，供拖動預覽平移形狀）。
fn next_shape_idx(ui: &egui::Ui) -> egui::layers::ShapeIdx {
    ui.ctx().graphics_mut(|graphics| graphics.entry(ui.layer_id()).next_idx())
}

/// 拖動預覽（MTT 風格，純繪製、不改變實際順序）：
/// 來源分頁浮起跟隨指針（帶陰影、置頂），鄰居以動畫滑開一個與來源等寬的空位。
/// 來源與鄰居的形狀是在 `tab_bar` 裡記錄的繪製區間，這裡按偏移量平移它們。
fn paint_tab_drag(
    ui: &egui::Ui,
    tabs: &[(usize, egui::Rect, egui::layers::ShapeIdx, egui::layers::ShapeIdx)],
    strip: egui::Rect,
) {
    let Some(source) = egui::DragAndDrop::payload::<usize>(ui.ctx()).map(|p| *p) else {
        return;
    };
    let Some(src_pos) = tabs.iter().position(|&(i, ..)| i == source) else {
        return;
    };
    let Some(pointer) = ui.input(|i| i.pointer.interact_pos()) else {
        return;
    };
    let released = ui.input(|i| i.pointer.primary_released());
    // 指針離開分頁列（如拖到編輯區）時不顯示鄰居滑動，只保留來源跟隨
    let row = tabs.first().zip(tabs.last()).map(|(f, l)| f.1.union(l.1));
    let gap = row
        .filter(|row| row.contains(pointer) && !released)
        .map(|_| tabs.iter().position(|&(_, r, ..)| pointer.x < r.center().x).unwrap_or(tabs.len()));
    let mut lifted = Vec::new();
    for (pos, &(index, rect, from, to)) in tabs.iter().enumerate() {
        let is_source = pos == src_pos;
        let delta = if is_source {
            // 跟隨指針：保持按下時的抓取偏移，橫向限制在分頁列內
            let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or(rect.center());
            let x = (rect.left() + pointer.x - origin.x).clamp(strip.left(), (strip.right() - rect.width()).max(strip.left()));
            egui::vec2(x - rect.left(), 0.0)
        } else {
            match gap {
                Some(gap) => {
                    let target = neighbor_offset(pos, src_pos, gap, tabs);
                    let animated = ui.ctx().animate_value_with_time(egui::Id::unique(("tab-drag", index)), target, 0.12);
                    egui::vec2(animated, 0.0)
                }
                None => egui::Vec2::ZERO,
            }
        };
        if delta == egui::Vec2::ZERO && !is_source {
            continue;
        }
        ui.ctx().graphics_mut(|graphics| {
            let list = graphics.entry(ui.layer_id());
            for shape in from.0..to.0 {
                list.mutate_shape(egui::layers::ShapeIdx(shape), |entry| {
                    entry.shape.translate(delta);
                    if is_source {
                        lifted.push(entry.clone());
                        entry.shape = egui::Shape::Noop;
                    }
                });
            }
        });
        if is_source {
            // 浮起的來源分頁加陰影，最後重新加入以覆蓋滑動中的鄰居
            let raised = rect.translate(delta);
            let painter = ui.painter();
            painter.add(
                egui::epaint::Shadow {
                    offset: [0, 2],
                    blur: 8,
                    spread: 1,
                    color: egui::Color32::from_black_alpha(65),
                }
                .as_shape(raised, 8),
            );
        }
    }
    for entry in lifted {
        ui.ctx().graphics_mut(|graphics| {
            graphics
                .entry(ui.layer_id())
                .add(entry.clip_rect, entry.shape);
        });
    }
}

/// 鄰居位移量：來源左側 index∈[gap,src) 右移一個槽位，右側 index∈(src,gap) 左移一個槽位。
/// 槽位寬度 = 來源寬度 + 與下一格之間的間距（參考 MTT-File-Manager 的同名實現）。
fn neighbor_offset(
    index: usize,
    source: usize,
    gap: usize,
    tabs: &[(usize, egui::Rect, egui::layers::ShapeIdx, egui::layers::ShapeIdx)],
) -> f32 {
    if index < source && index >= gap {
        tabs[source].1.width() + (tabs[index + 1].1.left() - tabs[index].1.right())
    } else if index > source && index < gap {
        -(tabs[source].1.width() + (tabs[index].1.left() - tabs[index - 1].1.right()))
    } else {
        0.0
    }
}
