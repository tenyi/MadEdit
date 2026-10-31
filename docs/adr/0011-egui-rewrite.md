# ADR 0011：介面全面改寫為 egui，補齊原版功能

- 狀態：已採納
- 日期：2026-10-23
- 取代：[ADR 0001](0001-slint-virtual-line-editor-view.md)、[ADR 0009](0009-madedit-classic-ui-and-focus-fix.md)、[ADR 0010](0010-classic-ui-consolidation.md) 中與 Slint 介面相關的部分
- 關聯：`docs/feature-gap-analysis.md`、`problem.md`
- Partially supersedes: [ADR 0002](0002-piece-table-large-file-engine.md), specifically shared text/hex storage, mmap and on-demand text-line decoding. Its updated architecture note identifies the current source modules.

## 背景

Slint 版在 macOS 上問題很多（輸入法、主題切換時好時壞、焦點遺失），
而且原版 MadEdit 可以讓使用者任選字型，連比例字型都支援。這需要逐字量測字寬，Slint 沒有公開這種 API。
`docs/feature-gap-analysis.md` 盤點的結果是：原版約 150 個命令，Slint 版只實作約兩成。
大部分功能還沒做，所以現在換框架的成本最低。

## 決策

1. **GUI 改用 eframe / egui 0.35**，刪除 `ui/*.slint` 與 Slint 相依。
   - 編輯區完全自繪（`src/app/view/`），逐字量測字寬。
     等寬模式用格網排版；比例字型模式改為累加各字的寬度。
   - 系統字型由 `fontique` 列舉，載入使用者選的字型；
     字型鏈依序為：使用者字型 → 系統 CJK 字型 → egui 內建字型。
   - 輸入法使用 egui 的 `Event::Ime`，預編輯文字畫在游標位置，候選窗位置透過 `IMEOutput` 回報。
2. **核心改為「行陣列為唯一真值」**（`core/document.rs`）。
   - 復原與重做以行範圍差異保存。
   - 檔案以 Piece Table 定位讀取（不使用 mmap，以避免檔案被截短時發生 SIGBUS）。
   - 超過門檻的檔案以十六進位模式開啟，資料留在磁碟上。
3. **命令表**（`src/app/commands.rs`）依原版 `CommandTable` 定義 165 個命令。
   - 選單、工具列、快捷鍵、巨集都透過同一個 `App::run(C)` 派送。
   - 快捷鍵可在選項對話框中重新指派。
4. **列印**改為產生 HTML，交給系統瀏覽器列印，支援頁首頁尾變數與語法色彩。
   **預覽列印**則在 App 內以獨立原生視窗（egui 多視窗 `show_viewport_immediate`）依 A4 紙張自行分頁繪製，
   可翻頁、縮放並直接按「列印」；HTML 與預覽共用同一份列印內容模型（`print::build_content`）。
5. **巨集**記錄為步驟清單（JSON）。原版是 Python 巨集，本版不內嵌直譯器。
6. **測試**：
   - 核心邏輯用單元測試。
   - UI 互動以 `egui_kittest` 模擬鍵盤與輸入法事件（`src/app/ui_tests.rs`）。
   - 無法開實體視窗的環境，用 `visual_tests.rs` 離屏繪製截圖。

## 取捨與未實作

- 單一執行個體、檔案關聯沒有實作。介面語言可跟隨系統或手動切換。
  檔案關聯請在作業系統中設定。
- RTL 只做鏡像排版，沒有真正的雙向文字（bidi）演算法。
- 語法配色只能選主題，不能逐項自訂顏色；粗體與斜體樣式不呈現。
- 搜尋以行為單位，正規表示式不能跨行。
- 原始碼格式化需要另外安裝 `astyle` 執行檔。

## 後果

- Editor behavior remains maintained in-project: caret, selection, wrapping, scrolling and IME.
  The original monolithic editor has since been split into `src/app/view/{mod,layout,paint,input}.rs`, retaining the existing public API.
- egui 每幀重繪，閒置時不耗 CPU（只在事件或游標閃爍時重繪）。
