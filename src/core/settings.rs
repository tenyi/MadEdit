//! 使用者設定（對應原版「選項」對話框與 MadEdit.cfg），以 JSON 存於系統設定目錄

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::favorites::FavoriteGroup;

/// 自動換行模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum WrapMode {
    #[default]
    NoWrap,
    ByWindow,
    ByColumn,
}

/// 開檔時的檢視模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum OpenViewMode {
    /// 依檔案內容自動判斷（二進位檔以十六進位開啟）
    #[default]
    Auto,
    Text,
    Column,
    Hex,
}

/// 深淺色主題
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

/// 介面語言。`System` 跟隨作業系統；其餘為固定語言。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum UiLanguage {
    #[default]
    #[serde(rename = "system")]
    System,
    #[serde(rename = "en")]
    En,
    #[serde(rename = "zh-TW")]
    ZhTw,
    #[serde(rename = "zh-CN")]
    ZhCn,
    #[serde(rename = "ja")]
    Ja,
    #[serde(rename = "ko")]
    Ko,
    #[serde(rename = "de")]
    De,
    #[serde(rename = "es")]
    Es,
    #[serde(rename = "fr")]
    Fr,
    #[serde(rename = "it")]
    It,
    #[serde(rename = "pt")]
    Pt,
}

impl UiLanguage {
    pub const ALL: [UiLanguage; 11] = [
        UiLanguage::System,
        UiLanguage::En,
        UiLanguage::ZhTw,
        UiLanguage::ZhCn,
        UiLanguage::Ja,
        UiLanguage::Ko,
        UiLanguage::De,
        UiLanguage::Es,
        UiLanguage::Fr,
        UiLanguage::It,
        UiLanguage::Pt,
    ];

    /// 語言在選單中的名稱。除「跟隨系統」外一律用該語言自己的寫法，方便在看不懂目前介面時切換。
    pub fn native_name(self) -> &'static str {
        match self {
            UiLanguage::System => "System",
            UiLanguage::En => "English",
            UiLanguage::ZhTw => "繁體中文",
            UiLanguage::ZhCn => "简体中文",
            UiLanguage::Ja => "日本語",
            UiLanguage::Ko => "한국어",
            UiLanguage::De => "Deutsch",
            UiLanguage::Es => "Español",
            UiLanguage::Fr => "Français",
            UiLanguage::It => "Italiano",
            UiLanguage::Pt => "Português",
        }
    }
}

/// 工作階段中的一個檔案（啟動時重新開啟）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionFile {
    pub path: PathBuf,
    pub line: usize,
    pub col: usize,
}

/// 排序選項
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct SortOptions {
    pub descending: bool,
    pub case_sensitive: bool,
    pub numeric: bool,
    pub remove_duplicates: bool,
}


/// 全形／半形轉換選項
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WidthOptions {
    pub letters: bool,
    pub digits: bool,
    pub symbols: bool,
    pub space: bool,
    pub katakana: bool,
}

impl Default for WidthOptions {
    fn default() -> Self {
        Self {
            letters: true,
            digits: true,
            symbols: true,
            space: true,
            katakana: true,
        }
    }
}

/// 插入遞增數字選項
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NumberOptions {
    pub initial: i64,
    pub step: i64,
    pub total: usize,
    /// 0 十進位、1 十六進位、2 八進位、3 二進位
    pub format: u8,
    pub uppercase_hex: bool,
    pub align_left: bool,
    pub zero_pad: bool,
    pub prefix: String,
    pub postfix: String,
}

impl Default for NumberOptions {
    fn default() -> Self {
        Self {
            initial: 1,
            step: 1,
            total: 10,
            format: 0,
            uppercase_hex: true,
            align_left: false,
            zero_pad: false,
            prefix: String::new(),
            postfix: String::new(),
        }
    }
}

/// 列印（輸出 HTML 後交給瀏覽器列印）選項
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PrintOptions {
    pub syntax_colors: bool,
    pub line_numbers: bool,
    pub bookmarks: bool,
    pub end_of_line: bool,
    pub tab_chars: bool,
    pub space_chars: bool,
    pub header: bool,
    pub header_left: String,
    pub header_center: String,
    pub header_right: String,
    pub footer: bool,
    pub footer_left: String,
    pub footer_center: String,
    pub footer_right: String,
    /// 十六進位模式列印位移標頭：0 無、1 僅第一頁、2 每頁
    pub hex_offset_header: u8,
}

impl Default for PrintOptions {
    fn default() -> Self {
        Self {
            syntax_colors: true,
            line_numbers: true,
            bookmarks: false,
            end_of_line: false,
            tab_chars: false,
            space_chars: false,
            header: true,
            header_left: "%f".into(),
            header_center: String::new(),
            header_right: "%n/%s".into(),
            footer: true,
            footer_left: String::new(),
            footer_center: "%d %t".into(),
            footer_right: String::new(),
            hex_offset_header: 1,
        }
    }
}

/// 在多個檔案中尋找的設定
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FindInFilesOptions {
    pub directory: String,
    pub file_filter: String,
    pub exclude_filter: String,
    pub subdirectories: bool,
    /// 只在已開啟的檔案中搜尋
    pub opened_files_only: bool,
}

impl Default for FindInFilesOptions {
    fn default() -> Self {
        Self {
            directory: String::new(),
            file_filter: "*".into(),
            exclude_filter: String::new(),
            subdirectories: true,
            opened_files_only: false,
        }
    }
}

/// 資源管理器頁籤狀態（跨階段記住位置與外觀）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ExplorerOptions {
    /// ▣ 分欄模式（樹下方顯示所選資料夾內容列表）
    pub split: bool,
    /// 上半部樹佔分欄高度的比例（拖拽分隔條後記住）
    pub ratio: f32,
    /// 生效中的檔名篩選詞（按「>」或 Enter 後的值）
    pub filter: String,
    /// 目前選中的資料夾
    pub selected: Option<String>,
    /// 展開中的資料夾（絕對路徑清單）
    pub expanded: Vec<String>,
    /// 分欄列表排序欄：0=名稱 1=修改日期 2=類型 3=大小
    pub sort_column: u8,
    /// 分欄列表是否降冪
    pub sort_descending: bool,
    /// 分欄列表欄寬 [名稱, 修改日期, 類型, 大小]；名稱 0 = 依內容自動
    pub col_widths: Vec<f32>,
}

impl Default for ExplorerOptions {
    fn default() -> Self {
        Self { split: false, ratio: 0.5, filter: String::new(), selected: None, expanded: Vec::new(), sort_column: 0, sort_descending: false, col_widths: Vec::new() }
    }
}

/// 全部設定
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {    pub auto_check_updates: bool,
    pub last_update_check: u64,
    pub skipped_update: String,
    // ── 檢視 ──
    /// 編輯區字型（空字串表示自動挑選）
    pub font_family: String,
    pub font_size: f32,
    /// 介面字型（選單、對話框、分頁等；空字串表示自動）。
    /// 不影響編輯區與「執行輸出」面板，兩者固定使用編輯區字型
    pub ui_font_family: String,
    /// 「執行輸出」面板的「輸出到檔案（result.log）」勾選狀態（跨階段記住，重開程式沿用）
    pub output_log_to_file: bool,
    /// 「執行輸出」面板是否顯示（檢視 → 運行輸出）：記住使用者最後的顯示/隱藏狀態
    pub show_script_output: bool,
    /// 以圖形預覽（SVG）勾選狀態：開啟時，打開 .svg 直接以圖形顯示
    pub svg_auto_preview: bool,
    /// 以圖形預覽（圖片）勾選狀態：開啟時，打開 .bmp/.jpg/.jpeg 等直接以圖形顯示
    pub img_auto_preview: bool,
    /// 「以 Markdown 預覽」勾選狀態：開啟時，打開 .md 檔直接進入預覽
    pub md_auto_preview: bool,
    /// 等寬模式：每個字元依固定格子寬度排列（關閉時依字型實際字寬排列）
    pub fixed_width: bool,
    pub tab_size: usize,
    pub indent_size: usize,
    /// 行距百分比（100–250）
    pub line_spacing: u32,
    pub wrap_mode: WrapMode,
    pub wrap_column: usize,
    pub show_line_numbers: bool,
    pub show_bookmarks: bool,
    /// 代碼折疊：左側 gutter 顯示折疊三角，支援折疊/展開大括號區塊
    pub code_folding: bool,
    pub show_col_hint: bool,
    pub col_hint_position: usize,
    pub show_ruler: bool,
    pub show_eol: bool,
    pub show_tab: bool,
    pub show_space: bool,
    pub mark_active_line: bool,
    pub mark_brace_pair: bool,
    pub right_to_left: bool,
    pub typewriter_mode: bool,
    pub spell_check: bool,
    pub spell_language: String,
    pub theme: ThemeChoice,
    /// 介面語言；預設跟隨作業系統
    pub ui_language: UiLanguage,
    pub syntax_theme_light: String,
    pub syntax_theme_dark: String,
    pub show_toolbar: bool,
    pub show_menubar: bool,
    pub show_statusbar: bool,
    pub always_on_top: bool,

    // ── 收藏 ──
    /// 收藏群組；每個群組可放多個檔案，面板可一次開啟整組
    pub favorites: Vec<FavoriteGroup>,
    /// 面板上的檔案只顯示檔名，不顯示完整路徑
    pub favorites_show_names_only: bool,
    /// 「最近開啟的檔案」區塊是否展開
    pub favorites_recent_expanded: bool,
    /// 左側「文件視圖」面板是否顯示（檢視 → 文件視圖）
    pub show_file_view: bool,
    /// 文件視圖面板寬度（使用者拖曳分隔線後記住）
    pub file_view_width: f32,
    /// 資源管理器頁籤狀態（分欄、篩選、選中位置與展開節點，跨階段記住）
    pub explorer: ExplorerOptions,

    // ── 一般 ──
    /// 新文件與「強制使用預設編碼」採用的編碼
    pub default_encoding: String,
    /// 內容不是合法 UTF-8 時優先嘗試的編碼
    pub fallback_encoding: String,
    pub enforce_default_encoding: bool,
    /// 超過此大小（MB）的檔案只以十六進位模式開啟
    pub max_text_file_mb: u64,
    pub open_view_mode: OpenViewMode,
    pub max_search_result_chars: usize,
    pub reload_files_on_startup: bool,
    pub restore_caret_position: bool,
    /// 檔案被其他程式修改時，沒有未儲存修改的文件直接重新載入（關閉時只顯示提示）
    pub auto_reload_external: bool,
    pub auto_save: bool,
    pub auto_save_minutes: u32,
    pub auto_backup: bool,
    pub purge_history_on_exit: bool,

    // ── 編輯 ──
    pub date_time_format: String,
    pub insert_spaces_for_tab: bool,
    pub auto_indent: bool,
    pub auto_complete_pair: bool,
    /// 自動補全成對字元時，若有選取則以成對字元包住選取內容（否則取代選取）
    pub wrap_selection_with_pair: bool,
    pub auto_copy_mouse_selection: bool,
    pub middle_click_paste: bool,
    pub auto_fill_column_paste: bool,
    pub double_click_highlight: bool,

    // ── 新文件 ──
    pub new_doc_encoding: String,
    pub new_doc_bom: bool,
    /// "CRLF" / "LF" / "CR"
    pub new_doc_newline: String,
    pub new_doc_syntax: String,

    // ── 其他 ──
    pub print: PrintOptions,
    pub sort: SortOptions,
    pub width: WidthOptions,
    pub numbers: NumberOptions,
    pub find_in_files: FindInFilesOptions,
    /// 快捷鍵覆寫：命令識別碼 → 按鍵清單（如 "Ctrl+Shift+K"）
    pub key_bindings: HashMap<String, Vec<String>>,
    pub astyle_options: String,
    pub search_match_case: bool,
    pub search_whole_word: bool,
    pub search_regex: bool,
    pub search_wrap_around: bool,
    pub search_in_selection: bool,

    // ── 歷史 ──
    pub recent_files: Vec<PathBuf>,
    pub recent_finds: Vec<String>,
    pub recent_replaces: Vec<String>,
    pub recent_dirs: Vec<String>,
    pub session_files: Vec<SessionFile>,
    pub session_active: usize,
    /// 各檔案上次的游標位置
    pub caret_positions: HashMap<String, (usize, usize)>,
    pub window_size: Option<(f32, f32)>,
    pub personal_dictionary: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            auto_check_updates: true,
            last_update_check: 0,
            skipped_update: String::new(),
            font_family: String::new(),
            font_size: 14.0,
            ui_font_family: String::new(),
            output_log_to_file: false,
            show_script_output: false,
            svg_auto_preview: false,
            img_auto_preview: false,
            md_auto_preview: false,
            fixed_width: false,
            tab_size: 4,
            indent_size: 4,
            line_spacing: 100,
            wrap_mode: WrapMode::NoWrap,
            wrap_column: 80,
            show_line_numbers: true,
            show_bookmarks: true,
            code_folding: true,
            show_col_hint: false,
            col_hint_position: 80,
            show_ruler: true,
            show_eol: false,
            show_tab: false,
            show_space: false,
            mark_active_line: true,
            mark_brace_pair: true,
            right_to_left: false,
            typewriter_mode: false,
            spell_check: false,
            spell_language: "en_US".into(),
            theme: ThemeChoice::System,
            ui_language: UiLanguage::System,
            syntax_theme_light: "InspiredGitHub".into(),
            syntax_theme_dark: "base16-ocean.dark".into(),
            show_toolbar: true,
            show_menubar: true,
            show_statusbar: true,
            always_on_top: false,

            favorites: Vec::new(),
            favorites_show_names_only: true,
            favorites_recent_expanded: false,
            // 預設關閉：面板是新的工作區，從「檢視 → 文件視圖」開啟
            show_file_view: false,
            file_view_width: 240.0,
            explorer: ExplorerOptions::default(),

            default_encoding: "UTF-8".into(),
            fallback_encoding: "Big5".into(),
            enforce_default_encoding: false,
            max_text_file_mb: 100,
            open_view_mode: OpenViewMode::Auto,
            max_search_result_chars: 100,
            reload_files_on_startup: true,
            restore_caret_position: true,
            auto_reload_external: false,
            auto_save: false,
            auto_save_minutes: 10,
            auto_backup: false,
            purge_history_on_exit: false,

            date_time_format: "%Y-%m-%d %H:%M:%S".into(),
            insert_spaces_for_tab: false,
            auto_indent: true,
            auto_complete_pair: false,
            wrap_selection_with_pair: true,
            auto_copy_mouse_selection: false,
            middle_click_paste: cfg!(target_os = "linux"),
            auto_fill_column_paste: true,
            double_click_highlight: true,

            new_doc_encoding: "UTF-8".into(),
            new_doc_bom: false,
            new_doc_newline: if cfg!(windows) { "CRLF".into() } else { "LF".into() },
            new_doc_syntax: "Plain Text".into(),

            print: PrintOptions::default(),
            sort: SortOptions::default(),
            width: WidthOptions::default(),
            numbers: NumberOptions::default(),
            find_in_files: FindInFilesOptions::default(),
            key_bindings: HashMap::new(),
            astyle_options: "--style=allman --indent=spaces=4".into(),
            search_match_case: false,
            search_whole_word: false,
            search_regex: false,
            search_wrap_around: true,
            search_in_selection: false,

            recent_files: Vec::new(),
            recent_finds: Vec::new(),
            recent_replaces: Vec::new(),
            recent_dirs: Vec::new(),
            session_files: Vec::new(),
            session_active: 0,
            caret_positions: HashMap::new(),
            window_size: None,
            personal_dictionary: Vec::new(),
        }
    }
}

/// 歷史清單上限
const MAX_RECENT: usize = 20;

/// 設定檔讀不到時的資料。介面再用目前語言組成完整句子
pub struct SettingsLoadError {
    /// 備份成功時為備份路徑，失敗時為備份錯誤
    pub backup: Result<PathBuf, String>,
    pub detail: String,
}

impl Settings {
    /// These live settings with the changes a dialog made to its copy: every top-level field
    /// where `edited` differs from `base` (the copy as the dialog started) takes `edited`'s
    /// value, and all other fields keep their live value. The Options dialog is not modal, so
    /// while it is open the recent files, View toggles, zoom and so on keep changing; copying
    /// its whole draft back would revert them.
    pub fn with_edits(&self, base: &Settings, edited: &Settings) -> Settings {
        use serde_json::{to_value, Value};
        let (Ok(Value::Object(base)), Ok(Value::Object(edited)), Ok(Value::Object(mut live))) = (to_value(base), to_value(edited), to_value(self)) else {
            return edited.clone();
        };
        for (key, value) in edited {
            if base.get(&key) != Some(&value) {
                live.insert(key, value);
            }
        }
        serde_json::from_value(Value::Object(live)).unwrap_or_else(|_| self.clone())
    }

    /// 設定檔路徑（例如 macOS 的 ~/Library/Application Support/MadEdit/settings.json）
    pub fn path() -> Option<PathBuf> {
        // 單元測試不讀寫使用者的設定檔
        if cfg!(test) {
            return None;
        }
        // 便攜版與原生整合測試可使用獨立目錄，避免覆寫日常工作的設定與工作階段。
        if let Some(dir) = std::env::var_os("MADEDIT_CONFIG_DIR").filter(|d| !d.is_empty()) {
            return Some(PathBuf::from(dir).join("settings.json"));
        }
        dirs::config_dir().map(|d| d.join("MadEdit").join("settings.json"))
    }

    /// 載入設定；檔案不存在或格式錯誤時使用預設值
    pub fn load() -> Self {
        Self::load_checked().0
    }

    /// 載入設定，並回傳需要告知使用者的錯誤（設定檔損毀時已另存備份）
    pub fn load_checked() -> (Self, Option<SettingsLoadError>) {
        match Self::path() {
            Some(p) => Self::load_from(&p),
            None => (Self::default(), None),
        }
    }

    /// 自指定路徑載入；檔案不存在時靜默使用預設值，讀取或解析失敗時先把原檔複製為 `.bak`
    /// 再使用預設值，避免之後存回設定時永久覆蓋使用者的原始設定
    fn load_from(path: &Path) -> (Self, Option<SettingsLoadError>) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Self::default(), None),
            Err(e) => return (Self::default(), Some(Self::corrupt_message(path, &e.to_string()))),
        };
        match serde_json::from_str::<Self>(&text) {
            Ok(mut s) => {
                s.migrate_encodings(&text);
                (s, None)
            }
            Err(e) => (Self::default(), Some(Self::corrupt_message(path, &e.to_string()))),
        }
    }

    /// 舊版設定沒有 fallback 編碼，且預設編碼（如 Big5）實際上只是偵測失敗時的備援：
    /// 把它移到 fallback，預設編碼改為 UTF-8。強制使用預設編碼的使用者維持原設定。
    fn migrate_encodings(&mut self, raw: &str) {
        let has_fallback = serde_json::from_str::<serde_json::Value>(raw).is_ok_and(|v| v.get("fallback_encoding").is_some());
        if has_fallback || self.enforce_default_encoding {
            return;
        }
        if self.default_encoding != "UTF-8" {
            self.fallback_encoding = std::mem::replace(&mut self.default_encoding, "UTF-8".into());
        }
    }

    /// 備份無法使用的設定檔。介面再依目前語言組成提示
    fn corrupt_message(path: &Path, err: &str) -> SettingsLoadError {
        let mut bak = path.as_os_str().to_owned();
        bak.push(".bak");
        let bak = PathBuf::from(bak);
        SettingsLoadError {
            backup: match std::fs::copy(path, &bak) {
                Ok(_) => Ok(bak),
                Err(e) => Err(e.to_string()),
            },
            detail: err.to_string(),
        }
    }

    /// 寫回設定檔
    pub fn save(&self) -> std::io::Result<()> {
        match Self::path() {
            Some(p) => self.save_to(&p),
            None => Ok(()),
        }
    }

    /// 以原子寫入存到指定路徑，寫入途中中止也不會留下截斷的設定檔
    fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        super::document::atomic_write(path, |f| std::io::Write::write_all(f, json.as_bytes()))
    }

    /// 將項目移到歷史清單最前面
    pub fn push_history<T: PartialEq + Clone>(list: &mut Vec<T>, item: T) {
        list.retain(|x| *x != item);
        list.insert(0, item);
        list.truncate(MAX_RECENT);
    }

    pub fn add_recent_file(&mut self, path: PathBuf) {
        Self::push_history(&mut self.recent_files, path);
    }

    /// 清除所有歷史紀錄
    pub fn purge_history(&mut self) {
        self.recent_files.clear();
        self.recent_finds.clear();
        self.recent_replaces.clear();
        self.recent_dirs.clear();
        self.caret_positions.clear();
    }

    /// 記住檔案的游標位置（最多保留 500 筆）
    pub fn remember_caret(&mut self, path: &std::path::Path, line: usize, col: usize) {
        if self.caret_positions.len() > 500 {
            self.caret_positions.clear();
        }
        self.caret_positions
            .insert(path.to_string_lossy().into_owned(), (line, col));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("madedit_settings_{}_{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn test_with_edits_keeps_live_changes_the_dialog_did_not_touch() {
        let base = Settings::default();
        let mut edited = base.clone();
        edited.tab_size = 8;
        edited.print.line_numbers = !base.print.line_numbers;
        let mut live = base.clone();
        live.recent_files.push(PathBuf::from("/tmp/opened-meanwhile.txt"));
        live.show_ruler = !base.show_ruler;
        live.tab_size = 2;
        let merged = live.with_edits(&base, &edited);
        assert_eq!(merged.tab_size, 8, "the dialog's change wins");
        assert_eq!(merged.print.line_numbers, edited.print.line_numbers);
        assert_eq!(merged.recent_files, live.recent_files);
        assert_eq!(merged.show_ruler, live.show_ruler);
        // Nothing edited: the live settings come back unchanged.
        let same = live.with_edits(&base, &base);
        assert_eq!(serde_json::to_value(&same).unwrap(), serde_json::to_value(&live).unwrap());
    }

    #[test]
    fn test_legacy_default_encoding_moves_to_fallback() {
        let path = std::env::temp_dir().join(format!("madedit-enc-migrate-{}.json", std::process::id()));
        std::fs::write(&path, r#"{"default_encoding":"Big5"}"#).unwrap();
        let (s, err) = Settings::load_from(&path);
        let _ = std::fs::remove_file(&path);
        assert!(err.is_none());
        assert_eq!(s.default_encoding, "UTF-8");
        assert_eq!(s.fallback_encoding, "Big5");
    }

    #[test]
    fn test_settings_save_and_load_roundtrip() {
        let dir = temp_dir("roundtrip");
        let p = dir.join("settings.json");
        let mut s = Settings::default();
        s.add_recent_file(PathBuf::from("/a.txt"));
        s.theme = ThemeChoice::Dark;
        s.save_to(&p).unwrap();
        let (loaded, err) = Settings::load_from(&p);
        assert!(err.is_none());
        assert_eq!(loaded.recent_files, vec![PathBuf::from("/a.txt")]);
        // The theme picked last time is the one the next launch starts in.
        assert_eq!(loaded.theme, ThemeChoice::Dark);
        // 檔案不存在：靜默使用預設值
        let (missing, err) = Settings::load_from(&dir.join("missing.json"));
        assert!(err.is_none());
        // Without a settings file the theme follows the system.
        assert_eq!(missing.theme, ThemeChoice::System);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_favorites_roundtrip_and_legacy_settings_without_the_field() {
        let mut s = Settings::default();
        let mut group = FavoriteGroup::new("工作");
        group.add(PathBuf::from("/work/a.txt"));
        group.add(PathBuf::from("/work/b.txt"));
        s.favorites.push(group);
        s.favorites_show_names_only = false;
        s.show_file_view = true;
        s.file_view_width = 320.0;
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.favorites, s.favorites);
        assert!(!back.favorites_show_names_only);
        assert!(back.show_file_view);
        assert_eq!(back.file_view_width, 320.0);
        // 舊版設定檔沒有收藏欄位：使用預設值（面板關閉、僅顯示名稱）
        let legacy: Settings = serde_json::from_str(r#"{"tab_size": 8}"#).unwrap();
        assert!(legacy.favorites.is_empty());
        assert!(legacy.favorites_show_names_only);
        assert!(!legacy.show_file_view);
    }

    #[test]
    fn test_auto_reload_external_defaults_off_and_roundtrips() {
        assert!(!Settings::default().auto_reload_external, "既有行為是顯示提示，不能預設自動覆蓋");
        let mut s = Settings::default();
        s.auto_reload_external = true;
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert!(back.auto_reload_external);
        // 舊版設定檔沒有這個欄位：維持提示流程
        let legacy: Settings = serde_json::from_str(r#"{"tab_size": 8}"#).unwrap();
        assert!(!legacy.auto_reload_external);
    }

    #[test]
    fn test_explorer_options_roundtrip_and_legacy_defaults() {
        let mut s = Settings::default();
        s.explorer.split = true;
        s.explorer.ratio = 0.7;
        s.explorer.filter = "*.rs".into();
        s.explorer.selected = Some(r"D:\work".into());
        s.explorer.expanded = vec![r"D:\".into(), r"D:\work".into()];
        s.explorer.col_widths = vec![180.0, 115.0, 60.0, 80.0];
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert!(back.explorer.split);
        assert_eq!(back.explorer.ratio, 0.7);
        assert_eq!(back.explorer.filter, "*.rs");
        assert_eq!(back.explorer.selected.as_deref(), Some(r"D:\work"));
        assert_eq!(back.explorer.expanded, vec![r"D:\".to_string(), r"D:\work".to_string()]);
        assert_eq!(back.explorer.col_widths, vec![180.0, 115.0, 60.0, 80.0], "欄寬要寫入並讀回設定");
        // 舊版設定檔沒有 explorer 欄位：全部回到預設（不分欄、無篩選、無選中）
        let legacy: Settings = serde_json::from_str(r#"{"tab_size": 8}"#).unwrap();
        assert_eq!(legacy.explorer, ExplorerOptions::default());
    }

    #[test]
    fn test_explorer_options_tolerate_missing_new_fields() {
        // 前一個版本寫出的 explorer 物件沒有排序欄位：必須正常載入，
        // 不能因缺欄位被判成「設定檔損壞」（那會把使用者的設定改名成 .bak 並回到預設）。
        let legacy = r#"{"explorer": {"split": true, "ratio": 0.6, "filter": "*.rs", "selected": "D:\\work", "expanded": ["D:\\"]}}"#;
        let s: Settings = serde_json::from_str(legacy).expect("缺少排序欄位的舊設定必須能載入");
        assert!(s.explorer.split);
        assert_eq!(s.explorer.ratio, 0.6);
        assert_eq!(s.explorer.filter, "*.rs");
        assert_eq!(s.explorer.selected.as_deref(), Some(r"D:\work"));
        assert_eq!(s.explorer.expanded, vec![r"D:\".to_string()]);
        // 缺少的新欄位回退預設：名稱升冪
        assert_eq!((s.explorer.sort_column, s.explorer.sort_descending), (0, false));
    }

    #[test]
    fn test_svg_auto_preview_defaults_off_and_roundtrips() {
        assert!(!Settings::default().svg_auto_preview, "預設不自動開啟圖形預覽");
        assert!(!Settings::default().md_auto_preview, "預設不自動開啟 Markdown 預覽");
        let mut s = Settings::default();
        s.svg_auto_preview = true;
        s.img_auto_preview = true;
        s.md_auto_preview = true;
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert!(back.svg_auto_preview, "勾選狀態必須寫入並讀回設定");
        assert!(back.img_auto_preview, "圖片預覽勾選狀態必須寫入並讀回設定");
        assert!(back.md_auto_preview, "Markdown 預覽勾選狀態必須寫入並讀回設定");
        // 舊版設定檔沒有這個欄位：維持關閉
        let legacy: Settings = serde_json::from_str(r#"{"tab_size": 8}"#).unwrap();
        assert!(!legacy.svg_auto_preview);
        assert!(!legacy.img_auto_preview);
        assert!(!legacy.md_auto_preview);
    }

    #[test]
    fn test_corrupt_settings_are_backed_up_and_reported() {
        let dir = temp_dir("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("settings.json");
        let truncated = r#"{"recent_files": ["/a.txt""#;
        std::fs::write(&p, truncated).unwrap();
        let (loaded, err) = Settings::load_from(&p);
        assert!(err.is_some());
        assert!(loaded.recent_files.is_empty());
        // 原始內容保留在 .bak，之後存回預設值也不會遺失
        loaded.save_to(&p).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("settings.json.bak")).unwrap(), truncated);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_settings_roundtrip_and_partial_json() {
        let mut s = Settings { font_size: 18.0, ui_font_family: "Segoe UI".into(), output_log_to_file: true, show_script_output: true, ..Default::default() };
        s.add_recent_file(PathBuf::from("/a"));
        s.add_recent_file(PathBuf::from("/b"));
        s.add_recent_file(PathBuf::from("/a"));
        assert_eq!(s.recent_files, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.font_size, 18.0);
        assert_eq!(back.ui_font_family, "Segoe UI");
        assert!(back.output_log_to_file, "the log toggle persists");
        assert!(back.show_script_output, "the Script Output panel state persists");
        // 舊版設定檔缺少欄位時以預設值補上
        let partial: Settings = serde_json::from_str(r#"{"tab_size": 8}"#).unwrap();
        assert_eq!(partial.tab_size, 8);
        assert!(partial.show_line_numbers);
        assert!(partial.ui_font_family.is_empty(), "missing ui_font_family means automatic");
        assert!(!partial.output_log_to_file, "missing log toggle defaults to off");
        assert!(!partial.show_script_output, "missing Script Output state defaults to hidden");
    }
}
