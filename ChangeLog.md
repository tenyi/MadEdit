# Changelog

This file is the source of user-visible changes. GitHub Release notes are taken directly from the matching `## [version]` section (see the release instructions in README).

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and version numbers follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- The text editor renders syntax-defined bold and italic styles, including Markdown headings and emphasis, without changing caret, selection or wrapping metrics.

### Fixed
- Syntax highlighting stays aligned after inserting or deleting lines, including undo/redo and edits during time-sliced parsing.
- Debug builds detect edits outside their declared line range using allocation-free fingerprints, protecting undo and incremental caches without copying untouched lines.
- Whole-document edits reuse the undo snapshot's changed strings instead of cloning them twice. Large canonical Unicode edits release original bytes before snapshotting; no-op commands still preserve the original bytes, and malformed, legacy-encoded or mixed-newline input retains its lossless recovery data.
- Character-position queries reuse a versioned line-start offset across local edits and nearby cursor moves instead of rescanning the document prefix on every keystroke. Undo/redo and full resets invalidate the offset when necessary.

### 繁體中文

#### 新增
- 文字編輯器支援語法定義的粗體與斜體，包括 Markdown 標題與強調文字，且不改變游標、選取範圍及折行的字寬計算。

#### 修正
- 插入或刪除行後，語法高亮仍與文字對齊；復原／重做及分幀解析期間的編輯也一併處理。
- 除錯版本以不配置記憶體的雜湊檢查偵測超出宣告行範圍的編輯，保護復原紀錄與增量快取，無須複製未修改的行。
- 全文編輯直接沿用復原快照中已變更的字串，避免重複複製。大型標準 Unicode 編輯會在建立快照前釋放原始位元組；無變更命令仍保留原始內容，解碼錯誤、傳統編碼或混合換行的資料也保留無損復原所需的原始位元組。
- 字元位置查詢在局部編輯及鄰近游標移動時沿用含版本的行首偏移錨點，不再每次按鍵都重掃文件前綴；復原／重做及全文重設會在必要時使錨點失效。

## [0.6.7] - 2026-10-30

### Fixed
- Selecting text with the mouse now starts at the position where the button was pressed. Dragging back no longer skips the character under the pointer, and dragging over the line-number gutter selects whole lines from the pressed line.

### 繁體中文

#### 修正
- 以滑鼠拖曳選取文字時，選取範圍改為從按下的位置開始，往回拖曳不再漏掉游標下的字元；在行號欄拖曳選取整行也改為從按下的那一行開始。

## [0.6.6] - 2026-10-29

### Added
- Automatically check GitHub Releases daily, show a non-blocking update notice with a download link, and offer manual checks and update preferences in Help.
- New Tools > Convert/Format > "dprint Format" command formats the whole document with [dprint](https://dprint.dev/): the bundled plugins cover C/C++/Objective-C (clang-format), JavaScript/TypeScript, JSON/JSONC, CSS/SCSS/Sass/Less, HTML/XML/SVG/Vue/Svelte and template languages, YAML, Markdown, TOML, GraphQL, SQL, Dockerfile, Shell (sh/bash/zsh), Python, Go, PHP, Lua and C#/MSBuild. Unsupported extensions are refused with a message instead of being passed through unchanged.
- Releases now come in two flavours per platform: the usual clean package and a `-dprint` package (for example `MadEdit-<version>-macOS-arm64-dprint.dmg`) that additionally bundles a pinned dprint CLI and plugins, so formatting works offline. The clean package uses the same command through a `dprint` found on `PATH`. Locally, `MADEDIT_WITH_FORMATTER=1 make dist-...` builds the bundle; `make formatters` downloads the pinned dprint and plugins into `target/formatters` (versions and SHA-256 come from `assets/formatters/manifest.json`).
- The macOS dprint helper is signed for Hardened Runtime with a minimal `com.apple.security.cs.allow-unsigned-executable-memory` entitlement (required by its Wasm runtime); the app binary does not carry this entitlement, and the helper's minimum macOS version is included in the app's `LSMinimumSystemVersion`.

### Changed
- The Markdown preview renders LaTeX math: inline `$…$` sits on the text baseline and `$$…$$` is centred on its own line. Chinese and other non-ASCII text in a top-level `\text{…}` is shown with the preview font; a formula that cannot be rendered is shown as its TeX source.

- Building from source needs Rust 1.99 or newer (required by latex-rust, the math renderer).

### Fixed
- Long submenus such as Recent Files no longer flicker between vertical positions near the bottom of the window.
- In the Markdown preview, a table inside a block quote (`> | a | b |`) ran together as a single paragraph of cell text. It now renders as a table inside the quote.

### 繁體中文

#### 新增
- 每日自動檢查 GitHub Releases，以不干擾操作的通知顯示更新與下載連結，並在「說明」選單提供手動檢查和更新偏好設定。
- 新增「工具 > 轉換／格式化 > dprint 格式化」命令，使用 [dprint](https://dprint.dev/) 格式化整份文件。隨附外掛支援 C/C++/Objective-C（clang-format）、JavaScript/TypeScript、JSON/JSONC、CSS/SCSS/Sass/Less、HTML/XML/SVG/Vue/Svelte 與範本語言、YAML、Markdown、TOML、GraphQL、SQL、Dockerfile、Shell（sh/bash/zsh）、Python、Go、PHP、Lua 和 C#/MSBuild。不支援的副檔名會顯示提示並拒絕處理。
- 各平台發布檔提供兩種版本：一般精簡套件，以及額外包含固定版本 dprint 命令列工具與外掛的 `-dprint` 套件（例如 `MadEdit-<version>-macOS-arm64-dprint.dmg`），讓格式化能離線運作。精簡套件透過 `PATH` 中的 `dprint` 使用相同命令。本機可用 `MADEDIT_WITH_FORMATTER=1 make dist-...` 建置完整套件；`make formatters` 會下載固定版本的 dprint 與外掛至 `target/formatters`（版本與 SHA-256 取自 `assets/formatters/manifest.json`）。
- macOS dprint 輔助程式使用最小權限的 `com.apple.security.cs.allow-unsigned-executable-memory` 權益簽署並啟用 Hardened Runtime（Wasm 執行環境需要此權益）；主程式不帶此權益，輔助程式要求的最低 macOS 版本也會列入 App 的 `LSMinimumSystemVersion`。

#### 變更
- Markdown 預覽支援 LaTeX 數學公式：行內 `$…$` 與文字基線對齊，`$$…$$` 獨立置中。最外層 `\text{…}` 中的中文及其他非 ASCII 文字使用預覽字型顯示；無法繪製的公式則顯示 TeX 原始碼。
- 從原始碼建置需要 Rust 1.99 或更新版本（數學公式繪製套件 latex-rust 的要求）。

#### 修正
- 最近檔案等較長的子選單在視窗底部附近不再上下閃動。
- Markdown 預覽中的引用區塊表格（`> | a | b |`）不再合併成一段儲存格文字，現在會在引用區塊中正確顯示為表格。

## [0.6.5] - 2026-10-28

### Changed
- The Markdown preview body text uses 16pt, so long documents are easier to read.

### Fixed
- With a proportional font and fixed-width mode off, long lines drifted from the caret and selection and showed a gap before the next differently drawn character (such as the closing quote after a base64 string). Each character is now drawn at its own laid-out position.

## [0.6.4] - 2026-10-28

### Added
- Picking a proportional editor font while fixed-width mode is on asks whether to turn the mode off, since wide letters such as m and W overlap their neighbours in it.
- The UI theme can follow the system or be set to Light or Dark. The View menu, the toolbar and the Options dialog all apply it, and dialogs, banners and the Markdown preview follow it.
- Help > Keyboard Shortcuts (F1) lists every current shortcut, customized ones included, grouped by menu in two columns. The list can be filtered by command name or key (words such as ctrl or cmd also find keys shown as symbols), and a button opens the Keys page of Options to customize them.
- The About dialog shows the app icon, version, platform, technologies used, settings file location and a project link, with a button that copies the version information for bug reports. Copied paths show the home folder as ~, so the user name is not revealed.

### Changed
- Fixed-width mode is off by default. Existing settings files keep their choice; Column mode still uses the fixed grid.
- View preferences changed from the View menu, the toolbar, shortcuts or Ctrl+wheel zoom (wrapping, line numbers, control characters, font, size, tab width, line spacing, fixed-width mode and so on) are saved at once instead of only on a normal exit.
- Refreshed look: rounded buttons, tabs and dialogs, soft shadows, and card-style find bar and file-change banner.
- When the window is wide enough, font zoom, the theme switch and quick search move to the right of the menu bar, leaving the toolbar a single row of icons (they stay when the toolbar is hidden). With the menu bar hidden or a narrow window they remain in the toolbar.
- Consistent menus: top-level menus, submenus and context menus share a minimum width with their item text in one column, and long menus scroll instead of running past the bottom of the window.
- The View menu is hierarchical: the top level keeps the edit modes, encoding, syntax, preview and window items, while Font, Layout (wrapping, tab width, line spacing, text direction) and Show (line numbers, ruler, control characters, highlights, spell check) each have a submenu.
- The theme switch at the right of the menu bar is one icon button instead of three text buttons: a sun in the light theme and a moon in the dark one, and each click switches to the other, like the Toggle Dark/Light Theme command. The chosen theme is saved and used at the next launch; without a settings file, MadEdit follows the system theme.
- The Options dialog is a modern settings page: a sidebar of pages, settings grouped into cards by topic, names on the left and values aligned on the right. Switches replace checkboxes (clicking the text flips them too), small choices such as the theme and line endings are segmented buttons, and the Keys page lists each command's shortcuts beside it with a card on the right for editing them. Page Setup uses the same layout. Buttons are ordered Apply, Cancel, OK, with the main action rightmost.
- Every other dialog (Font, Recent Files, Insert Numbers, Sort Options, Find/Replace in Files, Temporary Macro, Syntax Highlighting, Source Formatter, Clear History, Convert Encoding, Word Count, Window List, About, Keyboard Shortcuts and all confirmations, including the new create-file prompt) uses the same card layout: switches, segmented buttons, and a right-aligned button row with the main action, in the accent color, rightmost. Buttons that discard data have red text. Dialogs get narrower in a small main window, content that does not fit scrolls so the buttons stay on screen, and dialogs reopen scrolled to the top. Setting cards have no dividers between rows; hovering a switch row highlights the whole row, since clicking anywhere on it flips the switch.
- Theme changes in Syntax Highlighting are saved at once.
- Screen readers announce toolbar icons by their command name, zoom buttons as zoom in / out instead of "A+" or "−", and × buttons by what they do instead of "×": the close buttons of tabs and of the Window List as "Close" followed by the file name, and the remove buttons in Options as "Remove" followed by the shortcut or word.
- In Recent Files, Tab moves on to the list and Enter on an item opens that file.
- The Options dialog has a fixed size, so switching pages no longer resizes it. In a small main window the Options and Keyboard Shortcuts dialogs shrink so their buttons stay visible, and the shortcut list uses one column when narrow.
- While Options is open, F1 switches it to the Keys page, and commands that open other dialogs (such as F9) no longer replace it; the status bar asks to close Options first, so unapplied settings are kept.
- OK and Apply in Options write back only the settings changed in the dialog, so other changes made while it was open (recent files, View menu toggles, font zoom, the personal dictionary and so on) are no longer reverted.
- Switching page or command after pressing Add Shortcut on the Keys page cancels the capture instead of silently assigning the next key to the previous command.
- Syntax highlighting for very long lines now tokenizes only the visible part of a line, uses the Oniguruma regex backend, and keeps each line's layout across frames. On a 1.99 MB sample whose longest line is 158 KB: keystroke 65 → 6 ms, End key 61 → 22 ms, PageDown per frame 119 → 1 ms, open plus first render 517 → 131 ms. The visible part at the end of a long line is now coloured too (only the first 16 KB used to be), and lines with full-width characters no longer crash while scrolling. Generate the sample with `python3 scripts/make_example_js.py`; see `docs/adr/0012-long-line-syntax-highlighting.md`.

### Fixed
- With fixed-width mode off, a newly opened file could still lay out characters on the fixed grid (letters overlapping in proportional fonts) until the mode was toggled. Characters not yet measured now get their real width at once, and all widths are measured again after the editor font changes.
- A message opened by a key press (such as an error after pressing Enter on a file in Recent Files that cannot be read) is no longer dismissed by that same key before it appears.
- The find bar background was transparent in the dark theme and showed as white.
- The drop-down arrow ▾ and close buttons showed as empty boxes.
- 한국어 in the UI language menu showed as empty boxes on macOS (PingFang has no Hangul); a Korean font is now added automatically.
- In Markdown preview, the stripes of tables with short columns ran past the table frame.
- In the dark Markdown preview, Mermaid edge labels sat on gray boxes instead of the black background.
- In the dark theme, black toolbar icons such as Find (binoculars), Replace, Wrap, Indent and Preview were nearly invisible. The dark theme now uses automatically lightened icons: outlines and solid black areas (such as the binoculars' body) are lightened, while details drawn on a light fill keep their color; the light theme keeps the original art. Thin lines are no longer lost when icons are shrunk on non-high-DPI displays.
- The 🔒 on read-only tabs and the copy button of Markdown code blocks showed as empty boxes; emoji fonts are now bundled.
- In the light theme, the hex view selection and warning and error text had too little contrast.

### 繁體中文

#### 新增
- 在固定寬度模式開啟時選用比例字型，會詢問是否要關閉該模式，因為 m、W 等寬字元在固定格線上會互相重疊。
- 介面主題可跟隨系統，或設定為淺色或深色。檢視選單、工具列與選項對話框都能套用主題，視窗對話框、提示橫幅與 Markdown 預覽也會跟著改變。
- 說明 > 鍵盤快捷鍵（F1）列出目前所有快捷鍵（含自訂的），依選單分成兩欄。可用命令名稱或按鍵篩選（輸入 ctrl、cmd 等字樣也能找到以符號顯示的按鍵），並可直接開啟選項的「按鍵」頁來自訂。
- 關於對話框會顯示程式圖示、版本、平台、使用到的技術、設定檔位置與專案連結，並提供按鈕把版本資訊複製到剪貼簿，方便回報問題。複製的路徑以 ~ 表示家目錄，不會露出使用者名稱。

#### 變更
- 固定寬度模式預設關閉，既有設定檔會保留原本的選擇；列塊模式仍使用固定格線。
- 從檢視選單、工具列、快捷鍵或 Ctrl+滾輪縮放變更的檢視偏好（自動換行、行號、控制字元、字型、字級、Tab 寬度、行距、固定寬度模式等）會立即保存，不必等到正常結束程式。
- 介面煥然一新：按鈕、分頁與對話框改為圓角，陰影柔和，尋找列與檔案變更提示改為卡片式樣。
- 視窗夠寬時，字型縮放、主題切換與快速搜尋移到選單列右側，工具列只留一排圖示（隱藏工具列時這些控制項仍然可用）。選單列隱藏或視窗較窄時，它們仍留在工具列。
- 選單樣式一致：最上層選單、子選單與右鍵選單共用最小寬度，項目文字排成一欄，選單過長時會捲動，不會超出視窗底部。
- 檢視選單改為階層式：最上層保留編輯模式、編碼、語法、預覽與視窗項目，字型、版面（自動換行、Tab 寬度、行距、文字方向）與顯示（行號、尺標、控制字元、高亮、拼字檢查）各自成為子選單。
- 選單列右側的主題切換改為單一圖示按鈕，不再是三個文字按鈕：淺色主題顯示太陽，深色主題顯示月亮，每次點擊在兩者之間切換，行為與「切換深色／淺色主題」命令相同。選用的主題會保存，下次啟動時沿用；沒有設定檔時，MadEdit 跟隨系統主題。
- 選項對話框改為現代設定頁面：左側是頁面側欄，設定依主題分組為卡片，左側是名稱、右側對齊設定值。勾選框改成開關（點文字也能切換），主題、換行符號等小選擇改用分段按鈕；「按鍵」頁在每個命令旁列出快捷鍵，右側卡片用來編輯。版面設定頁使用相同版面。按鈕順序為套用、取消、確定，主要動作在最右。
- 其他所有對話框（字型、最近檔案、插入數字、排序選項、多檔搜尋／取代、暫存巨集、語法突顯、格式化程式、清除記錄、轉換編碼、字數統計、視窗清單、關於、鍵盤快捷鍵，以及全數確認對話框，含新增的建立檔案詢問）都採相同卡片版面：開關、分段按鈕，底部靠右的按鈕列，主要動作以強調色放在最右；會丟失資料的按鈕改用紅色文字。主視窗較小時對話框會變窄，內容放不下時可捲動，讓按鈕留在畫面上；對話框重新開啟時會從最上方開始。設定卡片的各列之間沒有分隔線；滑鼠停在開關列上會整列反白，點列上任何位置都能切換開關。
- 語法突顯對話框中的主題變更會立即保存。
- 螢幕閱讀器會以命令名稱朗讀工具列圖示，縮放按鈕讀作「放大」／「縮小」而不是「A+」或「−」，× 按鈕則讀出其功能而不是「×」：分頁與視窗清單的關閉按鈕讀作「關閉」加檔名，選項中的移除按鈕讀作「移除」加快捷鍵或字詞。
- 在「最近檔案」中，Tab 可移到清單，Enter 可開啟所選檔案。
- 選項對話框固定大小，切換頁面不再改變視窗大小。主視窗較小時，選項與鍵盤快捷鍵對話框會縮小讓按鈕保持可見，快捷鍵清單在空間不足時改為單欄。
- 選項開啟時，F1 會切換到「按鍵」頁，其他會開啟其他對話框的命令（例如 F9）不再取代它；狀態列會提示先關閉選項，因此尚未套用的設定會保留。
- 選項中的確定與套用只會寫回在對話框內修改過的設定，因此開啟選項期間的其他變更（最近檔案、檢視選單切換、字型縮放、個人詞典等）不再被還原。
- 在「按鍵」頁按下「新增快捷鍵」後若切換頁面或命令，會取消擷取，而不是把下一組按鍵悄悄指派給上一個命令。
- 超長行的語法突顯現在只分析行內可見的部分，改用 Oniguruma 正規表示式後端，並保留每一行跨影格的版面配置。在最長行 158 KB、總長 1.99 MB 的測試樣本上：按鍵 65 → 6 毫秒、End 鍵 61 → 22 毫秒、每影格 PageDown 119 → 1 毫秒、開檔到首次繪製 517 → 131 毫秒。超長行尾端的可見內容也會上色（先前只有前 16 KB），含全形字元的行捲動時不再當掉。可用 `python3 scripts/make_example_js.py` 產生測試樣本，細節見 `docs/adr/0012-long-line-syntax-highlighting.md`。

#### 修正
- 關閉固定寬度模式時，新開啟的檔案仍可能在切換模式前以固定格線排列字元（比例字型下字元重疊）。尚未量測的字元現在會立即取得真實寬度，編輯字型變更後也會重新量測所有寬度。
- 由按鍵觸發的訊息（例如在「最近檔案」中對無法讀取的檔案按 Enter 後出現的錯誤）不會在出現前就被同一組按鍵關掉。
- 深色主題下尋找列背景原本是透明的，顯示成白色。
- 下拉箭頭 ▾ 與關閉按鈕原本顯示成空方框。
- macOS 上介面語言選單中的「한국어」原本顯示成空方框（PingFang 沒有韓文字），現在會自動加入韓文字型。
- Markdown 預覽中，欄位較短的表格條紋會超出表格框線。
- 深色 Markdown 預覽中，Mermaid 邊線標籤原本落在灰色方塊上，而不是黑色背景。
- 深色主題下，尋找（望遠鏡）、取代、換行、縮排、預覽等黑色工具列圖示幾乎看不見。現在深色主題使用自動變亮的圖示：外框與大面積黑色（例如望遠鏡的鏡身）會變亮，畫在淺色底上的細節則保留原色；淺色主題仍使用原始圖。圖示縮小到非高 DPI 螢幕時，細線也不會消失。
- 唯讀分頁上的 🔒 與 Markdown 程式碼區塊的複製按鈕原本顯示成空方框，現在已內建 emoji 字型。
- 淺色主題下，十六進位檢視的選取範圍與警告、錯誤文字對比不足。

## [0.6.3] - 2026-10-27

### Added
- Single instance: launching again or opening files from File Explorer sends files to the existing window, opens them in tabs, and restores and focuses the window. Reopening a file selects its existing tab. Different configuration directories remain independent, and screenshot tests run separately.

### Fixed
- Markdown preview now opens in a separate tab in the main window instead of a separate native window. Reopening selects the existing preview, and closing it keeps the source document and unsaved changes intact. Side-by-side and top-and-bottom layouts remain available.
- Opening a nonexistent file now asks whether to create it, as Notepad does, instead of immediately reporting an open failure. If created, saving writes to the same path. The confirmation message is translated into all supported UI languages.

## [0.6.2] - 2026-10-27

### Changed

- The default encoding is now UTF-8. A new fallback encoding option (Big5 by default) is used only when the content is not valid UTF-8. The default encoding from legacy settings is automatically migrated to the fallback encoding.

## [0.6.1] - 2026-10-27

### Fixed
- Linux AppImage packaging failed because the temporary desktop file lacked the `.desktop` extension.

## [0.6.0] - 2026-10-27

### Added
- The UI language can follow the system or be set to English, Traditional Chinese, Simplified Chinese, Japanese, Korean, German, Spanish, French, Italian or Portuguese. The selection applies to menus and the Options dialog.
- `Makefile` provides unified development and packaging commands (`make check`, `make test`, `make dist`).
- Linux AppImage packages for x86_64 and aarch64, including the executable and spelling dictionaries.
- Linux archives and Windows zip packages can target `x86_64` or `aarch64`.
- GitHub Actions builds Linux archives, Linux AppImages, macOS DMGs and Windows zip packages when a `v*` tag is pushed, using the matching section in this file as release notes. Linux and Windows include x64 and ARM builds; macOS includes Apple Silicon and Intel builds.
- Markdown preview supports two-way synchronized scrolling, enabled by default in side-by-side and separate-window layouts. Preview UI text is translated.
- Markdown preview reuses unchanged Mermaid diagrams instead of converting them again when body text is edited.
- `README.md` is now in English; the Traditional Chinese version has moved to `README_zh-TW.md`.

### Fixed
- Markdown preview sizing in side-by-side layouts and loading of ordinary and reference-style images, including file names with CJK characters, spaces and percent-encoding.

## [0.5.24] - 2026-10-26

### Added
- macOS native integration: Finder "Open With" and dropping files on the Dock icon. Quitting from the system menu or Dock asks about unsaved changes first.
- macOS App packaging now verifies the signature before replacing the old version. Added `scripts/make_dmg.sh` to create disk images.

### Fixed
- Crash when quitting on Touch Bar Macs, while preserving accessibility support.
- Save As and Save a Copy now trim leading and trailing whitespace from file names, along with extra trailing periods and spaces.

## [0.5.23] - 2026-10-26

### Added
- In-app Markdown preview (`egui_commonmark`), with ` ```mermaid ` diagrams rendered by the pure-Rust fermaid library.
- Preview zoom, fit-to-width diagrams, relative-path images, side-by-side and top-and-bottom layouts, and synchronized scrolling with the editor in split layouts.
- Preview state is saved per tab; preview cannot be opened for non-Markdown files.
- In-app print preview with A4 pages, paging and zoom.
- Windows release archive script and one-step Linux installation to `~/.local`.

### Fixed
- Upgraded egui / eframe to 0.37.
- Text overlap caused by multiple Markdown tables sharing column widths.
- Black preview background in light mode; split layouts now give the preview and editor half the space each.
- Panic when deleting a multiline selection with word wrap enabled.
- Saving, Save a Copy and background replacement now use atomic writes. Corrupt settings files are backed up as `.bak`.
- Closing now resumes correctly when saving requires an additional confirmation.
- Shortcut capture no longer binds editing keys such as Enter and Backspace.

## [0.5.22] - 2026-10-23

### Added
- Linux release archives (cargo-zigbuild, glibc 2.28) and an installation script targeting `~/.local`.
- One-step macOS build and installation to `/Applications`.
- `Ctrl` / `⌘` + `1`–`9` tab switching.

### Fixed
- Startup failure with older Mesa software rendering.
- Linux icons are installed to `48x48`; the application can also be upgraded while running.

## [0.5.21] - 2026-10-23

### Added
- The window and toolbar now use the original MadEdit icons.

### Fixed
- Dialogs are centered when first opened.
- UI fonts retain CJK fallback when the editor uses a CJK font.

## [0.2.0] - 2026-10-23

### Added
- Rewrote the UI from Slint to egui / eframe, with custom-drawn text and hex views, scrollbars and ruler.
- User-selectable system fonts, including proportional fonts, automatic CJK fallback, and macOS input method preedit and candidate window positioning.
- Implemented 165 commands from the original command table: multi-document tabs, undo / redo, find and replace, column mode, bookmarks, encodings, macros, spelling, printing and options.
- Large files now open through a Piece Table with positioned reads instead of mmap.

## [0.1.0] - 2026-10-21

### Added
- Initial project. The Slint version implemented encoding support, a custom-drawn editor, a large-file engine, rectangular selection, find and replace, syntax highlighting, go-to-line, line endings, bookmarks, external change monitoring and the classic UI.

[Unreleased]: https://github.com/tenyi/MadEdit/compare/v0.6.7...HEAD
[0.6.7]: https://github.com/tenyi/MadEdit/compare/v0.6.6...v0.6.7
[0.6.6]: https://github.com/tenyi/MadEdit/compare/v0.6.5...v0.6.6
[0.6.5]: https://github.com/tenyi/MadEdit/compare/v0.6.4...v0.6.5
[0.6.4]: https://github.com/tenyi/MadEdit/compare/v0.6.3...v0.6.4
[0.6.3]: https://github.com/tenyi/MadEdit/compare/v0.6.2...v0.6.3
[0.6.2]: https://github.com/tenyi/MadEdit/compare/v0.6.1...v0.6.2
[0.6.1]: https://github.com/tenyi/MadEdit/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/tenyi/MadEdit/compare/v0.5.24...v0.6.0
[0.5.24]: https://github.com/tenyi/MadEdit/compare/v0.5.23...v0.5.24
[0.5.23]: https://github.com/tenyi/MadEdit/compare/v0.5.22...v0.5.23
[0.5.22]: https://github.com/tenyi/MadEdit/compare/v0.5.21...v0.5.22
[0.5.21]: https://github.com/tenyi/MadEdit/compare/v0.2.0...v0.5.21
[0.2.0]: https://github.com/tenyi/MadEdit/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/tenyi/MadEdit/releases/tag/v0.1.0
