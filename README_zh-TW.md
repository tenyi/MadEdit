# MadEdit Rust 重製版

**以現代 Rust 與 egui 重新打造的跨平台、輕量、極速文字 / 列塊 / 十六進位編輯器**  

[English](README.md) | 繁體中文

---

## 📖 專案背景與願景

原版 C++ / wxWidgets 架構的 **MadEdit** 是許多開發者與資料處理人員心目中的經典工具（支援列塊編輯、Hex 檢視與多國語系編碼切換），但隨著時間推移，龐雜的 C++ 跨平台依賴與編譯鏈在現代 Windows、macOS 與 Linux 環境下日益難以維護。在下原本也由 MadEdit-Mod 的原始碼重新建置出 Mac 版本與 Linux 版本，但是速度慢，有許多功能在非Windows難以正常實現，尤其是字型顯示，修改後各平臺不一致，於是才使用 Rust 重製。

**MadEdit** 以 **Rust (Edition 2024)** 與即時模式 GUI 框架 **[egui / eframe](https://github.com/emilk/egui)** 重新打造，遵循 **KISS (Keep It Simple, Stupid)** 設計原則，在零 C++ GUI 系統庫依賴的前提下，重現原版 MadEdit 的經典介面與命令集（依原版命令表實作 165 個命令）。

> 介面曾以 Slint 實作，2026-10 改寫為 egui，以支援使用者任選系統字型（含比例字型）與更完整的輸入法處理，詳見 [ADR 0011](docs/adr/0011-egui-rewrite.md)。

---

## ✨ 核心功能特色

### 1. 三大編輯與檢視模式
- **文字模式 (Text Mode)**：一般串流文字與程式碼編輯，支援智慧縮排行首 (`Home`)、整行上下移動、複製行與刪除行。
- **列塊模式 (Column / Block Mode)**：
  - 以視覺欄位（半形 1 欄、CJK 全形 2 欄、Tab 對齊）進行二維矩形選取。
  - 支援工具列切換或按住 `Alt + 滑鼠拖曳` / `Alt + Shift + 方向鍵` 進行矩形選取。
  - 支援**矩形區塊複製、剪下、列塊對齊貼上、多行同步打字輸入與區塊切除**（短行自動補齊空白）。
- **十六進位模式 (Hex Mode)**：
  - 經典三欄對照檢視：**位址偏移 (Offset)**、**十六進位位元組 (Hex Bytes)**、**ASCII 可視預覽**。
  - 支援十六進位位移跳轉（如 `0x1A40`）與 Hex Byte Pattern 搜尋（如 `4D 5A 90`）。

### 2. 字型與輸入法
- 編輯區字型可任選系統字型與字級（含比例字型），CJK 字元自動以系統中日韓字型補字。使用比例字型時請關閉等寬模式（檢視 > 字型），選用比例字型時 MadEdit 也會詢問是否關閉。等寬字型的繪製會比比例字型快一點，因為比例字型要逐字繪製，才能與游標及選取範圍對齊。
- 支援 macOS 輸入法（注音、倉頡等）預編輯顯示與候選窗定位。
- 介面語言可跟隨系統，或在「檢視 → 介面語言」與選項中改為 English、繁體中文、简体中文、日本語、한국어、Deutsch、Español、Français、Italiano、Português。語言名稱一律用該語言自己的寫法。

### 3. 多語系編碼支援與防損毀保護
- 整合 `encoding_rs` 與自訂 UTF-16 編解碼器，支援 **UTF-8、UTF-8 BOM、UTF-16LE、UTF-16BE、Big5、GB18030、Shift_JIS、EUC-KR、windows-1252、ISO-8859-1** 等編碼。
- **智慧開檔偵測**：優先識別 BOM，無 BOM 時驗證嚴格 UTF-8，失敗則回退至可設定的備援編碼（預設 Big5）。新文件預設為 UTF-8。
- **無損防呆機制**：檔案含有無法無損解碼的位元組時，狀態列警告並拒絕覆寫存檔，提供**「重新解碼」**與**「轉換編碼」**。

### 4. 大檔案引擎
- **Piece Table** 以定位讀取（pread）分頁快取原始檔案，不使用 mmap（避免檔案被外部截短時發生 SIGBUS）。
- 超過門檻的大型檔案以十六進位模式開啟，資料留在磁碟上按需讀取。

### 5. Markdown 預覽（含 Mermaid 圖表）
- 從「檢視 → 預覽為 → 以 Markdown 預覽」或工具列預覽按鈕開啟，在 App 內直接繪製 Markdown（`egui_commonmark`）；目前檔案不是 Markdown 時按鈕停用。
- ` ```mermaid ` 區塊以 [**fermaid**](https://github.com/tenyi/fermaid) 轉成圖表顯示，不需瀏覽器或 Node.js。
  fermaid 是本專案作者為 Markdown 預覽特別開發的純 Rust Mermaid 繪製引擎，支援目前 Mermaid 的所有圖表類型，能快速產生預覽圖。
- LaTeX 數學式（行內 `$…$`、置中區塊 `$$…$$`）以同為純 Rust 的 [`latex-rust`](https://crates.io/crates/latex-rust) 繪製；最外層 `\text{…}` 裡的中文以預覽字型顯示，無法繪製的式子顯示 TeX 原始碼。
- 相對路徑圖片以文件所在資料夾為基準，支援 PNG、JPEG、GIF、WebP 與 SVG。
- 預設在主視窗內的**獨立分頁**開啟，也可切換**左右並排**或**上下並排**；關閉預覽分頁會保留來源文件。並排時與編輯區各佔一半，並可**同步捲動**跟隨編輯區。
- 支援縮放（`Ctrl` / `⌘` + 滾輪或觸控板捏合）與圖表符合寬度；高對比配色（深色純黑底、淺色純白底）。
- 預覽狀態隨分頁各自保存，切換分頁時同步切換。
- 另可「以 HTML 預覽」交給系統瀏覽器開啟。

### 6. 原版 MadEdit 命令集
- **單一執行個體**：再次啟動時將檔案交給既有視窗，並還原、喚回視窗。檔案以分頁開啟，重複開啟會切換到原分頁；未指定檔案時喚回既有視窗。不同 `MADEDIT_CONFIG_DIR` 目錄各自獨立。
- **檔案**：多文件分頁、另存副本、全部儲存、最近檔案、唯讀旗標、列印與 App 內預覽列印（A4 分頁、翻頁、縮放）、工作階段保存。
- **編輯**：書籤行操作、Hex 字串互轉、縮排、註解、大小寫、全半形、Tab 與空白互轉、遞增數字、排序、欄位對齊。
- **搜尋**：純文字 / 大小寫 / 全字 / Regex / Hex 搜尋取代、在多個檔案中尋找／取代、跳至對應括號。
- **檢視**：自動換行、行號、書籤欄、尺標、控制字元、目前行、欄位提示線、系統／淺色／深色主題、Markdown／HTML 預覽。
- **自訂語法與主題**：將 Sublime Text `.sublime-syntax` 定義放入 `settings.json` 旁的 `syntaxes/`，TextMate `.tmTheme` 檔案放入 `themes/`（「說明 > 關於」顯示設定路徑，也可用 `MADEDIT_CONFIG_DIR` 指定目錄）。支援子目錄，略過符號連結。安裝後請重新啟動 MadEdit。
  - 自訂定義的副檔名比對優先於內建語法及近似語言回退。可從「檢視 > 語法類型」選擇；主題以去除 `.tmTheme` 的檔名顯示於「語法高亮」對話框。同名資源會覆寫內建項目。
  - 損壞的檔案會個別略過，並將診斷訊息寫至 stderr，不影響有效資源與內建項目。
- **巨集與工具**：巨集錄製播放、快捷鍵重新指派與可篩選的快捷鍵一覽（說明 > 鍵盤快捷鍵，F1）、拼字檢查、字數統計、簡繁與日文漢字轉換、JSON／XML 格式化，以及以 dprint 為基礎的格式化（工具 > 文字轉換／格式化 >「dprint 格式化」），支援 C/C++/Objective-C（clang-format）、JavaScript/TypeScript、JSON/JSONC、CSS/SCSS、HTML/XML/SVG/Vue/Svelte、YAML、Markdown、TOML、GraphQL、SQL、Dockerfile、Shell、Python、Go、PHP、Lua、C#。

### 7. macOS 原生整合
- 支援 Finder「打開方式」與拖曳到 Dock 圖示開檔（含中文、空白、百分比檔名），執行中開檔會在同一個 App 新增分頁。
- 系統選單與 Dock 的「結束」也會先確認未儲存的修改。
- 快捷鍵依 macOS 慣例（⌘W、⌘Q、⌘⇧S、⌘⌥F、Control+Tab 等），⌘H、⌘M、⌘Tab 保留給系統。

---

## 🏗️ 專案目錄結構

```text
MadEdit/
├── Cargo.toml / Cargo.lock
├── Makefile                # 開發、打包與發行說明指令
├── ChangeLog.md            # 變更紀錄；GitHub Release 說明取自對應版本章節
├── build.rs                # Windows 執行檔圖示嵌入
├── .github/workflows/release.yml  # 標籤發行：Linux、AppImage、macOS DMG、Windows
├── assets/                 # 應用程式圖示、工具列圖示（取自原版）、拼字字典
├── scripts/
│   ├── make_appbundle.sh   # 產生 macOS MadEdit.app
│   ├── make_dmg.sh         # 產生 macOS DMG
│   ├── install_macos.sh    # 建置並安裝 MadEdit.app 到 /Applications
│   ├── test_macos_bundle.sh / test_macos_runtime.py  # macOS 打包與原生執行回歸測試
│   ├── make_linux_dist.sh  # 產生 Linux 發行壓縮檔（x86_64 / aarch64）
│   ├── make_appimage.sh    # 產生 Linux AppImage（x86_64 / aarch64）
│   ├── install_linux.sh    # 建置並安裝到本機 ~/.local
│   ├── make_windows_dist.ps1 # 產生 Windows 發行壓縮檔（x86_64 / aarch64）
│   └── changelog_notes.sh  # 從 ChangeLog.md 取出指定版本的發行說明
├── src/
│   ├── main.rs             # 進入點：解析命令列、建立視窗、啟動 eframe
│   ├── macos.rs            # macOS Finder／Dock 開檔與結束事件橋接
│   ├── app/                # egui 介面層（選單、工具列、自繪編輯區、Hex 檢視、Markdown 預覽、對話框、命令表）
│   └── core/               # 與 UI 無關的核心邏輯（文件、編碼、Piece Table、搜尋、語法、設定）
├── vendor/accesskit_macos/ # 回補 Touch Bar 退出崩潰修正的 AccessKit（見 vendor/README.md）
├── docs/                   # 架構決策紀錄 (ADR)、領域文件與 macOS 驗證紀錄
├── GLOSSARY.md             # 專案領域詞彙表
└── AGENTS.md               # AI 協作與開發指南
```

---

## 🚀 建置與執行指南

### 環境需求
- **Rust 工具鏈**：Rust `1.99` 或以上版本（Markdown 預覽繪製數學式所用的 latex-rust 的最低需求）
- 支援平台：**Windows**、**macOS**、**Linux**

### 開發與測試指令

```bash
make check             # cargo check
make test              # 單元測試與 egui_kittest 介面互動測試
make run               # 啟動開發版
make build             # 編譯最佳化正式版（cargo build --release --locked）
```

`make help` 會列出全部目標。`cargo check`、`cargo test`、`cargo run` 仍然可以直接使用。

核心／介面測試、經檢視的離屏截圖基準（`make test-visual`）、獨立執行的 1 GiB
回歸測試（`make test-large`），以及可重現的效能報告與基準比較（`make bench`），
請參閱[本機測試說明](docs/testing.md)。

編譯完成後，可於 `target/release/MadEdit`（Windows 為 `target/release/MadEdit.exe`）取得獨立執行檔。

### macOS 建置

macOS 上的 GUI 程式需放在 `.app` bundle 內，才能正確顯示 Dock 圖示、選單列名稱並與系統互動。

1. 安裝 Xcode Command Line Tools（提供 `sips`、`iconutil`、`codesign`）：
   ```bash
   xcode-select --install
   ```
2. 編譯並產生 App 與 DMG：
   ```bash
   make dist-macos
   ```
   等同於 `./scripts/make_appbundle.sh` 之後再執行 `./scripts/make_dmg.sh`。
3. 產物位於 `dist/MadEdit.app`（已含圖示與拼字字典，並以 ad-hoc 簽章）與 `dist/MadEdit-<版本>-macOS-<架構>.dmg`，可直接開啟或拖進「應用程式」資料夾：
   ```bash
   open dist/MadEdit.app
   ```

腳本會自動建置本機架構（Apple Silicon 或 Intel），支援 Cargo 自訂的 target-dir／build.target。
如已建好 universal binary，可用 `MADEDIT_BIN=/path/to/MadEdit ./scripts/make_appbundle.sh` 打包。
App 內含圖示與字典；打包、簽章及驗證全部完成後才取代舊版。

安裝到 `/Applications`：`./scripts/install_macos.sh`；也可用
`DEST="$HOME/Applications/MadEdit.app" ./scripts/install_macos.sh` 指定安裝位置。

支援 Finder「打開方式」、拖曳檔案至 Dock 圖示，以及
`open -a "$PWD/dist/MadEdit.app" 檔案.txt`。已啟動時會在同一個 App 開啟新分頁。
常用快捷鍵為 ⌘W 關閉分頁、⌘Q 結束（未儲存時先確認）、⌘⇧S 另存新檔、⌘⌥F 取代、
⌘/ 註解、Control+Tab 切換分頁；Option+方向鍵移動字詞、⌘+方向鍵移到行首尾／文件頭尾。
⌘H、⌘M、⌘Tab 保留給系統。

macOS 打包回歸測試：`./scripts/test_macos_bundle.sh`（不覆寫已安裝的 App）。
在已登入的 macOS 桌面執行 `python3 scripts/test_macos_runtime.py`，會連續啟動正式 App，
驗證文字／二進位文件繪製與正常退出，截圖與記錄留在 `dist/validation/runtime/`。
本次平台、測試結果與驗證範圍見 [macOS 移植驗證](docs/macos-validation.md)。
設定預設存於 `~/Library/Application Support/MadEdit/settings.json`；可用環境變數
`MADEDIT_CONFIG_DIR` 指定獨立設定目錄。`--screenshot 輸出.png` 使用預設設定並自動離開，
不讀寫日常工作階段，可供原生視窗驗證使用。

> 由於未經 Apple 公證，從網路下載的 App 第一次開啟時可能被 Gatekeeper 阻擋，
> 請在 Finder 中按右鍵選「打開」，或執行 `xattr -cr MadEdit.app` 移除隔離屬性。

### 下載發行檔

正式發行檔在 [Releases](https://github.com/tenyi/MadEdit/releases)。檔名裡的 `<版本>` 與 `Cargo.toml` 的 `version` 相同。

| 平台 | 檔名 |
| :--- | :--- |
| Linux x86_64 | `MadEdit-<版本>-linux-x86_64.tar.gz` |
| Linux ARM | `MadEdit-<版本>-linux-aarch64.tar.gz` |
| Linux AppImage x86_64 | `MadEdit-<版本>-linux-x86_64.AppImage` |
| Linux AppImage ARM | `MadEdit-<版本>-linux-aarch64.AppImage` |
| macOS Apple Silicon | `MadEdit-<版本>-macOS-arm64.dmg` |
| macOS Intel | `MadEdit-<版本>-macOS-x86_64.dmg` |
| Windows x64 | `MadEdit-<版本>-windows-x86_64.zip` |
| Windows ARM | `MadEdit-<版本>-windows-aarch64.zip` |

每個發行檔旁有同名 `.sha256`。Linux 壓縮檔解壓後可執行 `./MadEdit`，或 `./install.sh` 安裝到 `~/.local`。AppImage 加上執行權限後直接執行，需要系統上的 Vulkan 或 OpenGL 驅動。Windows 解壓後執行 `MadEdit.exe`。macOS 打開 DMG，把 `MadEdit.app` 拖進「應用程式」。

**每個平臺都有兩種版本。** 上表是「乾淨版」，不含任何格式化工具；每個檔案都有對應的 `-dprint` 版（例如 `MadEdit-<版本>-macOS-arm64-dprint.dmg`、`MadEdit-<版本>-linux-x86_64-dprint.AppImage`、`MadEdit-<版本>-windows-x86_64-dprint.zip`），額外附帶固定版本的 dprint 執行檔與 Wasm 插件：C/C++（clang-format）、JavaScript/TypeScript、JSON/JSONC、CSS/SCSS、HTML/XML/SVG/Vue/Svelte、YAML、Markdown、TOML、GraphQL、SQL、Dockerfile、Shell、Python、Go、PHP、Lua、C#，「工具 > dprint 格式化」離線即可使用。乾淨版在 PATH 上找得到 `dprint` 時同樣可用這個命令；不支援的副檔名會顯示訊息拒絕。自行打包時，在任何 `make dist-*` 加 `MADEDIT_WITH_FORMATTER=1`（Windows 腳本用 `-WithFormatter`）即會產生 `-dprint` 版；`make formatters` 會把固定版本的 dprint 下載到 `target/formatters`，打包腳本需要時也會自動執行。

### Windows 自行打包

在 Windows 上（x64 或 ARM 會依本機架構命名）：

```powershell
make dist-windows
```

或：

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\make_windows_dist.ps1
powershell -ExecutionPolicy Bypass -File .\scripts\make_windows_dist.ps1 -Arch aarch64
```

### Linux 自行打包

使用 [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild) 鎖定 glibc 2.28，讓同一個執行檔可在較舊的發行版（如 Ubuntu 20.04）上執行：

```bash
cargo install cargo-zigbuild
# 安裝 zig 0.16（可用系統套件管理員，或 uv tool install ziglang）
make dist-linux       # tar.gz，架構為本機
make dist-appimage    # AppImage，並一併產生 tar.gz
make dist-linux ARCH=aarch64
make dist-appimage ARCH=x86_64
```

`./scripts/make_linux_dist.sh [輸出目錄] [架構]` 與 `./scripts/make_appimage.sh` 是上面兩個目標的底層腳本。架構為 `x86_64` 或 `aarch64`。

### 發行步驟

變更紀錄寫在 [ChangeLog.md](ChangeLog.md)。推上版本標籤後，[`.github/workflows/release.yml`](.github/workflows/release.yml) 會建置上表的全部檔案，並把該版本章節原文設為 GitHub Release 說明。標籤含連字號（例如 `v0.6.0-rc.1`）時標成預發行。

1. 在 `ChangeLog.md` 把 `## [Unreleased]` 裡的條目移到新章節，標題格式為 `## [x.y.z] - YYYY-MM-DD`。章節不能是空的。
2. 把 `Cargo.toml` 的 `version` 改成同一個 `x.y.z`，並更新 `Cargo.lock`（`cargo check` 會改寫鎖定檔）。
3. 確認說明可以單獨取出（把 `0.6.0` 換成這次的版本）：
   ```bash
   make release-notes VERSION=0.6.0
   ```
4. 提交後打上標籤並推送：
   ```bash
   git tag v0.6.0
   git push origin main
   git push origin v0.6.0
   ```
   標籤版本必須與 `Cargo.toml` 相同（`v0.6.0` 對應 `0.6.0`）。不一致，或 ChangeLog 沒有該章節時，工作流程會在開始打包前停止。
5. Actions 完成後，Release 頁會出現 Linux（x64 / ARM 的 tar.gz 與 AppImage）、macOS DMG（Apple Silicon 與 Intel）與 Windows zip（x64 / ARM），每種都有乾淨版與 `-dprint` 版，說明文字就是 ChangeLog 的該章節。

只想試打包、不建立 Release 時，可在 GitHub 的 Actions 頁手動執行 Release workflow。產物會留在該次執行的 artifact，不會發佈。

---

## 🛠️ 核心技術棧

| 領域 | 套件 / 技術 | 用途 |
| :--- | :--- | :--- |
| **GUI 框架** | [`eframe` / `egui`](https://github.com/emilk/egui) (`v0.37`) | 即時模式 GUI、自繪編輯區與 Hex 檢視、輸入法事件 |
| **系統字型** | [`fontique`](https://crates.io/crates/fontique) | 列舉並載入系統字型 |
| **Markdown 預覽** | [`egui_commonmark`](https://crates.io/crates/egui_commonmark) | 在 App 內繪製 Markdown |
| **Mermaid 圖表** | [`fermaid`](https://github.com/tenyi/fermaid) + [`resvg`](https://crates.io/crates/resvg) | 自製純 Rust Mermaid 引擎，支援所有 Mermaid 圖表類型，轉為 SVG 並點陣化 |
| **數學式** | [`latex-rust`](https://crates.io/crates/latex-rust) | 純 Rust 的 LaTeX 數學式排版，輸出 SVG（內附 STIX Two Math 字型），與圖表一樣點陣化 |
| **macOS 整合** | [`objc2`](https://crates.io/crates/objc2) | Finder／Dock 開檔與結束事件 |
| **字元編碼** | [`encoding_rs`](https://crates.io/crates/encoding_rs) | 多語系字元編碼轉換與無損驗證 |
| **語法突顯** | [`syntect`](https://crates.io/crates/syntect) | Sublime Text 語法定義與主題 |
| **正規表達式** | [`regex`](https://crates.io/crates/regex) | 正規表達式搜尋與群組取代 |
| **拼字檢查** | [`spellbook`](https://crates.io/crates/spellbook) | Hunspell 字典拼字檢查 |
| **系統對話框** | [`rfd`](https://crates.io/crates/rfd) | 原生開啟舊檔與儲存檔案對話框 |
| **剪貼簿** | [`arboard`](https://crates.io/crates/arboard) | 跨平台系統剪貼簿 |

---

## 🌐 文件語言

- [English](README.md)（主要版本）
- [繁體中文](README_zh-TW.md)

`README.md` 是內容基準。它有變動時，須在同一次提交中同步更新所有翻譯版 README。

---

## 🙏 致謝

感謝原版 **MadEdit** 作者 Alston Chen，以及 **MadEdit-Mod** 維護者 Minggang Li 與所有貢獻者。本專案的圖示、工具列圖示、簡繁對照表與拼字字典皆取自原版，沒有他們多年的心血就沒有這個重製版。

---

## 📄 授權條款 (License)

本專案採用與原版開源精神一致之自由開源授權，歡迎提交 Issue 與 Pull Request！
