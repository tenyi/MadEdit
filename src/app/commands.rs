//! 命令表（對應原版 MadEdit 的 CommandTable）：識別碼、選單文字與預設快捷鍵，以及快捷鍵解析

use eframe::egui::{Key, KeyboardShortcut, Modifiers};

/// 定義所有可綁定快捷鍵的命令。預設快捷鍵中 "Ctrl" 代表 macOS 的 ⌘、其他平台的 Ctrl；
/// 多個快捷鍵以 "|" 分隔。
macro_rules! commands {
    ($( $name:ident => $id:literal, $label:literal, $key:literal; )*) => {
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        pub enum C { $($name),* }

        pub const ALL: &[C] = &[$(C::$name),*];

        impl C {
            /// 穩定識別碼（存於設定檔的快捷鍵與巨集中）
            pub fn id(self) -> &'static str { match self { $(C::$name => $id),* } }
            /// 選單與對話框顯示的名稱
            pub fn label(self) -> &'static str { match self { $(C::$name => $label),* } }
            /// 預設快捷鍵
            pub fn default_keys(self) -> &'static str {
                // 保留其他平台的原版配置；macOS 避開隱藏、最小化、關閉等系統組合鍵。
                if cfg!(target_os = "macos") {
                    match self {
                        C::Close => return "Ctrl+W",
                        C::Exit => return "Ctrl+Q",
                        C::SaveAs => return "Ctrl+Shift+S",
                        C::SaveAll => return "Ctrl+Alt+S",
                        C::Replace => return "Ctrl+Alt+F",
                        C::Comment => return "Ctrl+Slash",
                        C::Uncomment => return "Ctrl+Shift+Slash",
                        C::FixedWidthMode => return "Ctrl+Alt+Shift+F",
                        C::NoWrap | C::WrapByWindow | C::ShowSpaceChar => return "",
                        C::ToggleWindow => return "MacCtrl+Tab",
                        _ => {}
                    }
                }
                match self { $(C::$name => $key),* }
            }
            pub fn from_id(id: &str) -> Option<C> { ALL.iter().copied().find(|c| c.id() == id) }

            /// 沒有開啟任何文件時仍可執行的命令：開檔、結束、外觀設定與不需要文件內容的對話框。
            /// 其餘命令由 `App::dispatch` 擋下並提示，避免操作不存在的文件。
            pub fn works_without_document(self) -> bool {
                matches!(
                    self,
                    C::New | C::Open | C::RecentFilesList | C::Exit
                        | C::AlwaysOnTop | C::FullScreen | C::PostIt | C::SetFont | C::ZoomIn | C::ZoomOut
                        | C::FixedWidthMode | C::NoWrap | C::WrapByWindow | C::WrapByColumn
                        | C::DisplayLineNumber | C::DisplayBookmark | C::Display80ColHint | C::ShowRuler
                        | C::ShowEndOfLine | C::ShowTabChar | C::ShowSpaceChar | C::ShowAllChar
                        | C::RightToLeft | C::MarkActiveLine | C::MarkBracePair | C::SpellCheck | C::TypewriterMode
                        | C::ToggleToolbar | C::ToggleMenubar | C::ToggleStatusbar | C::ToggleFileView | C::ToggleTheme
                        | C::ShowFindInFilesResults | C::CollapseAllResults
                        | C::Options | C::Highlighting | C::FileAssociation | C::SourceFormatter | C::PurgeHistories
                        | C::WindowList | C::KeyboardShortcuts | C::About
                )
            }
        }
    };
}

commands! {
    // ── 檔案 ──
    New => "menuNew", "新增檔案(&N)", "Ctrl+N";
    Open => "menuOpen", "開啟舊檔(&O)...", "Ctrl+O";
    Save => "menuSave", "儲存檔案(&S)", "Ctrl+S";
    SaveAs => "menuSaveAs", "另存新檔(&A)...", "";
    SaveACopy => "menuSaveACopy", "另存副本...", "";
    SaveAll => "menuSaveAll", "全部儲存(&V)", "Ctrl+Shift+S";
    Reload => "menuReload", "重新載入(&R)", "Ctrl+R";
    Close => "menuClose", "關閉檔案(&C)", "Ctrl+F4";
    CloseAll => "menuCloseAll", "全部關閉(&L)", "";
    CloseOthers => "menuCloseAllButThis", "關閉其他檔案", "";
    CloseLeft => "menuCloseAllToTheLeft", "關閉左側所有檔案", "";
    CloseRight => "menuCloseAllToTheRight", "關閉右側所有檔案", "";
    ToggleReadOnly => "menuToggleReadOnly", "設定/清除唯讀屬性", "";
    PageSetup => "menuPageSetup", "頁面設定(&U)...", "";
    PrintPreview => "menuPrintPreview", "預覽列印(&W)...", "";
    Print => "menuPrint", "列印(&P)...", "Ctrl+P";
    RecentFilesList => "menuRecentFilesList", "最近使用的檔案清單...", "F9";
    Exit => "menuExit", "結束(&X)", "";

    // ── 編輯 ──
    Undo => "menuUndo", "復原(&U)", "Ctrl+Z";
    Redo => "menuRedo", "重做(&R)", "Ctrl+Shift+Z";
    Cut => "menuCut", "剪下(&T)", "Ctrl+X";
    Copy => "menuCopy", "複製(&C)", "Ctrl+C";
    Paste => "menuPaste", "貼上(&P)", "Ctrl+V";
    Delete => "menuDelete", "刪除(&D)", "";
    CutLine => "menuCutLine", "剪下整行(&I)", "Ctrl+Shift+L";
    DeleteLine => "menuDeleteLine", "刪除整行(&L)", "Ctrl+L|Ctrl+Y";
    DuplicateLine => "menuDuplicateLine", "複製整行", "Ctrl+Shift+D";
    MoveLinesUp => "menuMoveLinesUp", "整行上移", "Alt+ArrowUp";
    MoveLinesDown => "menuMoveLinesDown", "整行下移", "Alt+ArrowDown";
    JoinLines => "menuJoinLines", "合併行", "Ctrl+J";
    DeleteEmptyLines => "menuDeleteEmptyLines", "刪除空行", "";
    DeleteEmptyLinesWithSpaces => "menuDeleteEmptyLinesWithSpaces", "刪除空行（含只有空白的行）", "";
    TrimTrailingSpaces => "menuTrimTrailingSpaces", "刪除行尾空白(&M)", "";
    TrimLeadingSpaces => "menuTrimLeadingSpaces", "刪除行首空白", "";
    SelectAll => "menuSelectAll", "全選(&A)", "Ctrl+A";
    StartEndSelecting => "menuStartEndSelction", "開始/結束選取", "";
    InsertTabChar => "menuInsertTabChar", "插入 Tab 字元(&B)", "Ctrl+Backtick";
    InsertDateTime => "menuInsertDateTime", "插入日期與時間(&E)", "F7";
    BookmarkCopy => "menuBookmarkCopy", "複製有書籤的行", "";
    BookmarkCut => "menuBookmarkCut", "剪下有書籤的行", "";
    BookmarkDel => "menuBookmarkDel", "刪除有書籤的行", "";
    BookmarkCopyUnmarked => "menuBookmarkCopyUnmarked", "複製沒有書籤的行", "";
    BookmarkCutUnmarked => "menuBookmarkCutUnmarked", "剪下沒有書籤的行", "";
    BookmarkDelUnmarked => "menuBookmarkDelUnmarked", "刪除沒有書籤的行", "";
    BookmarkReplace => "menuBookmarkReplace", "以剪貼簿取代有書籤的行", "";
    CopyAsHexString => "menuCopyAsHexString", "複製為十六進位字串(&H)", "";
    CopyAsHexStringWithSpace => "menuCopyAsHexStringWithSpace", "複製為十六進位字串（含空白）(&X)", "";
    CopyRevertHex => "menuCopyRevertHex", "複製由十六進位字串還原的內容(&R)", "";
    IncreaseIndent => "menuIncreaseIndent", "增加縮排(&I)", "";
    DecreaseIndent => "menuDecreaseIndent", "減少縮排(&D)", "";
    Comment => "menuComment", "註解(&C)", "Ctrl+M";
    Uncomment => "menuUncomment", "取消註解(&U)", "Ctrl+Shift+M";
    WordWrapToNewLine => "menuWordWrapToNewLine", "自動換行轉為換行字元", "";
    NewLineToWordWrap => "menuNewLineToWordWrap", "換行字元轉為自動換行", "";
    ToUpperCase => "menuToUpperCase", "轉為大寫(&P)", "Ctrl+U";
    ToLowerCase => "menuToLowerCase", "轉為小寫(&O)", "Ctrl+Shift+U";
    InvertCase => "menuInvertCase", "反轉大小寫(&T)", "Ctrl+Alt+U";
    Capitalize => "menuCapitalize", "首字大寫", "";
    ToHalfWidth => "menuToHalfWidth", "轉為半形(&A)", "";
    ToHalfWidthByOptions => "menuToHalfWidthByOptions", "依選項轉為半形...", "";
    ToFullWidth => "menuToFullWidth", "轉為全形(&F)", "";
    ToFullWidthByOptions => "menuToFullWidthByOptions", "依選項轉為全形...", "";
    TabToSpace => "menuTabToSpace", "Tab 字元轉為空白", "";
    SpaceToTab => "menuSpaceToTab", "空白轉為 Tab 字元", "";
    InsertNumbers => "menuInsertNumbers", "插入遞增數字...", "Ctrl+Shift+N";
    ColumnAlignLeft => "menuColumnAlignLeft", "列塊靠左對齊", "";
    ColumnAlignRight => "menuColumnAlignRight", "列塊靠右對齊", "";
    SortAscending => "menuSortAscending", "排序（遞增）(&A)", "";
    SortDescending => "menuSortDescending", "排序（遞減）(&D)", "";
    SortAscendingCase => "menuSortAscendingCase", "排序（遞增，區分大小寫）(&S)", "";
    SortDescendingCase => "menuSortDescendingCase", "排序（遞減，區分大小寫）(&E)", "";
    SortByOptions => "menuSortByOptions", "依目前選項排序(&C)", "";
    SortOptions => "menuSortOptions", "排序選項(&O)...", "";

    // ── 搜尋 ──
    Find => "menuFind", "尋找(&F)...", "Ctrl+F";
    FindNext => "menuFindNext", "找下一個(&N)", "F3";
    FindPrevious => "menuFindPrevious", "找上一個(&P)", "Ctrl+F3|Shift+F3";
    QuickFind => "menuShowQuickSearchBar", "快速尋找(&Q)", "F8";
    Replace => "menuReplace", "取代(&R)...", "Ctrl+H";
    FindInFiles => "menuFindInFiles", "在檔案中尋找/取代(&D)...", "Ctrl+Shift+F";
    ShowFindInFilesResults => "menuShowFindInFilesResults", "顯示/隱藏檔案搜尋結果(&S)", "Ctrl+Shift+R";
    CollapseAllResults => "menuCollapseAllResults", "摺疊所有結果(&A)", "Alt+Minus";
    GoToLine => "menuGoToLine", "移至行(&G)...", "Ctrl+G";
    GoToPosition => "menuGoToPosition", "移至位置(&O)...", "Ctrl+Shift+G";
    GoBack => "menuGoBack", "上一個游標位置", "Alt+OpenBracket";
    GoForward => "menuGoForward", "下一個游標位置", "Alt+CloseBracket";
    LeftBrace => "menuLeftBrace", "移至左括號(&E)", "Ctrl+OpenBracket";
    RightBrace => "menuRightBrace", "移至右括號(&I)", "Ctrl+CloseBracket";
    ToggleBookmark => "menuToggleBookmark", "設定/移除書籤", "Ctrl+F2";
    NextBookmark => "menuGotoNextBookmark", "移至下一個書籤", "F2";
    PreviousBookmark => "menuGotoPreviousBookmark", "移至上一個書籤", "Shift+F2";
    ClearAllBookmarks => "menuClearAllBookmarks", "清除所有書籤", "Ctrl+Shift+F2";

    // ── 檢視 ──
    AlwaysOnTop => "menuAlwaysOnTop", "永遠在最上層", "Ctrl+F11";
    FullScreen => "menuFullScreen", "全螢幕模式", "F11";
    PostIt => "menuPostIt", "便利貼模式", "F12";
    SetFont => "menuSetFont", "設定字型...", "";
    ZoomIn => "menuZoomIn", "放大字型", "Ctrl+Plus|Ctrl+Equals";
    ZoomOut => "menuZoomOut", "縮小字型", "Ctrl+Minus";
    FixedWidthMode => "menuFixedWidthMode", "等寬模式(&F)", "Ctrl+Alt+F";
    NoWrap => "menuNoWrap", "不自動換行(&N)", "Ctrl+Q";
    WrapByWindow => "menuWrapByWindow", "依視窗寬度換行(&W)", "Ctrl+W";
    WrapByColumn => "menuWrapByColumn", "依欄數換行(&C)", "Ctrl+E";
    DisplayLineNumber => "menuDisplayLineNumber", "顯示行號(&L)", "";
    DisplayBookmark => "menuDisplayBookmark", "顯示書籤(&B)", "";
    Display80ColHint => "menuDisplay80ColHint", "顯示欄位提示線", "";
    ShowRuler => "menuShowRuler", "顯示尺標(&R)", "";
    ShowEndOfLine => "menuShowEndOfLine", "顯示行尾字元", "Ctrl+Alt+L";
    ShowTabChar => "menuShowTabChar", "顯示 Tab 字元", "Ctrl+Alt+T";
    ShowSpaceChar => "menuShowSpaceChar", "顯示空白字元", "Ctrl+Alt+S";
    ShowAllChar => "menuShowAllChar", "顯示所有字元", "Ctrl+Alt+A";
    RightToLeft => "menuViewRightToLeft", "由右至左", "Ctrl+Alt+R";
    MarkActiveLine => "menuMarkActiveLine", "標示目前行", "";
    MarkBracePair => "menuMarkBracePair", "標示對應括號", "";
    SpellCheck => "menuSpellChecker", "拼字檢查", "Ctrl+K";
    TypewriterMode => "menuTypewriterMode", "打字機模式", "";
    ToggleToolbar => "menuToolBarsToggleAll", "顯示/隱藏工具列", "";
    ToggleMenubar => "menuMenuBarToggle", "顯示/隱藏選單列", "Ctrl+F10";
    ToggleStatusbar => "menuStatusBarToggle", "顯示/隱藏狀態列", "";
    ToggleFileView => "menuFileView", "文件視圖(&F)", "";
    ToggleTheme => "menuToggleTheme", "切換深色/淺色主題", "";
    TextMode => "menuTextMode", "文字模式(&T)", "Alt+1";
    ColumnMode => "menuColumnMode", "列塊模式(&C)", "Alt+2";
    HexMode => "menuHexMode", "十六進位模式(&H)", "Alt+3";
    PreviewMarkdown => "menuPreviewMarkdown", "以 Markdown 預覽", "";
    PreviewHtml => "menuPreviewHtml", "以 HTML 預覽", "";
    PreviewSvg => "menuPreviewSvg", "以圖形預覽（SVG）", "";
    FoldToggle => "menuFoldToggle", "折疊/展開(&F)", "Ctrl+Shift+OpenBracket";
    FoldAll => "menuFoldAll", "全部折疊(&A)", "";
    UnfoldAll => "menuUnfoldAll", "全部展開(&L)", "";
    ToggleCodeFolding => "menuCodeFolding", "啟用代碼折疊", "";

    // ── 巨集 ──
    RunTempMacro => "menuRunTempMacro", "執行暫存巨集...", "";
    LoadMacro => "menuRunMacroFile", "載入並執行已儲存的巨集...", "";
    StartRecMacro => "menuStartRecMacro", "開始錄製", "";
    StopRecMacro => "menuStopRecMacro", "停止錄製", "";
    PlayRecMacro => "menuPlayRecMacro", "播放", "Ctrl+Shift+P";
    SaveRecMacro => "menuSaveRecMacro", "儲存目前錄製的巨集...", "";
    EditMacroFile => "menuEditMacroFile", "編輯已儲存的巨集...", "";

    // ── 工具 ──
    Options => "menuOptions", "選項(&O)...", "";
    RunScript => "menuRunScript", "運行腳本(&R)", "F5";
    Highlighting => "menuHighlighting", "語法突顯設定(&S)...", "";
    FileAssociation => "menuFileAssociation", "檔案類型關聯(&F)...", "";
    SourceFormatter => "menuSourceFormatter", "原始碼格式化設定(&S)...", "";
    PurgeHistories => "menuPurgeHistories", "清除歷史紀錄(&P)...", "";
    ToggleBOM => "menuToggleBOM", "加入/移除 BOM", "";
    ConvertToDOS => "menuConvertToDOS", "轉為 CRLF/0D0A (&DOS)", "";
    ConvertToMac => "menuConvertToMAC", "轉為 CR/0D（舊版 &Mac）", "";
    ConvertToUNIX => "menuConvertToUNIX", "轉為 LF/0A (&Unix/macOS)", "";
    InsertDOS => "menuInsertDOS", "插入 CRLF/0D0A (&DOS)", "";
    InsertMac => "menuInsertMAC", "插入 CR/0D（舊版 &Mac）", "";
    InsertUNIX => "menuInsertUNIX", "插入 LF/0A (&Unix/macOS)", "";
    ConvertEncoding => "menuConvertEncoding", "轉換檔案編碼(&E)...", "";
    ChineseSafeMode => "menuConvertChineseSafe", "安全模式", "";
    Simp2TradChinese => "menuSimp2TradChinese", "檔案：簡體中文轉繁體中文(&T)", "";
    Trad2SimpChinese => "menuTrad2SimpChinese", "檔案：繁體中文轉簡體中文(&S)", "";
    Kanji2TradChinese => "menuKanji2TradChinese", "檔案：日文漢字轉繁體中文(&A)", "";
    Kanji2SimpChinese => "menuKanji2SimpChinese", "檔案：日文漢字轉簡體中文(&M)", "";
    Chinese2Kanji => "menuChinese2Kanji", "檔案：中文轉日文漢字(&J)", "";
    Simp2TradClipboard => "menuSimp2TradClipboard", "剪貼簿：簡體中文轉繁體中文(&R)", "";
    Trad2SimpClipboard => "menuTrad2SimpClipboard", "剪貼簿：繁體中文轉簡體中文(&I)", "";
    Kanji2TradClipboard => "menuKanji2TradClipboard", "剪貼簿：日文漢字轉繁體中文(&D)", "";
    Kanji2SimpClipboard => "menuKanji2SimpClipboard", "剪貼簿：日文漢字轉簡體中文(&P)", "";
    Chinese2KanjiClipboard => "menuChinese2KanjiClipboard", "剪貼簿：中文轉日文漢字(&K)", "";
    Markdown2Html => "menuMarkdown2Html", "Markdown 轉 HTML(&M)", "";
    Html2PlainText => "menuHtml2PlainText", "HTML 轉純文字(&H)", "";
    AstyleFormat => "menuAstyleFormat", "AStyle 格式化（C++/C#/Java/ObjC）(&A)", "Ctrl+Shift+K";
    XmlFormat => "menuXMLFormat", "XML 格式化(&X)", "";
    JsonFormat => "menuJSONFormat", "JSON 格式化(&J)", "";
    DprintFormat => "menuDprintFormat", "dprint 格式化（C/C++、JS/TS、JSON、MD、TOML、Python）(&D)", "";
    WordCount => "menuWordCount", "字數統計(&W)...", "";

    // ── 視窗 ──
    ToggleWindow => "menuToggleWindow", "切換視窗(&T)", "Ctrl+Tab";
    PreviousWindow => "menuPreviousWindow", "上一個視窗(&P)", "F10";
    NextWindow => "menuNextWindow", "下一個視窗(&N)", "F6";
    WindowList => "menuWindowList", "視窗清單(&W)...", "";
    // 以下僅供快捷鍵使用，不放進選單（原版無此命令；第 9 個為最後一個分頁，同瀏覽器慣例）
    SwitchTab1 => "menuSwitchTab1", "切換至第 1 個分頁", "Ctrl+1";
    SwitchTab2 => "menuSwitchTab2", "切換至第 2 個分頁", "Ctrl+2";
    SwitchTab3 => "menuSwitchTab3", "切換至第 3 個分頁", "Ctrl+3";
    SwitchTab4 => "menuSwitchTab4", "切換至第 4 個分頁", "Ctrl+4";
    SwitchTab5 => "menuSwitchTab5", "切換至第 5 個分頁", "Ctrl+5";
    SwitchTab6 => "menuSwitchTab6", "切換至第 6 個分頁", "Ctrl+6";
    SwitchTab7 => "menuSwitchTab7", "切換至第 7 個分頁", "Ctrl+7";
    SwitchTab8 => "menuSwitchTab8", "切換至第 8 個分頁", "Ctrl+8";
    SwitchLastTab => "menuSwitchLastTab", "切換至最後一個分頁", "Ctrl+9";

    // ── 說明 ──
    KeyboardShortcuts => "menuKeyboardShortcuts", "鍵盤快捷鍵(&K)...", "F1";
    About => "menuAbout", "關於 MadEdit(&A)...", "";
}

/// Drops a trailing "..." and then a trailing one-letter mnemonic such as "(F)" or "(&F)"
/// from a menu label, for lists where neither means anything ("尋找(F)..." becomes "尋找").
/// Longer parentheses such as "(LF)" are part of the name and stay.
pub fn plain_label(label: &str) -> String {
    let s = label.trim_end_matches("...").trim_end_matches('…').trim_end();
    let mnemonic = s.strip_suffix(')').and_then(|r| {
        let r = r.strip_suffix(|ch: char| ch.is_ascii_alphanumeric())?;
        r.strip_suffix("(&").or_else(|| r.strip_suffix('('))
    });
    mnemonic.unwrap_or(s).trim_end().to_string()
}

/// 解析單一快捷鍵字串（如 "Ctrl+Shift+K"、"Alt+ArrowUp"、"F3"）
pub fn parse_shortcut(s: &str) -> Option<KeyboardShortcut> {
    let mut mods = Modifiers::NONE;
    let mut key = None;
    for part in s.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "cmd" | "command" => mods |= Modifiers::COMMAND,
            "macctrl" => mods |= Modifiers::CTRL,
            "shift" => mods |= Modifiers::SHIFT,
            "alt" | "option" => mods |= Modifiers::ALT,
            _ => key = Key::from_name(part),
        }
    }
    // "Ctrl++" 這類寫法：最後一段為空代表加號鍵
    if key.is_none() && s.ends_with("++") {
        key = Some(Key::Plus);
    }
    key.map(|k| KeyboardShortcut::new(mods, k))
}

/// 快捷鍵轉回設定檔使用的字串
pub fn shortcut_to_string(sc: &KeyboardShortcut) -> String {
    let mut parts = Vec::new();
    if sc.modifiers.command || sc.modifiers.mac_cmd {
        parts.push("Ctrl".to_string());
    } else if sc.modifiers.ctrl {
        parts.push(if cfg!(target_os = "macos") { "MacCtrl".into() } else { "Ctrl".into() });
    }
    if sc.modifiers.alt {
        parts.push("Alt".into());
    }
    if sc.modifiers.shift {
        parts.push("Shift".into());
    }
    parts.push(sc.logical_key.name().to_string());
    parts.join("+")
}

/// 按下的修飾鍵是否與快捷鍵完全相符（不像 egui 的 matches_logically 會忽略多按的 Shift）
pub fn modifiers_match(pressed: Modifiers, pattern: Modifiers) -> bool {
    pressed.alt == pattern.alt && pressed.shift == pattern.shift && pressed.command == pattern.command && {
        // macOS 上 Ctrl 與 ⌘ 是不同的鍵；其他平台 command 即 ctrl
        if cfg!(target_os = "macos") { pressed.ctrl == pattern.ctrl } else { true }
    }
}

/// 快捷鍵擷取時是否接受此組合
///
/// 沒有 Ctrl／⌘／Alt 的按鍵（含只加 Shift）只接受 F1～F35；否則 Enter、Backspace、方向鍵、
/// 一般字元等編輯用按鍵會被命令攔截，編輯器再也收不到。
pub fn is_bindable(sc: &KeyboardShortcut) -> bool {
    let m = sc.modifiers;
    if m.command || m.mac_cmd || m.ctrl || m.alt {
        return true;
    }
    let name = sc.logical_key.name();
    name.len() > 1 && name.starts_with('F') && name[1..].bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_label_drops_only_one_letter_mnemonics() {
        assert_eq!(plain_label("尋找(F)..."), "尋找");
        assert_eq!(plain_label("About MadEdit (A)..."), "About MadEdit");
        assert_eq!(plain_label("鍵盤快捷鍵(&K)…"), "鍵盤快捷鍵");
        assert_eq!(plain_label("Help (H)"), "Help");
        assert_eq!(plain_label("Convert to UNIX (LF)"), "Convert to UNIX (LF)");
        assert_eq!(plain_label("Close (Esc)"), "Close (Esc)");
        assert_eq!(plain_label("Select All"), "Select All");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_macos_default_bindings_are_unique_and_keep_system_keys_free() {
        let mut seen = std::collections::HashMap::new();
        for &c in ALL {
            for key in c.default_keys().split('|').filter(|s| !s.is_empty()) {
                assert!(seen.insert(key, c).is_none(), "重複快捷鍵：{key}");
            }
        }
        assert_eq!(seen.get("Ctrl+W"), Some(&C::Close));
        assert_eq!(seen.get("Ctrl+Q"), Some(&C::Exit));
        assert_eq!(seen.get("Ctrl+Shift+S"), Some(&C::SaveAs));
        assert!(!seen.contains_key("Ctrl+H"), "⌘H 留給系統隱藏視窗");
        assert!(!seen.contains_key("Ctrl+M"), "⌘M 留給系統最小化視窗");
        assert!(!seen.contains_key("Ctrl+Tab"), "⌘Tab 留給系統切換程式");
    }

    #[test]
    fn test_is_bindable_requires_modifier_except_function_keys() {
        for s in ["F3", "Shift+F3", "Ctrl+Enter", "Alt+Backspace", "Ctrl+A", "MacCtrl+A"] {
            assert!(is_bindable(&parse_shortcut(s).unwrap()), "{s} 應可綁定");
        }
        for s in ["Enter", "Backspace", "Tab", "Up", "Space", "A", "Shift+A", "Shift+Up", "Delete"] {
            assert!(!is_bindable(&parse_shortcut(s).unwrap()), "{s} 不應可綁定");
        }
    }

    #[test]
    fn test_run_script_and_previous_window_shortcuts() {
        // F5 讓給「運行腳本」，原「上一個視窗」改用 F10
        assert_eq!(C::RunScript.default_keys(), "F5");
        assert_eq!(C::PreviousWindow.default_keys(), "F10");
        assert_eq!(C::FoldToggle.default_keys(), "Ctrl+Shift+OpenBracket");
    }

    #[test]
    fn test_ids_unique_and_shortcuts_parse() {
        let mut ids: Vec<&str> = ALL.iter().map(|c| c.id()).collect();
        ids.sort();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "命令識別碼重複");
        for c in ALL {
            for k in c.default_keys().split('|').filter(|k| !k.is_empty()) {
                assert!(parse_shortcut(k).is_some(), "{} 的快捷鍵 {k} 無法解析", c.id());
            }
        }
        let sc = parse_shortcut("Ctrl+Shift+K").unwrap();
        assert_eq!(shortcut_to_string(&sc), "Ctrl+Shift+K");
        assert_eq!(C::from_id("menuFind"), Some(C::Find));
        assert_eq!(crate::app::i18n::strip_mnemonic(C::Find.label()), "尋找(F)...");
    }
}
