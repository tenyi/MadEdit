# Changelog

This file is the source of user-visible changes. GitHub Release notes are taken directly from the matching `## [version]` section (see the release instructions in README).

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and version numbers follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- The Find bar becomes a floating, modeless "Find and Replace" window (like UltraEdit): the editor stays usable while it is open, the window drags by its compact title row (or any blank area), closes with the title-row "×" or Escape, and remembers its position across reopenings and restarts. It is split into "Find" and "Replace" tabs (Ctrl+F / Ctrl+H) with a File-View-style tab strip, and the search and replace boxes are 4-row multiline fields (fixed height, internal scrolling) so multi-line text can be searched. Enter runs Find next, Shift+Enter runs Find previous, Ctrl+Enter inserts a newline. A query spanning several lines is matched across lines, and Replace All handles cross-line matches with bookmark remapping. The search options (whole word, match case, regex, hex, wrap around) are grouped under an "Options" gear at the bottom-left. The search and replace history opens from a clock button at the top-right of each field (UltraEdit-style): one row per entry with a red "×" at the right end to remove it and "Clear search history" at the bottom, and picking an entry closes the list. The title bar's Quick find box gets the same history dropdown.

### 繁體中文

#### 新增
- 查找列改為浮動的非模態「查找和替換」視窗（仿 UltraEdit）：開啟時編輯器仍可正常操作，視窗可拖動緊湊標題列（或任意空白處）移動、以標題列右端「×」或 Esc 關閉，並記住位置（關閉後重開與重新啟動皆回到原處）。視窗分為「查找」「替換」兩個分頁（Ctrl+F／Ctrl+H），頁籤樣式與檔案視圖停靠欄一致；查找與替換欄位為 4 行多行輸入框（固定高度、內部捲動），可搜尋含換行的內容。純 Enter＝查找下一個、Shift+Enter＝查找上一個、Ctrl+Enter＝插入換行。跨行模式：含換行的查詢會跨行比對，「全部取代」也能處理跨行符合並位移書籤。搜尋選項（全字比對／區分大小寫／正則／十六進位／循環）收合於左下角「選項」齒輪。查找與替換欄位的歷史改為欄位右上角的時鐘鈕下拉（仿 UltraEdit）：每一列右端 hover 紅色「×」移除單筆、底部「清空搜尋紀錄」，點選任一筆自動收起清單。標題欄的「快速查找」框也接入同一套歷史下拉。

## [0.6.10] - 2026-11-25

### Added
- A dockable File View panel (View > File View): its tabbed left panel has a Favorites tree for grouping files. Right-click the Favorites root to add a group, a folder's files, individual files, the active file or all open files, and to switch between names and full paths; right-click a group to rename or delete it, and double-click it to open every file in the group. Missing files are skipped with a status message. Its Open tab lists every editor tab, including preview tabs, for switching files when the tab bar is crowded. Recently opened files are listed below, and favorites persist in `settings.json` without touching the files on disk.
- PowerShell scripts (`.ps1`, `.psm1`, `.psd1`) open with bundled syntax highlighting: comments, strings, variables, cmdlets, keywords and here-strings are colored without any external files or network access.
- Squirrel scripts (`.nut`, <https://squirrel-lang.org/>) open with bundled syntax highlighting: classes, functions, keywords, numbers, strings and `//`, `/* */` comments are colored offline.
- The bundled Squirrel definition also covers the user's ADO database, Win32 and MFC keyword extensions (`ADOSQLCommand`, `ADODataset`, `AttachToDB`, `MB_OK`, `CString`, …); types, functions and constants use distinct colors in the light and dark themes.
- `.svg` files can be previewed as a graphic (View > Preview): resvg renders the current text into an image shown in a tab or a docked panel, with zoom buttons and Ctrl+wheel zoom, live refresh while editing and parse errors reported in the panel. The menu item is now a checked toggle stored in the settings: while it is checked, opening a `.svg` shows the graphic directly; unchecking closes an active graphic preview.
- Raster images (`.bmp`, `.jpg`, `.jpeg`, plus `.png`/`.gif`/`.webp`) can be previewed as a graphic the same way (View > Preview): the file is decoded from disk with the `image` crate and shown in a tab or docked panel with zoom, because these files open in hex mode. Its menu item is a checked toggle stored in the settings: while checked, opening such an image shows the graphic directly; unchecking closes an active image preview. Oversized images are scaled down to the GPU texture limit before display.
- Code folding (View > Code Folding): click the gutter triangles to collapse `{ }` blocks or multi-line comments (consecutive `//` lines, `/* … */`) in C-like files, indentation blocks in Python (plus `#` runs), and braces/comments in PowerShell (`#`, `<# … #>`, here-strings skipped), with `⋯ N lines` on the header, Fold / Unfold (Ctrl+Shift+`[`), Fold All / Unfold All, and automatic unfolding around the caret. Off by default.
- Multi-line `[ … ]` arrays fold like braces: in C/C++/JSON/Java/JavaScript and the other brace-based languages a line that leaves a `[` open folds until the line that closes it (brackets inside strings and comments are ignored), and Python folds multi-line array literals the same way.
- INI files (`.ini`, `.cfg`, `.conf`) open with bundled highlighting for sections, keys, values and `;` comments.
- `.gitignore` (plus `.ignore` and `.dockerignore`) files open with bundled highlighting: `#` comments, `!` negation, glob operators (`*`, `?`, `**`), `[a-z]` character classes, `\` escapes and path separators are colored; `#` works as the line-comment command.
- Tools > Run Script (F5) runs the current file, or only the selection, and streams stdout/stderr into a resizable "Script Output" panel with exit code, elapsed time and a Stop button. `.nut` uses `nut.exe`, `.py` uses `python -u`, `.lua` uses `lua`; output is decoded as UTF-8 or GBK. F5 previously opened the previous window, which moved to F10.
- A selection run writes the temporary file in the document's own encoding (a GBK script stays GBK), and the elapsed time stops counting after the run finishes.
- Options > General has "Automatically reload files changed by other programs". When enabled, a file that was changed on disk and has no unsaved edits is reloaded without the banner; files with unsaved edits still show the usual prompt.
- Opening a file now detects Big5, GBK/GB18030, Shift_JIS, EUC-KR, EUC-JP and similar encodings from its content (UTF-8 and BOMs are handled first). A GBK `.ini` file opens as GBK instead of being decoded with the Big5 fallback, so Chinese text is no longer garbled.
- The text in the Script Output panel can be selected and copied: drag to select, Ctrl+C to copy, and right-click (which keeps the selection, like the editor) for Select All, Copy and Clear. Long output still scrolls in the panel; past 5,000 lines or 200 KB the oldest lines are dropped (the panel says how many) so huge outputs cannot slow down the interface.
- The output panel's context menu has an "Output to file (result.log)" checkbox: when checked, the file is opened from scratch and the panel's current output is written into it (keeping both in sync), then every new line is also written to `result.log` next to `MadEdit.exe` with no size limit — the on-screen cap does not apply to the file. Each F5 run starts the file from scratch; clearing the panel does not touch the file. The checkbox state is remembered across sessions.
- The editor tab context menu has "Open Folder", which opens the file's folder in the system file manager (with the file selected on Windows and macOS). It is disabled for unsaved documents.
- Tools > Options > General has an "Interface font" setting (Automatic by default) for menus, dialogs, tabs and the status bar. The editor and the Script Output panel keep using the editor font.
- View > Script Output shows or hides the Script Output panel and is checked while the panel is open, the same way View > File View works. The panel's state is remembered, including when it was closed with the panel's × or opened automatically by running a script, so the next launch starts the same way.
- Left double-clicking an editor tab's title closes that tab, like its × button; double-clicking a Markdown preview tab closes the preview. Unsaved changes are still confirmed first.
- The Script Output panel accepts input while a script is running: a prompt-style input line (`>`) appears at the bottom of the panel, focused once when the process starts, and pressing Enter writes the line to the child process's stdin (echoed as `> line` in the output and `result.log`). Line-based interaction such as batch `set /p` or Python `input()` now works; programs that read the console directly (`pause`, `choice`, full-screen TUIs) still need the Stop button or a real terminal.
- The File View panel's Open tab shows a "×" at the right end of a row while it is hovered or selected; clicking it closes that tab (a Markdown preview row closes the preview). Double-clicking a row closes it the same way as double-clicking the editor tab's title. Unsaved changes are still confirmed first.
- The left File View dock can be dragged up to 800 px wide (was 420), and the Explorer tree and content list scroll horizontally when a deep path or a long file name does not fit. The list header stays fixed while the list scrolls vertically, drag a column border to resize that column (the widths are remembered), and a column header click sorts by that column (four-way choice, second click reverses, ▲/▼ marks the active column). While split view is checked the tree stays a plain folder list (the contents are shown in the list below); unchecking it makes the selected folder render its subfolders and files inline again.
- The File View panel gains an "Explorer" tab: a lazily loaded drive/folder tree (a folder is read only when first expanded) with a filter box that applies only after clicking ">" (or pressing Enter). The filter matches the selected folder's file names only (`*` / `?` wildcards, `;`-separated): matching files appear as leaf rows under the selected folder, unselected folders stay a pure folder tree, and single-clicking a file opens it. A split-view toggle shows the selected folder's contents below the tree — a Name / Modified / Type / Size list (columns adapt to the panel width) with a draggable splitter; double-clicking a folder in the list enters it. Rows show the real Windows shell icons (folders, files by type, drives), a ".." row at the top returns to the parent folder, and clicking a column header sorts by that column (four-way choice, second click reverses, ▲/▼ marks the active column). The list header stays fixed while the list scrolls.
- The editor uses the bundled **Fixedsys Excelsior 3.01** font (FSEX300) at size 16 by default: the font is embedded in the executable, so the classic Fixedsys look works with no font installation on any platform. CJK and other scripts still fall back to the platform fonts; choosing another font in View > Font overrides the default as before.
- The Script Output panel's "Output to file" log now follows the script: F5 writes `<script folder>/<script file name>.result` (for example `a.nut` → `a.nut.result`, `a.py` → `a.py.result`, `run.ztrtoml` → `run.ztrtoml.result`), so every script keeps its own log right next to it — sorting by name puts each log beside its script, and same-named scripts in different languages never overwrite each other. Unsaved selection runs (no script file) still fall back to `result.log` next to MadEdit.exe, and an explicit log path overrides the rule.

### Changed
- View > Preview, View > File View and View > Script Output are now checked by default: the best-experience workspace (auto preview dispatch, the File View dock and the Script Output panel) is shown from the first launch. Unchecking any of them hides it as before and is remembered.
- The four preview entries (Markdown / HTML / SVG / image) are merged into a single checked toggle "Preview" (View menu, and the tab & editor "Preview as" context menus): while checked, opening a file dispatches automatically by extension — `.md`/`.markdown` to the Markdown preview, `.html`/`.htm` to the HTML preview, `.svg` to the graphic preview, `.bmp`/`.jpg`/`.jpeg`/`.png`/`.gif`/`.webp` to the image preview; other types show a status message. Unchecking closes every active preview. The old per-type settings keys are ignored (the new `auto_preview` key replaces them).
- Editor tabs can be dragged to reorder them with a live preview (like MTT-File-Manager): while dragging, the source tab lifts out with a shadow and follows the pointer while the neighbors animate aside to open a slot; the move commits only on release. Clicking / double-clicking / the close button behave as before.
- The Squirrel interpreter is now expected as `nut.exe` (previously `ZSSQConsole.exe`) next to MadEdit.exe or on PATH, and `.nut` files show "nut" as their script type in the status bar (syntax highlighting stays the Squirrel definition).
- Tools > Run Script (F5) recognizes `*.ztrtoml` files (config files of the ztr compression tool): instead of script interpretation it runs `ztr --ztrtoml <file name>` with the working directory set to the file's own folder, so the tool compresses that folder with this configuration. It prefers `ztr.exe` sitting next to MadEdit.exe (bundled release), falling back to PATH; unsaved changes are saved first, as with other F5 runs. The status bar shows the script type as "ztr" (highlighting uses the YAML syntax).
- In the File View panel's List tab, hovering a favorites entry or a recent-file entry shows a red "×" at the end of the row: clicking it removes only that entry from the favorites/recent list (the file on disk is untouched).
- Duplicate Line (Edit menu) also accepts `Ctrl+D` (it keeps `Ctrl+Shift+D`), and Cut (`Ctrl+X`) behaves like Visual Studio in text mode: with no selection it cuts the whole current line; with a selection it cuts as before. Hex mode and read-only documents are unaffected.
- Windows: the menu bar now lives in a custom-drawn title bar together with the app icon and the window buttons (minimize / maximize / close), like VS Code. The bar is 1.5× the menu height, is draggable, and double-clicking toggles maximize; edge resizing and Aero Snap still work. The zoom controls, theme toggle and quick search stay on the bar's right side. Other platforms keep the native title bar and menu bar. Because the window buttons are ordinary EGUI widgets, Windows 11 does not show the Snap Layouts flyout when hovering the maximize button.
- Editor tabs and the File View panel's tab now use a flat, content-connected style with rounded top corners only (like Windows 11 Explorer), instead of detached rounded buttons, so the selected tab merges with the pane below it.
- Closing the last tab leaves an empty workspace: MadEdit neither creates an untitled document nor exits, and the editor area offers New and Open.
- Reloading a file (manually or automatically after an external change) keeps the current scroll position instead of jumping back to the caret or the top of the file.
- The toolbar's text, column and hex mode buttons are now one three-way choice: a single outlined group holds all three icons, the current mode is filled in the accent colour with an accent outline, and assistive technology is told these are radio buttons instead of three unrelated toggles.
- The title bar's minimize / maximize / close buttons follow the Windows 11 style: flat full-height tiles without frames of their own, thin drawn glyphs instead of font characters, and Close turns red with a white glyph on hover.
- In the dark theme the editor area and the tab strip now use the same background as the other panels (the Script Output dock, side panels) instead of the syntax theme's own grey-blue background, which looked hazy next to them; light theme keeps the theme's white background.
- The title bar's font zoom buttons (A− / A+) no longer paint a background while idle, so they blend into the bar in both themes; the border and hover feedback remain.
- The horizontal separator line between the menu bar and the toolbar is gone, as are all separators around the title bar's quick controls (font zoom, theme toggle, quick search); at fractional scale factors the title bar height now snaps to whole physical pixels, which removes the thin dark line that could show at the panel seam on 100% / 125% displays.

### Fixed
- Markdown preview: SVG images are no longer rasterized at their natural size — some packing-record SVGs measure tens of thousands of pixels and froze the interface for minutes (and oversized ones crashed the renderer on the GPU texture-size limit). SVGs now render at a size bounded by the texture limit and are downscaled proportionally when needed.
- Markdown preview: relative-path images inside HTML `<img src="data/logo.jpeg">` tags (common in README headers) now resolve against the document's folder and render as images with their `height` attribute honored (falling back to fit-to-width), so sizes match the README's intent like in VSCode. GIF files animate (the `egui_extras/gif` loader feature is now enabled). Code-block examples are untouched.
- Markdown preview keeps remote images offline and scales large local SVGs to the GPU texture limit.
- Windows: paths shown to the user no longer carry the `\\?\` prefix that `canonicalize` adds. The tab tooltip, Copy Full Path, the File View list, the open/create status messages, the "create this file?" prompt and the print header now show `D:\work\a.txt` instead of `\\?\D:\work\a.txt`.

### 繁體中文

#### 新增
- 可停靠的「文件視圖」面板（檢視 > 文件視圖）：左側分頁面板中的「收藏」樹可將檔案分組。在「收藏」上按右鍵可新增群組、資料夾中的檔案、個別檔案、使用中的檔案或所有開啟的檔案，並切換顯示檔名／完整路徑；在群組上按右鍵可重新命名或刪除，連按兩下則開啟群組中的所有檔案，已不存在的檔案會略過並顯示訊息。「開啟」頁籤列出所有編輯器分頁（含預覽分頁），分頁列放不下時可切換檔案。下方另列最近開啟的檔案；收藏存於 `settings.json`，不會更動磁碟上的檔案。
- PowerShell 腳本（`.ps1`、`.psm1`、`.psd1`）內建語法高亮：註解、字串、變數、Cmdlet、關鍵字與 here-string 都會著色，不需要外部檔案或網路。
- Squirrel 腳本（`.nut`，<https://squirrel-lang.org/>）內建語法高亮：類別、函式、關鍵字、數字、字串與 `//`、`/* */` 註解都會著色，可離線使用。
- 內建 Squirrel 定義也涵蓋 ADO 資料庫、Win32 與 MFC 擴充關鍵字（`ADOSQLCommand`、`ADODataset`、`AttachToDB`、`MB_OK`、`CString` 等）；型別、函式與常數在淺色與深色主題各有不同顏色。
- `.svg` 檔案可以圖形預覽（檢視 > 預覽）：以 resvg 將目前內容點陣化，顯示在獨立分頁或並排面板，支援縮放按鈕與 Ctrl+滾輪、編輯時即時更新，解析失敗會在面板顯示訊息。此選單項改為**勾選式並寫入設定**：勾選狀態下打開 `.svg` 直接以圖形顯示；取消勾選會關閉目前的圖形預覽。
- 點陣圖片（`.bmp`、`.jpg`、`.jpeg`，以及 `.png`／`.gif`／`.webp`）同樣可以圖形預覽（檢視 > 預覽）：這類檔案以十六進位模式開啟，預覽因此直接從磁碟解碼（`image` crate）後顯示在獨立分頁或並排面板，支援縮放；選單項同為**勾選式並寫入設定**，勾選時打開圖片直接以圖形顯示，取消勾選會關閉圖片預覽。超過 GPU 紋理上限的巨型圖會先縮到上限再顯示。
- 代碼折疊（檢視 > 代碼折疊）：點行號右側小三角可折疊 C 系檔案的 `{ }` 區塊或多行註解（連續 `//` 行、`/* … */`）、Python 的縮排區塊（含 `#` 註解群組），以及 PowerShell 的 `{ }` 與 `#`、`<# … #>` 註解（here-string 會跳過），折疊行顯示「⋯ N 行」；另有折疊/展開（Ctrl+Shift+`[`）、全部折疊、全部展開，游標所在區塊會自動展開。預設關閉。
- 多行 `[ … ]` 陣列比照大括號折疊：C/C++/JSON/Java/JavaScript 等大括號語系中，一行結束時仍有未閉合的 `[`，折到閉合它的那一行（字串與註解裡的括號會忽略）；Python 的多行陣列字面值同樣可折疊。
- INI 檔案（`.ini`、`.cfg`、`.conf`）內建高亮：節區、鍵、值與 `;` 註解都會著色。
- `.gitignore`（含 `.ignore`、`.dockerignore`）內建高亮：`#` 註解、`!` 否定、萬用字元（`*`、`?`、`**`）、`[a-z]` 字元組、`\` 轉義與路徑分隔都會著色，`#` 同時是行註解命令。
- 「工具 > 執行腳本」（F5）可執行目前檔案或僅執行選取片段，並把 stdout/stderr 串流到可調整大小的「執行輸出」面板，顯示退出碼、耗時與「停止」按鈕。`.nut` 使用 `nut.exe`、`.py` 使用 `python -u`、`.lua` 使用 `lua`；輸出以 UTF-8 或 GBK 解碼。F5 原本是「上一個視窗」，該功能改到 F10。
- 執行選取片段時，暫存檔沿用文件本身編碼（GBK 腳本仍是 GBK）；執行結束後耗時不再繼續增加。
- 「工具 > 選項 > 一般」新增「其他程式修改時自動重新載入」：開啟後，磁碟上被修改且沒有未儲存編輯的檔案會直接重新載入，不再顯示提示；有未儲存編輯的檔案仍保留原本的提示流程。
- 開檔時會依內容辨識 Big5、GBK/GB18030、Shift_JIS、EUC-KR、EUC-JP 等編碼（UTF-8 與 BOM 優先）。GBK 的 `.ini` 會以 GBK 開啟，不再被 Big5 備援編碼解成亂碼。
- 「執行輸出」面板的文字可以選取與複製：拖曳選取、Ctrl+C 複製，右鍵選單提供「全選／複製／清空」；右鍵不會取消選取（與編輯區一致），長輸出仍可在面板內捲動。輸出超過 5,000 行或 200 KB 時自動捨棄較早的行（面板會顯示省略了多少行），大量輸出不會拖慢介面。
- 「執行輸出」面板右鍵選單新增「輸出到檔案（result.log）」勾選項：勾選時以清空方式開檔，並把面板目前的輸出整段寫入（兩邊同步），之後每一行都同時寫入 `MadEdit.exe` 同目錄的 `result.log`，檔案沒有大小限制（面板截斷不影響檔案內容）。每次按 F5 運行時檔案從頭寫；中途清除面板不影響檔案。勾選狀態會記住，重開程式沿用。
- 編輯器分頁右鍵選單新增「開啟檔案所在資料夾」：用系統檔案管理員開啟檔案所在資料夾（Windows/macOS 上會選中該檔案）；未存檔的文件此項停用。
- 「工具 > 選項 > 一般」新增「介面字型」（預設自動）：指定選單、對話框、分頁與狀態列的字型；編輯區與「執行輸出」面板仍使用編輯區字型。
- 「檢視 > 運行輸出」可顯示或隱藏「運行輸出」面板，面板開啟時選單會打勾（與「檢視 > 文件視圖」一致）。面板狀態會記住：無論是從選單切換、按面板的 × 關閉，還是執行腳本時自動開啟，下次啟動都沿用最後的顯示／隱藏狀態。
- 以滑鼠左鍵雙擊編輯器分頁標題即可關閉該分頁（與 × 按鈕相同）；雙擊 Markdown 預覽分頁則關閉該預覽。有未儲存的變更時仍會先詢問。
- 運行期間「執行輸出」面板可輸入：面板最底部出現提示符（`>`）樣式的輸入行，程序啟動時自動聚焦一次，按 Enter 把該行寫進子程序的 stdin（並以「> 行」回顯到輸出與 `result.log`）。批次檔的 `set /p`、Python 的 `input()` 等行式互動因此可用；直接讀主控台按鍵的程式（`pause`、`choice`、全螢幕 TUI）仍需按「停止」或改用真正的終端。
- 檔案檢視面板的「開啟」頁籤在列被滑鼠懸停或選中時於列尾顯示「×」，點擊即關閉該分頁（Markdown 預覽列則關閉預覽）；雙擊列與雙擊編輯器分頁標題同效，同樣關閉。未存檔修改仍會先確認。
- 左側檔案檢視停靠欄可拖曳加寬至 800 px（原上限 420），資源管理器的目錄樹與內容列表在深層路徑或長檔名超出範圍時提供**橫向捲軸**；列表標題固定不隨垂直捲動，可用滑鼠拖曳欄位邊界調整欄寬（欄寬會記住），點擊列頭依該欄排序（四選一，再點切換升降冪，以 ▲/▼ 標示）。勾選分欄時樹保持純資料夾清單（內容由下方列表呈現）；取消勾選後選中資料夾會重新把子資料夾與檔案內聯到樹上。
- 檔案檢視面板新增「資源管理器」頁籤：懶載入的磁碟／資料夾樹（資料夾首次展開時才讀取），附篩選框——按「>」或 Enter 後才生效，只匹配**選中資料夾**的檔名（`*`／`?` 萬用字元，可用 `;` 分隔多個）：匹配的檔案以葉節點顯示在選中資料夾下，未選中的節點維持純資料夾樹；單擊檔案葉節點即開啟該檔案。勾選分欄開關後，樹下方以「名稱／修改日期／類型／大小」列表顯示所選資料夾內容（欄位隨面板寬度自適應），並可用拖拽分隔條調整上下高度；雙擊列表中的資料夾即進入。列前置**真實 Windows 系統圖示**（資料夾、依副檔名的檔案、磁碟機）；列表首項「..」單擊回上層；點擊列頭依該欄排序（四選一，再點切換升降冪，作用中的欄位以 ▲/▼ 標示）；列表標題行固定，不隨列表捲動。
- 編輯區預設使用內建的 **Fixedsys Excelsior 3.01** 字型（FSEX300），大小 16：字型直接嵌入執行檔，任何平台都無需安裝即可重現經典 Fixedsys 觀感。中日韓等字元仍走平台後備字型；在「檢視 > 字型」選擇其他字型即可覆蓋預設。
- 「執行輸出」面板的「輸出到檔案」記錄檔改為跟著腳本走：F5 寫入「腳本所在目錄／腳本檔名.result」（例如 `a.nut` → `a.nut.result`、`a.py` → `a.py.result`、`run.ztrtoml` → `run.ztrtoml.result`），每個腳本的日誌各自放在旁邊——按檔名排序時日誌緊挨著腳本、綁定關係一目了然；同目錄下同名不同語言的腳本（`a.nut`／`a.py`／`a.ps1`）也互不覆蓋。未存檔的選取片段運行（無腳本檔案）仍退回 MadEdit.exe 同目錄的 `result.log`，使用者自訂的記錄檔路徑優先於此規則。

#### 變更
- 「檢視 > 預覽」「檢視 > 文件視圖」「檢視 > 運行輸出」改為**預設勾選**：第一次啟動即是體驗最好的工作區（自動預覽分發、文件視圖停靠欄與運行輸出面板）。取消勾選照舊隱藏對應面板並記住狀態。
- 四個預覽項目（Markdown／HTML／SVG／圖片）合併為單一勾選開關「預覽」（檢視選單，以及分頁與編輯區的「預覽為」右鍵選單同步）：勾選時打開檔案依副檔名自動分發——`.md`/`.markdown` 進 Markdown 預覽、`.html`/`.htm` 進 HTML 預覽、`.svg` 進圖形預覽、`.bmp`/`.jpg`/`.jpeg`/`.png`/`.gif`/`.webp` 進圖片預覽，其他類型顯示狀態訊息；取消勾選會關閉所有作用中的預覽。舊的分項設定鍵不再使用（改由新的 `auto_preview` 鍵取代）。
- 編輯器分頁標題支援拖動排序，並帶即時預覽（比照 MTT-File-Manager）：拖動時來源分頁浮起（帶陰影）跟隨指針，鄰居以動畫滑開一個空位；鬆手才提交搬移。單擊、雙擊與關閉按鈕行為不變。
- Squirrel 直譯器改為使用 `nut.exe`（原 `ZSSQConsole.exe`），放在 `MadEdit.exe` 同目錄或 PATH；`.nut` 檔在狀態欄的腳本類型顯示為「nut」（語法高亮仍是 Squirrel 定義）。
- 「工具 > 執行腳本」（F5）支援 `*.ztrtoml` 檔案（ztr 壓縮工具的配置檔）：不再當腳本解譯，而是以該檔所在目錄為工作目錄執行 `ztr --ztrtoml <檔名>`，直接按此配置壓縮該目錄。優先使用 MadEdit.exe 同目錄的 `ztr.exe`（可捆綁發佈），找不到才依賴 PATH；與其他 F5 運行一樣，未存檔修改會先存檔。狀態欄的腳本類型顯示為「ztr」（高亮沿用 YAML 樣式）。
- 「文件視圖」面板的「列表」頁中，滑鼠懸停在收藏條目或最近開啟條目上時，列尾會出現紅色「×」：點擊只把該條目從收藏／最近清單移除（不會刪除磁碟上的檔案）。
- 「複製整行」（編輯選單）新增 `Ctrl+D` 快捷鍵（原有 `Ctrl+Shift+D` 保留）；文字模式下「剪下」（`Ctrl+X`）比照 Visual Studio：沒有選取時剪下目前整行，有選取時行為不變。十六進位模式與唯讀文件不受影響。
- Windows：選單列移進自繪標題欄，與應用程式圖示、視窗按鈕（最小化／最大化／關閉）同一列（類似 VS Code）。標題欄高度為選單高度的 1.5 倍，可拖動、雙擊可最大化／還原，邊緣縮放與 Aero Snap 貼邊正常。字型縮放、主題切換與快速搜尋保留在同一列右側。其他平台維持原生標題列與選單列。視窗按鈕是一般 EGUI 控件，因此 Windows 11 懸停最大化按鈕時不會顯示 Snap Layouts 浮層。
- 編輯器分頁與「文件視圖」面板的分頁改為上緣圓角、下緣與內容相連的扁平樣式（類似 Windows 11 檔案總管），不再是有分離感的圓角按鈕；選取的分頁會與下方內容連成一體。
- 關閉最後一個分頁後改為空白工作區：不再自動建立未命名文件，也不會結束程式；編輯區提供「新增」與「開啟」按鈕。
- 重新載入檔案（手動或偵測到外部修改後自動載入）會保留目前的捲動位置，不再跳回游標或檔案開頭。
- 工具列的「文字模式／列塊模式／十六進位模式」改為同一組三選一：三個圖示收在同一個外框內，目前的模式以強調色填滿並加上強調色外框；螢幕閱讀器會讀成選項按鈕（radio），不再是三個互不相關的切換鈕。
- 標題欄的最小化／最大化／關閉按鈕改為 Windows 11 風格：整欄高的平面熱區、不再自帶底框，字形改由線段繪製（不依賴字型字元），懸停時整格變色，關閉鍵懸停變紅底白叉。
- 暗色主題下，編輯區與分頁列改用與其他面板（執行輸出停靠欄、側邊面板）一致的背景色，不再使用語法主題自帶的灰藍底（與整體介面相比顯得灰蒙蒙）；淺色主題維持主題原本的白底。
- 標題欄的字型縮放按鈕（A−／A+）閒置時不再繪製底色，深淺主題都與標題欄融為一體；邊框與懸停回饋保留。
- 選單列與工具列之間的橫向分隔線移除，標題欄快捷控件（字型縮放、主題切換、快速搜尋）周圍的豎向分隔線也一併移除；標題欄高度在非整數縮放比例（100%／125% 等）下吸附到整數物理像素，消除面板接縫處可能出現的細黑線。

#### 修正
- Markdown 預覽：SVG 圖片不再按原生尺寸點陣化——部分打包記錄 SVG 原生尺寸高達數萬像素，會讓介面凍結數分鐘，超過 GPU 紋理上限的還會直接崩潰；現在渲染尺寸以紋理上限為界，必要時等比縮小。
- Markdown 預覽：HTML `<img src="data/logo.jpeg">` 標籤內的相對路徑圖片（README 頁首常見）現在會以文件所在資料夾解析並按 `height` 屬性的高度等比顯示（無 height 時自適應寬度），尺寸與 README 原意（如 VSCode）一致；GIF 檔會播放動畫（啟用 `egui_extras/gif` 載入器特性）；程式碼區塊裡的 `<img>` 範例不受影響。
- Markdown 預覽維持離線圖片處理，並依 GPU 紋理上限縮小大型本機 SVG。
- Windows：顯示給使用者的路徑不再帶 `canonicalize` 產生的 `\\?\` 前綴。分頁提示、「複製完整路徑」、文件視圖清單、開啟／建立的狀態訊息、「是否建立新檔案」詢問與列印頁首都改成 `D:\work\a.txt` 這種寫法，不再是 `\\?\D:\work\a.txt`。

## [0.6.9] - 2026-11-10

### Changed
- Simplified → Traditional Chinese conversion now matches phrases (longest first) to resolve ambiguous characters, e.g. 头发→頭髮 vs 开发→開發, 干净→乾淨 vs 干部→幹部, 面条→麵條, 关系→關係. Characters whose per-character default is usually wrong (面, 制, 干, 系, 只, …) are converted only inside known phrases. Safe mode still applies phrase matches. Traditional → Simplified keeps 乾坤/乾隆 intact. "Safe mode" is now off by default, so common characters such as 发, 后 and 于 are converted.

### Fixed
- Windows MSVC builds statically link the C runtime, so MadEdit.exe does not require a separate Visual C++ Redistributable.
- Linux AppImage packages use distinct clean and dprint filenames and include the requested formatter bundle.

### 繁體中文

#### 變更
- 簡轉繁改用詞組比對（最長優先）判斷歧義字，例如 头发→頭髮、开发→開發，干净→乾淨、干部→幹部，面条→麵條、关系→關係。「面、制、干、系、只」等逐字預設常出錯的字，只在已知詞組內轉換；安全模式仍會套用詞組比對。繁轉簡保留「乾坤」「乾隆」。「安全模式」預設改為關閉，因此「发、后、于」等常用字會被轉換。

#### 修正
- Windows MSVC 版本改為靜態連結 C 執行階段，執行 MadEdit.exe 不再需要另行安裝 Visual C++ 可轉散發套件。
- Linux AppImage 的一般版及 dprint 版使用不同檔名，並正確包含選用的格式化工具。

## [0.6.8] - 2026-11-03

### Added
- The text editor renders syntax-defined bold and italic styles, including Markdown headings and emphasis, without changing caret, selection or wrapping metrics.
- Load custom `.sublime-syntax` definitions and `.tmTheme` themes from `syntaxes/` and `themes/` beside the settings file at startup. Custom extension matches take precedence over approximate language fallbacks; malformed files do not prevent other resources from loading.

### Changed
- Replace the oversized agent handover/history with a concise, source-checked contributor guide covering safe branch/PR workflow, current architecture, verification tiers and platform limitations.
- Mark ADR 0002's text-storage and mmap design as historical and partially superseded by ADR 0011, with links to the current text, hex and positioned-read implementations.
- Split the custom text editor into layout, painting and input modules while preserving its public API and rendered output.
- Dialogs have a more modern look and keyboard behaviour. Controls are taller and footer buttons wider. Text fields are outlined, so they no longer disappear on light-theme cards. Hint text is readable rather than small, list filters show a magnifier, empty lists say why they are empty, and keyboard shortcuts appear as key caps. Esc closes a dialog and Enter runs its main action. While a dialog is open, typing no longer reaches the document behind it, and focus returns to the editor when the dialog closes.
- The find bar is redesigned. It counts matches as you type ("3 / 12", "No results", or the regex error) and highlights every match in the editor, so the Count button is gone. Match case, whole word, regex, hex and wrap-around are compact toggle chips, the history is a drop-down beside the field, and previous / next / close are icon buttons with their shortcuts in the tooltip. Replace and Replace All are disabled in read-only and hex documents. The go-to bar shows the valid line range and marks invalid input in red.
- The Find in Files results panel groups matches under each file, with a match count and a fold arrow; the "Collapse all results" command folds them all. Each match shows its line number and the line with the match highlighted, without the indent and scrolled to the match on long lines. Replacements list each file changed, skipped or failed (including open and read-only files) instead of mixing them into the matches. A spinner and a Stop button appear while a search runs.

### Fixed
- Syntax highlighting stays aligned after inserting or deleting lines, including undo/redo and edits during time-sliced parsing.
- Debug builds detect edits outside their declared line range using allocation-free fingerprints, protecting undo and incremental caches without copying untouched lines.
- Whole-document edits reuse the undo snapshot's changed strings instead of cloning them twice. Large canonical Unicode edits release original bytes before snapshotting; no-op commands still preserve the original bytes, and malformed, legacy-encoded or mixed-newline input retains its lossless recovery data.
- Character-position queries reuse a versioned line-start offset across local edits and nearby cursor moves instead of rescanning the document prefix on every keystroke. Undo/redo and full resets invalidate the offset when necessary.
- The print preview window follows the UI language (title, toolbar and page counter were always Traditional Chinese). Its first / last page buttons and the Find in Files folder button now have accessible names for screen readers.
- Find in Files messages follow the UI language (the results summary, skipped-file reasons and errors were always Traditional Chinese), and starting a new search stops the one still running.
- Clearing the Find in Files results no longer crashes while the results are on screen; the list, notes and counters reset after the frame completes and a new search works right away.
- Matches deep inside long lines (beyond the per-result character budget) show a short excerpt centred on the match with the match highlighted; such results used to crash and never showed the match. Clicking a result still selects the original columns.
- With the find bar open, moving the caret or using Find Next no longer rescans the whole document for the live count; the pattern and per-line counts are cached per document version and only invalidated by edits, so navigation stays instant in large files.

### 繁體中文

#### 新增
- 文字編輯器支援語法定義的粗體與斜體，包括 Markdown 標題與強調文字，且不改變游標、選取範圍及折行的字寬計算。
- 啟動時從設定檔旁的 `syntaxes/` 與 `themes/` 載入自訂 `.sublime-syntax` 定義和 `.tmTheme` 主題。自訂副檔名比對優先於近似語言回退，損壞的檔案也不會阻止其他資源載入。

#### 變更
- 將過長的代理交接與歷史紀錄精簡為經原始碼核對的協作指南，涵蓋安全的分支／PR 流程、現行架構、依變更範圍選擇的驗證方式及平台限制。
- 將 ADR 0002 的文字儲存與 mmap 設計標記為歷史方案，說明其部分已由 ADR 0011 取代，並連結至現行文字、十六進位及定位讀取實作。
- 將自繪文字編輯器拆分為版面計算、繪製與輸入模組，保留既有公開介面與畫面輸出。
- 對話框外觀與鍵盤操作更現代：控制項加高、底部按鈕加寬；文字欄位加上外框，在淺色主題的卡片上不再隱形；說明文字改為易讀的大小，清單篩選欄加上放大鏡圖示，空清單會顯示說明，快捷鍵以鍵帽樣式呈現。按 Esc 關閉對話框、按 Enter 執行主要動作；對話框開啟時輸入不再傳到後方的文件，關閉後焦點回到編輯區。
- 重新設計尋找列：輸入時即時計算符合數量（「3 / 12」、「無結果」或正規表達式錯誤），並在編輯區標示所有符合處，因此移除「計數」按鈕。區分大小寫、全字、正規表達式、十六進位及循環搜尋改為精簡的切換按鈕，歷史紀錄改為欄位旁的下拉選單，上一個／下一個／關閉改為圖示按鈕，提示中附快捷鍵。唯讀與十六進位文件中停用「取代」與「全部取代」。移至行列會顯示有效的行號範圍，輸入錯誤時以紅框標示。
- 「在檔案中尋找」結果面板依檔案分組，顯示符合數量與摺疊箭頭；「摺疊所有結果」命令可一次全部摺疊。每筆結果顯示行號，以及去除縮排、標示符合處的該行內容，長行會捲到符合處。取代時逐一列出已取代、略過或失敗的檔案（包括已開啟與唯讀的檔案），不再混入搜尋結果。搜尋進行中顯示進度圖示與「停止」按鈕。

#### 修正
- 插入或刪除行後，語法高亮仍與文字對齊；復原／重做及分幀解析期間的編輯也一併處理。
- 除錯版本以不配置記憶體的雜湊檢查偵測超出宣告行範圍的編輯，保護復原紀錄與增量快取，無須複製未修改的行。
- 全文編輯直接沿用復原快照中已變更的字串，避免重複複製。大型標準 Unicode 編輯會在建立快照前釋放原始位元組；無變更命令仍保留原始內容，解碼錯誤、傳統編碼或混合換行的資料也保留無損復原所需的原始位元組。
- 字元位置查詢在局部編輯及鄰近游標移動時沿用含版本的行首偏移錨點，不再每次按鍵都重掃文件前綴；復原／重做及全文重設會在必要時使錨點失效。
- 預覽列印視窗跟隨介面語言（標題、工具列與頁碼先前一律為繁體中文）；其第一頁／最後一頁按鈕及多檔案尋找的選擇資料夾按鈕提供螢幕閱讀器可讀的名稱。
- 「在檔案中尋找」的訊息跟隨介面語言（結果摘要、略過檔案的原因與錯誤訊息先前一律為繁體中文）；開始新的搜尋時會停止仍在執行的上一次搜尋。
- 結果顯示時按下「在檔案中尋找」的「清除」不再當機；清單、訊息與計數會在本幀繪製完成後統一清除，並可立即再次搜尋。
- 位於長行深處（超過每筆結果字元上限）的符合項目，現在顯示以符合處為中心的片段並標示符合文字；先前這類結果可能導致當機，也看不到符合內容。點擊結果仍會選取原始欄位。
- 尋找列開啟時，移動游標或「找下一個」不再為了即時計數重掃整份文件；搜尋樣式與逐行符合數會依文件版本快取，僅在編輯時失效，大型文件中的導航不再延遲。

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

[Unreleased]: https://github.com/tenyi/MadEdit/compare/v0.6.10...HEAD
[0.6.10]: https://github.com/tenyi/MadEdit/compare/v0.6.9...v0.6.10
[0.6.9]: https://github.com/tenyi/MadEdit/compare/v0.6.8...v0.6.9
[0.6.8]: https://github.com/tenyi/MadEdit/compare/v0.6.7...v0.6.8
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
