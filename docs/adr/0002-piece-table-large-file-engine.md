# 0002: 大檔案核心採用 Piece Table 搭配 memmap2 記憶體對映與按需行解碼

- Status: Partially superseded by [ADR 0011](0011-egui-rewrite.md). Text mode now uses a `Vec<String>` line array; the Piece Table and on-demand byte reads remain for hexadecimal editing and files above the text-size threshold.

## Architecture evolution

The proposal below records the historical Slint implementation, not the current text engine:

- [ADR 0011](0011-egui-rewrite.md) replaced ordinary text-mode storage with fully decoded lines in [`EditorState`](../../src/core/editor.rs). [`Document`](../../src/core/document.rs) owns encoding, text undo/redo, saving and the text/hex transition.
- [`HexBuffer`](../../src/core/hexbuf.rs) owns the [`PieceTable`](../../src/core/piece_table.rs) byte buffer. Files exceeding `OpenOptions::max_text_size` open in **hex-only mode**, without decoding the whole file; this is not a paged text-editing mode.
- File-backed pieces use cached positioned reads, not `memmap2`. This avoids `SIGBUS` if another process truncates the original file. The historical `LineIndex` and on-demand text-line decoding described below are no longer part of the current implementation.

## Historical proposal

為了解決原先將整個檔案載入記憶體並另存一份整檔 `String` 所導致的記憶體膨脹與開檔卡頓，我們決定採用 Piece Table 搭配 `memmap2` 唯讀記憶體對映與按需行解碼（On-demand Line Decoding）。

## Context

MadEdit 的核心特色之一是支援 GB 級超大檔案秒級開啟與隨機存取。若以傳統記憶體字串或單純 `Vec<String>` 實作，1GB 檔案會產生數千萬個小字串物件，使記憶體佔用達到檔案大小的 3 至 5 倍甚至引發 OOM。此外，文字模式與十六進位模式需要統一的高效位元組定址能力。

## Decision

1. **Piece Table 架構**：
   - 核心維護 **Original Buffer**（只讀原始檔案，大檔透過 `memmap2` 零拷貝對映）與 **Add Buffer**（儲存使用者所有輸入或插入的內容，僅向後追加）。
   - 文件內容由循序的 `Piece(source, offset, length)` 清單表示。文字與二進位編輯的插入與刪除僅需拆分、新增或移除 Piece 節點，時間複雜度極低且不複製原檔資料。
2. **LineIndex 與按需解碼**：
   - 掃描換行符建立輕量的 `LineIndex`（記錄每行的起始位元組位移）。
   - 僅當 UI 可視範圍（`Viewport`）請求繪製特定的 30～50 行時，才按需自 Piece Table 讀取該範圍的位元組並依當前編碼解碼為文字，徹底消除全檔解碼的記憶體負擔。
3. **儲存最佳化**：
   - 存檔時直接循序串流寫出各 Piece 對應的切片位元組，避免在記憶體中建立完整副本。

## Consequences

- 數 GB 級檔案可於數毫秒內瞬間開啟，記憶體佔用降至僅有索引與修改片段的極小空間（數 MB 級）。
- 編輯器文字檢視與十六進位檢視能共用同一套底層位元組隨機切片邏輯。
