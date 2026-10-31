//! 單一文件：檔案 I/O、編碼、文字內容（唯一真值）、復原／重做與十六進位模式

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use encoding_rs::Encoding;

use super::editor::{EditorState, Selection, TextPosition};
use super::encoding::{self, UnmappableChars};
use super::hexbuf::HexBuffer;
use super::model::{EditMode, NewLineFormat};
use super::piece_table::PieceTable;
use super::syntax::{self, SyntaxCache};

/// 編輯動作種類：連續的同類輸入會合併成一筆復原紀錄
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    Typing,
    Other,
}

/// 一筆復原紀錄：以「行範圍」記錄修改前後內容
struct UndoRecord {
    start: usize,
    old: Vec<String>,
    new: Vec<String>,
    sel_before: Selection,
    sel_after: Selection,
    bm_before: BTreeSet<usize>,
    bm_after: BTreeSet<usize>,
    kind: EditKind,
    serial_before: u64,
    serial_after: u64,
}

/// 開檔選項
#[derive(Debug, Clone)]
pub struct OpenOptions {
    pub default_encoding: &'static Encoding,
    /// 內容不是合法 UTF-8 時優先嘗試的編碼
    pub fallback_encoding: &'static Encoding,
    pub enforce_default_encoding: bool,
    /// 超過此大小（位元組）的檔案只以十六進位模式開啟（不整份解碼為文字）
    pub max_text_size: u64,
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self {
            default_encoding: encoding_rs::UTF_8,
            fallback_encoding: encoding_rs::BIG5,
            enforce_default_encoding: false,
            max_text_size: 100 * 1024 * 1024,
        }
    }
}

/// 存檔失敗原因
#[derive(Debug)]
pub enum SaveError {
    Io(std::io::Error),
    /// 尚未指定路徑
    NoPath,
    /// 文件為唯讀
    ReadOnly,
    /// 文件解碼時有錯誤（U+FFFD），直接存檔會遺失原始位元組
    DecodeErrors,
    /// 目標編碼無法表示部分字元
    Unmappable(UnmappableChars),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::Io(e) => write!(f, "寫入檔案失敗：{e}"),
            SaveError::NoPath => write!(f, "尚未指定檔案路徑"),
            SaveError::ReadOnly => write!(f, "文件為唯讀"),
            SaveError::DecodeErrors => write!(
                f,
                "檔案含有無法以目前編碼解碼的位元組，直接存檔會以 U+FFFD 取代而遺失原始資料"
            ),
            SaveError::Unmappable(u) => {
                let samples: String = u.samples.iter().collect();
                write!(f, "目前編碼無法表示 {} 個字元（例如「{}」）", u.count, samples)
            }
        }
    }
}

impl From<std::io::Error> for SaveError {
    fn from(e: std::io::Error) -> Self {
        SaveError::Io(e)
    }
}

static NEXT_DOC_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// 單一文件
pub struct Document {
    pub id: u64,
    pub path: Option<PathBuf>,
    pub encoding: &'static Encoding,
    pub has_bom: bool,
    /// 解碼時有錯誤（已以 U+FFFD 取代）
    pub has_decode_error: bool,
    pub newline: NewLineFormat,
    /// 載入時偵測到混用多種換行格式（存檔時會統一為 `newline`）
    pub mixed_newlines: bool,
    pub syntax: String,
    /// 文字內容、游標、選取與書籤（文字／列塊模式下的唯一真值）
    pub editor: EditorState,
    pub mode: EditMode,
    /// 十六進位模式的資料（僅在 Hex 模式或超大檔時存在）
    pub hex: Option<HexBuffer>,
    /// 超大檔只能以十六進位模式檢視編輯
    pub hex_only: bool,
    pub read_only: bool,
    /// 文字每次變動遞增，供 UI 判斷是否需要重算版面
    pub version: u64,
    pub syntax_cache: SyntaxCache,
    /// 跳轉歷史（上一個／下一個游標位置）
    caret_history: Vec<TextPosition>,
    caret_history_pos: usize,
    /// 「開始／結束選取」模式：開啟時移動游標會延伸選取
    pub selecting_mode: bool,
    undo: Vec<UndoRecord>,
    redo: Vec<UndoRecord>,
    serial: u64,
    next_serial: u64,
    saved_serial: u64,
    /// 非文字內容的修改（編碼、BOM、換行格式）
    meta_modified: bool,
    /// 載入時的原始位元組；文字被修改後即丟棄（供重新解碼與精確顯示十六進位）
    pristine_bytes: Option<Vec<u8>>,
    pub disk_mtime: Option<SystemTime>,
    pub disk_len: Option<u64>,
}

impl Default for Document {
    fn default() -> Self {
        Self::new_untitled(encoding_rs::UTF_8, false, NewLineFormat::default_for_platform(), syntax::PLAIN_TEXT)
    }
}

impl Document {
    /// 建立新的未命名文件
    pub fn new_untitled(encoding: &'static Encoding, has_bom: bool, newline: NewLineFormat, syntax: &str) -> Self {
        Self {
            id: NEXT_DOC_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            path: None,
            encoding,
            has_bom: has_bom && encoding::supports_bom(encoding),
            has_decode_error: false,
            newline,
            mixed_newlines: false,
            syntax: syntax.to_string(),
            editor: EditorState::default(),
            mode: EditMode::Text,
            hex: None,
            hex_only: false,
            read_only: false,
            version: 0,
            syntax_cache: SyntaxCache::default(),
            caret_history: Vec::new(),
            caret_history_pos: 0,
            selecting_mode: false,
            undo: Vec::new(),
            redo: Vec::new(),
            serial: 0,
            next_serial: 1,
            saved_serial: 0,
            meta_modified: false,
            pristine_bytes: None,
            disk_mtime: None,
            disk_len: None,
        }
    }

    /// 開啟檔案
    pub fn open(path: &Path, opts: &OpenOptions) -> std::io::Result<Self> {
        let meta = fs::metadata(path)?;
        let mut doc = Self::new_untitled(opts.default_encoding, false, NewLineFormat::default_for_platform(), syntax::PLAIN_TEXT);
        doc.path = Some(path.to_path_buf());
        doc.read_only = meta.permissions().readonly();
        doc.disk_mtime = meta.modified().ok();
        doc.disk_len = Some(meta.len());

        if meta.len() > opts.max_text_size {
            // 超大檔：以檔案定位讀取建立 Piece Table，只提供十六進位模式
            let pt = PieceTable::open_file(fs::File::open(path)?)?;
            doc.hex = Some(HexBuffer::new(pt));
            doc.hex_only = true;
            doc.mode = EditMode::Hex;
            doc.editor = EditorState::from_text("");
            return Ok(doc);
        }

        let bytes = fs::read(path)?;
        doc.load_bytes(bytes, opts);
        let first_line = doc.editor.lines.first().cloned().unwrap_or_default();
        doc.syntax = syntax::detect_syntax(path, &first_line);
        if encoding::looks_binary(doc.pristine_bytes.as_deref().unwrap_or(&[])) {
            // 二進位檔預設以十六進位模式開啟，避免文字解碼後存檔毀損
            let _ = doc.set_mode(EditMode::Hex);
        }
        Ok(doc)
    }

    /// 由位元組載入文字內容（偵測編碼與換行）
    fn load_bytes(&mut self, bytes: Vec<u8>, opts: &OpenOptions) {
        let decoded = encoding::detect_and_decode(&bytes, opts.default_encoding, opts.fallback_encoding, opts.enforce_default_encoding);
        self.apply_decoded(decoded);
        self.pristine_bytes = Some(bytes);
    }

    fn apply_decoded(&mut self, decoded: encoding::Decoded) {
        let nl = encoding::detect_newlines(&decoded.text);
        self.encoding = decoded.encoding;
        self.has_bom = decoded.has_bom;
        self.has_decode_error = decoded.had_errors;
        self.newline = nl.dominant.unwrap_or(self.newline);
        self.mixed_newlines = nl.mixed;
        let bookmarks = std::mem::take(&mut self.editor.bookmarks);
        let overwrite = self.editor.overwrite_mode;
        self.editor.set_text(&decoded.text);
        self.editor.bookmarks = bookmarks.into_iter().filter(|&l| l < self.editor.lines.len()).collect();
        self.editor.overwrite_mode = overwrite;
        self.version += 1;
        self.syntax_cache.clear();
    }

    /// 目前語法為 Markdown（決定能否開啟 Markdown 預覽）
    pub fn is_markdown(&self) -> bool {
        self.syntax == "Markdown"
    }

    /// 顯示用名稱（檔名或「未命名」）
    pub fn display_name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("未命名-{}", self.id))
    }

    /// 是否有未存檔修改
    pub fn is_modified(&self) -> bool {
        if self.hex_only {
            return self.hex.as_ref().is_some_and(|h| h.is_modified());
        }
        let hex_dirty = self.mode == EditMode::Hex && self.hex.as_ref().is_some_and(|h| h.is_modified());
        self.serial != self.saved_serial || self.meta_modified || hex_dirty
    }

    /// Notify incremental caches of a replacement in pre-edit line coordinates.
    fn text_changed(&mut self, line: usize, removed: usize, inserted: usize) {
        self.version += 1;
        self.syntax_cache.lines_replaced(line, removed, inserted);
    }

    // ── 編輯與復原 ──────────────────────────────────────────────

    /// 以選取範圍推算編輯可能影響的行範圍（含前後各一行，涵蓋合併行的情況）
    pub fn selection_hint(&self) -> Range<usize> {
        let sel = &self.editor.selection;
        let a = sel.anchor.line.min(sel.head.line);
        let b = sel.anchor.line.max(sel.head.line);
        a.saturating_sub(1)..(b + 2).min(self.editor.lines.len())
    }

    /// 整份文件範圍
    pub fn whole_hint(&self) -> Range<usize> {
        0..self.editor.lines.len()
    }

    /// 自選取起點到文件結尾（用於可能在結尾新增行的操作，如列塊貼上）
    pub fn to_end_hint(&self) -> Range<usize> {
        self.selection_hint().start..self.editor.lines.len()
    }

    /// 執行一次可復原的文字編輯
    ///
    /// `hint` 必須涵蓋 `f` 可能修改的所有行（修改前的行號）；`f` 只能在這個範圍內
    /// 修改、插入或刪除行。回傳內容或選取是否有變動。
    pub fn edit<R>(&mut self, kind: EditKind, hint: Range<usize>, f: impl FnOnce(&mut EditorState) -> R) -> R {
        let len_before = self.editor.lines.len();
        let a = hint.start.min(len_before);
        let b = hint.end.clamp(a, len_before);
        let old: Vec<String> = self.editor.lines[a..b].to_vec();
        let sel_before = self.editor.selection;
        let bm_before = self.editor.bookmarks.clone();

        let result = f(&mut self.editor);

        if self.editor.lines.is_empty() {
            self.editor.lines.push(String::new());
        }
        let len_after = self.editor.lines.len();
        let new_end = (b as isize + len_after as isize - len_before as isize).max(a as isize) as usize;
        let new_end = new_end.min(len_after);
        let new_region = &self.editor.lines[a..new_end];

        // 修剪前後相同的行，只保存真正變動的部分
        let prefix = old.iter().zip(new_region).take_while(|(x, y)| x == y).count();
        let max_suffix = old.len().min(new_region.len()) - prefix;
        let suffix = old
            .iter()
            .rev()
            .zip(new_region.iter().rev())
            .take(max_suffix)
            .take_while(|(x, y)| x == y)
            .count();
        let old_changed = old[prefix..old.len() - suffix].to_vec();
        let new_changed = new_region[prefix..new_region.len() - suffix].to_vec();

        if old_changed.is_empty() && new_changed.is_empty() {
            // 內容未變（可能只是選取改變），不產生復原紀錄
            return result;
        }

        let start = a + prefix;
        self.pristine_bytes = None;
        self.text_changed(start, old_changed.len(), new_changed.len());

        // 連續輸入合併為一筆；已存檔的紀錄不再合併，否則 serial 不前進，存檔後的輸入不會被視為修改
        let mergeable = kind == EditKind::Typing
            && old_changed.len() == 1
            && new_changed.len() == 1
            && self.redo.is_empty()
            && self.serial != self.saved_serial
            && self.undo.last().is_some_and(|last| {
                last.kind == EditKind::Typing
                    && last.start == start
                    && last.new.len() == 1
                    && last.old.len() == 1
                    && last.sel_after == sel_before
                    && last.serial_after == self.serial
            });
        if mergeable {
            let last = self.undo.last_mut().unwrap();
            last.new = new_changed;
            last.sel_after = self.editor.selection;
            last.bm_after = self.editor.bookmarks.clone();
            return result;
        }

        let serial_after = self.next_serial;
        self.next_serial += 1;
        self.undo.push(UndoRecord {
            start,
            old: old_changed,
            new: new_changed,
            sel_before,
            sel_after: self.editor.selection,
            bm_before,
            bm_after: self.editor.bookmarks.clone(),
            kind,
            serial_before: self.serial,
            serial_after,
        });
        self.serial = serial_after;
        self.redo.clear();
        // 復原紀錄過多時丟棄最舊的（保留最近 10000 筆）
        if self.undo.len() > 10_000 {
            self.undo.drain(..1000);
        }
        result
    }

    pub fn can_undo(&self) -> bool {
        if self.mode == EditMode::Hex {
            return self.hex.as_ref().is_some_and(|h| h.can_undo());
        }
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        if self.mode == EditMode::Hex {
            return self.hex.as_ref().is_some_and(|h| h.can_redo());
        }
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        if self.mode == EditMode::Hex {
            return self.hex.as_mut().is_some_and(|h| h.undo());
        }
        let Some(rec) = self.undo.pop() else { return false };
        let end = rec.start + rec.new.len();
        self.editor.lines.splice(rec.start..end, rec.old.iter().cloned());
        self.editor.selection = rec.sel_before;
        self.editor.bookmarks = carry_bookmarks(&self.editor.bookmarks, &rec.bm_after, &rec.bm_before, rec.start, rec.new.len(), rec.old.len());
        self.editor.bookmarks.retain(|&l| l < self.editor.lines.len());
        self.editor.clamp_caret();
        self.serial = rec.serial_before;
        self.text_changed(rec.start, rec.new.len(), rec.old.len());
        self.redo.push(rec);
        true
    }

    pub fn redo(&mut self) -> bool {
        if self.mode == EditMode::Hex {
            return self.hex.as_mut().is_some_and(|h| h.redo());
        }
        let Some(rec) = self.redo.pop() else { return false };
        let end = rec.start + rec.old.len();
        self.editor.lines.splice(rec.start..end, rec.new.iter().cloned());
        self.editor.selection = rec.sel_after;
        self.editor.bookmarks = carry_bookmarks(&self.editor.bookmarks, &rec.bm_before, &rec.bm_after, rec.start, rec.old.len(), rec.new.len());
        self.editor.bookmarks.retain(|&l| l < self.editor.lines.len());
        self.editor.clamp_caret();
        self.serial = rec.serial_after;
        self.text_changed(rec.start, rec.old.len(), rec.new.len());
        self.undo.push(rec);
        true
    }

    /// 以新內容取代整份文件（可復原）
    pub fn replace_all_text(&mut self, text: &str) {
        let hint = self.whole_hint();
        let caret = self.editor.selection.head;
        self.edit(EditKind::Other, hint, |ed| {
            let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
            ed.lines = normalized.split('\n').map(str::to_string).collect();
            ed.set_caret(caret.line, caret.col);
        });
    }

    /// 文件全文（以文件換行格式串接）
    pub fn text(&self) -> String {
        self.editor.lines.join(self.newline.newline_str())
    }

    // ── 編碼、換行與 BOM ────────────────────────────────────────

    /// 依目前編碼產生要寫入的位元組
    pub fn encode(&self) -> Result<Vec<u8>, UnmappableChars> {
        encoding::encode_text(&self.text(), self.encoding, self.has_bom)
    }

    /// 重新解碼：以指定編碼重新解讀檔案的位元組（不視為修改）
    ///
    /// 文件未修改時使用載入時的原始位元組，因此即使原本解碼有錯也能無損改用正確編碼。
    pub fn reinterpret(&mut self, enc: &'static Encoding) -> Result<(), String> {
        let bytes = match &self.pristine_bytes {
            Some(b) => b.clone(),
            None => {
                if self.has_decode_error {
                    return Err("文件已修改且原本解碼有誤，無法取得原始位元組；請先重新載入檔案".into());
                }
                self.encode().map_err(|u| SaveError::Unmappable(u).to_string())?
            }
        };
        let decoded = encoding::decode_with(&bytes, enc);
        let caret = self.editor.selection.head;
        let modified_text = self.serial != self.saved_serial;
        self.apply_decoded(decoded);
        self.editor.set_caret(caret.line, caret.col);
        // 重新解碼後內容已不同，原本的復原紀錄不再適用
        self.undo.clear();
        self.redo.clear();
        if !modified_text {
            self.saved_serial = self.serial;
        }
        self.pristine_bytes = Some(bytes);
        Ok(())
    }

    /// 轉換編碼：保留文字內容，存檔時改用新編碼（視為修改）
    pub fn convert_encoding(&mut self, enc: &'static Encoding, with_bom: bool) -> Result<(), String> {
        if self.has_decode_error {
            return Err(
                "文件含有無法解碼的位元組（顯示為 �），直接轉換會永久遺失原始資料；請先以正確的編碼「重新解碼」".into(),
            );
        }
        // 先確認新編碼能表示所有字元
        encoding::encode_text(&self.text(), enc, false).map_err(|u| SaveError::Unmappable(u).to_string())?;
        self.encoding = enc;
        self.has_bom = with_bom && encoding::supports_bom(enc);
        self.meta_modified = true;
        self.pristine_bytes = None;
        Ok(())
    }

    /// 加入或移除 BOM
    pub fn set_bom(&mut self, has_bom: bool) {
        let has_bom = has_bom && encoding::supports_bom(self.encoding);
        if has_bom != self.has_bom {
            self.has_bom = has_bom;
            self.meta_modified = true;
            self.pristine_bytes = None;
        }
    }

    /// 轉換換行格式
    pub fn convert_newline(&mut self, fmt: NewLineFormat) {
        if self.newline != fmt || self.mixed_newlines {
            self.newline = fmt;
            self.mixed_newlines = false;
            self.meta_modified = true;
            self.pristine_bytes = None;
        }
    }

    // ── 模式切換 ────────────────────────────────────────────────

    /// 切換編輯模式；離開十六進位模式時，若位元組有修改則解碼回文字
    pub fn set_mode(&mut self, mode: EditMode) -> Result<(), String> {
        if self.hex_only && mode != EditMode::Hex {
            return Err("檔案過大，只能以十六進位模式開啟".into());
        }
        if mode == self.mode {
            return Ok(());
        }
        if mode == EditMode::Hex {
            let bytes = match &self.pristine_bytes {
                Some(b) if self.serial == self.saved_serial && !self.meta_modified => b.clone(),
                _ => self.encode().map_err(|u| SaveError::Unmappable(u).to_string())?,
            };
            let mut hb = HexBuffer::new(PieceTable::from_bytes(bytes));
            // 游標換算為位元組位置
            let head = self.editor.selection.head;
            hb.caret = self.byte_offset_of(head).min(hb.len());
            hb.top_row = hb.caret / super::hexbuf::BYTES_PER_ROW;
            self.hex = Some(hb);
        } else if self.mode == EditMode::Hex {
            let hb = self.hex.take().expect("Hex 模式必有位元組緩衝");
            if hb.has_edits() {
                let bytes = hb.data.collect_bytes();
                let decoded = encoding::decode_with(&bytes, self.encoding);
                let caret = self.editor.selection.head;
                let text = decoded.text.clone();
                self.has_decode_error = decoded.had_errors;
                self.has_bom = decoded.has_bom;
                self.replace_all_text(&text);
                self.editor.set_caret(caret.line, caret.col);
                if !hb.is_modified() {
                    // 位元組改了又全部復原：文字與存檔時相同
                    self.saved_serial = self.serial;
                }
                self.pristine_bytes = Some(bytes);
            }
        }
        self.mode = mode;
        Ok(())
    }

    /// 游標位置換算為（含 BOM 的）位元組位移
    pub fn byte_offset_of(&self, pos: TextPosition) -> usize {
        let nl_len = encoding::encoded_len(self.newline.newline_str(), self.encoding);
        let mut offset = if self.has_bom { encoding::bom_bytes(self.encoding).len() } else { 0 };
        for line in self.editor.lines.iter().take(pos.line) {
            offset += encoding::encoded_len(line, self.encoding) + nl_len;
        }
        if let Some(line) = self.editor.lines.get(pos.line) {
            let prefix: String = line.chars().take(pos.col).collect();
            offset += encoding::encoded_len(&prefix, self.encoding);
        }
        offset
    }

    // ── 存檔、重新載入與外部修改 ────────────────────────────────

    /// 存檔；`allow_lossy` 為真時即使有解碼錯誤也照存（使用者已確認）
    pub fn save(&mut self, path: Option<&Path>, allow_lossy: bool) -> Result<(), SaveError> {
        let target = match path {
            Some(p) => p.to_path_buf(),
            None => self.path.clone().ok_or(SaveError::NoPath)?,
        };
        let same_file = self.path.as_deref() == Some(target.as_path());
        if self.read_only && same_file {
            return Err(SaveError::ReadOnly);
        }
        let in_hex = self.mode == EditMode::Hex;
        if !in_hex && self.has_decode_error && !allow_lossy {
            return Err(SaveError::DecodeErrors);
        }

        self.write_atomic(&target)?;

        if self.hex_only {
            // 改由新檔案提供資料（舊的檔案控制代碼指向已被取代的檔案）
            let pt = PieceTable::open_file(fs::File::open(&target)?)?;
            let mut hb = HexBuffer::new(pt);
            if let Some(old) = &self.hex {
                hb.caret = old.caret.min(hb.len());
                hb.top_row = old.top_row;
            }
            self.hex = Some(hb);
        } else if in_hex
            && let Some(hb) = self.hex.as_mut() {
                hb.mark_saved();
            }
        if !self.hex_only && !in_hex {
            self.pristine_bytes = None;
        }
        self.path = Some(target.clone());
        self.saved_serial = self.serial;
        self.meta_modified = false;
        self.mixed_newlines = false;
        if !in_hex {
            self.has_decode_error = false;
        }
        self.refresh_disk_stamp();
        if !same_file {
            let first_line = self.editor.lines.first().cloned().unwrap_or_default();
            if self.syntax == syntax::PLAIN_TEXT {
                self.syntax = syntax::detect_syntax(&target, &first_line);
            }
        }
        Ok(())
    }

    /// 另存一份副本（不改變文件路徑與修改狀態）
    pub fn save_copy(&self, target: &Path) -> Result<(), SaveError> {
        if self.mode != EditMode::Hex && self.has_decode_error {
            return Err(SaveError::DecodeErrors);
        }
        self.write_atomic(target)
    }

    /// 將目前內容寫入同目錄暫存檔，成功後原子取代目標檔
    ///
    /// 編碼或寫入失敗時目標檔不受影響（不會先被截斷）。
    fn write_atomic(&self, target: &Path) -> Result<(), SaveError> {
        atomic_write(target, |f| {
            if self.mode == EditMode::Hex {
                self.hex.as_ref().expect("Hex 模式必有位元組緩衝").data.write_to(f)?;
            } else {
                let bytes = self.encode().map_err(SaveError::Unmappable)?;
                f.write_all(&bytes)?;
            }
            Ok(())
        })
    }

    fn refresh_disk_stamp(&mut self) {
        if let Some(meta) = self.path.as_ref().and_then(|p| fs::metadata(p).ok()) {
            self.disk_mtime = meta.modified().ok();
            self.disk_len = Some(meta.len());
            self.read_only = meta.permissions().readonly();
        }
    }

    /// 磁碟上的檔案是否在本程式之外被修改（時間戳或大小改變）
    pub fn changed_on_disk(&self) -> bool {
        let Some(path) = &self.path else { return false };
        let Ok(meta) = fs::metadata(path) else {
            return false;
        };
        meta.modified().ok() != self.disk_mtime || Some(meta.len()) != self.disk_len
    }

    /// 忽略外部修改（之後不再提示，直到檔案再次被修改）
    pub fn ignore_disk_change(&mut self) {
        let keep_ro = self.read_only;
        self.refresh_disk_stamp();
        self.read_only = keep_ro;
    }

    /// 從磁碟重新載入（保留游標、書籤與編輯模式）
    pub fn reload(&mut self, opts: &OpenOptions) -> std::io::Result<()> {
        let Some(path) = self.path.clone() else {
            return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "文件沒有對應的檔案"));
        };
        let mut fresh = Document::open(&path, opts)?;
        if !fresh.hex_only && self.mode != EditMode::Hex {
            // 沿用目前編碼（使用者可能已手動重新解碼）
            if let Some(bytes) = fresh.pristine_bytes.clone() {
                let decoded = encoding::decode_with(&bytes, self.encoding);
                fresh.apply_decoded(decoded);
                fresh.pristine_bytes = Some(bytes);
            }
            let _ = fresh.set_mode(self.mode);
        }
        let caret = self.editor.selection.head;
        fresh.editor.bookmarks = self
            .editor
            .bookmarks
            .iter()
            .copied()
            .filter(|&l| l < fresh.editor.lines.len())
            .collect();
        fresh.editor.set_caret(caret.line, caret.col);
        fresh.syntax = self.syntax.clone();
        fresh.id = self.id;
        *self = fresh;
        self.version += 1;
        Ok(())
    }

    /// 切換磁碟檔案的唯讀屬性
    pub fn toggle_read_only(&mut self) -> std::io::Result<()> {
        if let Some(path) = &self.path {
            let mut perms = fs::metadata(path)?.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(!self.read_only);
            fs::set_permissions(path, perms)?;
            self.refresh_disk_stamp();
        } else {
            self.read_only = !self.read_only;
        }
        Ok(())
    }

    // ── 游標跳轉歷史 ────────────────────────────────────────────

    /// 記錄一次較大的游標跳轉（搜尋、跳行、書籤等），供「上一個／下一個位置」使用
    pub fn record_jump(&mut self, from: TextPosition) {
        self.caret_history.truncate(self.caret_history_pos);
        if self.caret_history.last() != Some(&from) {
            self.caret_history.push(from);
        }
        if self.caret_history.len() > 200 {
            self.caret_history.remove(0);
        }
        self.caret_history_pos = self.caret_history.len();
    }

    pub fn go_back(&mut self) -> bool {
        if self.caret_history_pos == 0 {
            return false;
        }
        let here = self.editor.selection.head;
        if self.caret_history_pos == self.caret_history.len() {
            self.caret_history.push(here);
        }
        self.caret_history_pos -= 1;
        let p = self.caret_history[self.caret_history_pos];
        self.editor.set_caret(p.line, p.col);
        true
    }

    pub fn go_forward(&mut self) -> bool {
        if self.caret_history_pos + 1 >= self.caret_history.len() {
            return false;
        }
        self.caret_history_pos += 1;
        let p = self.caret_history[self.caret_history_pos];
        self.editor.set_caret(p.line, p.col);
        true
    }
}

impl NewLineFormat {
    /// 作業系統慣用的換行格式
    pub fn default_for_platform() -> Self {
        if cfg!(windows) { NewLineFormat::Dos } else { NewLineFormat::Unix }
    }
}

/// 以 `write` 寫入同目錄暫存檔並 `sync_all`，成功後保留原檔權限並原子取代 `target`
///
/// 寫入失敗或行程在寫入途中中止時，`target` 維持原狀（不會先被截斷）。
pub fn atomic_write<E: From<std::io::Error>>(target: &Path, write: impl FnOnce(&mut fs::File) -> Result<(), E>) -> Result<(), E> {
    // 暫存檔放在同目錄，檔名後附加，不取代原副檔名
    let file_name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = target.with_file_name(format!(".{}.madedit-{}.tmp", file_name, std::process::id()));
    let write_result: Result<(), E> = (|| {
        let mut f = fs::File::create(&tmp)?;
        write(&mut f)?;
        f.sync_all()?;
        Ok(())
    })();
    if let Err(e) = write_result {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    // 保留原檔權限
    if let Ok(meta) = fs::metadata(target) {
        let _ = fs::set_permissions(&tmp, meta.permissions());
    }
    // 原子取代：失敗時原檔不受影響
    if let Err(e) = fs::rename(&tmp, target) {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

/// 復原／重做時還原書籤快照，並保留編輯之後使用者自行做的書籤增刪
///
/// `current` 為目前書籤，`expected` 為未經使用者變動時應有的書籤（與 `current` 同一座標），
/// `target` 為要還原的快照。該次編輯把第 `start` 行起的 `from_len` 行換成 `to_len` 行，
/// 使用者的增刪依此換算到 `target` 的行號座標後套用。
fn carry_bookmarks(
    current: &BTreeSet<usize>,
    expected: &BTreeSet<usize>,
    target: &BTreeSet<usize>,
    start: usize,
    from_len: usize,
    to_len: usize,
) -> BTreeSet<usize> {
    let map = |l: usize| {
        if l < start {
            l
        } else if l >= start + from_len {
            l - from_len + to_len
        } else {
            // 位於被取代範圍內：落到對應範圍內最接近的一行
            start + (l - start).min(to_len.saturating_sub(1))
        }
    };
    let mut out = target.clone();
    for &l in current.difference(expected) {
        out.insert(map(l));
    }
    for &l in expected.difference(current) {
        out.remove(&map(l));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("madedit_doc_{}_{}", std::process::id(), name))
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "edit modified lines before hint")]
    fn test_edit_rejects_changes_before_hint() {
        let mut doc = Document::default();
        doc.editor.set_text("prefix\neditable\nsuffix");
        doc.edit(EditKind::Other, 1..2, |ed| ed.lines[0].push('!'));
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "edit modified lines after hint")]
    fn test_edit_rejects_changes_after_shifted_hint() {
        let mut doc = Document::default();
        doc.editor.set_text("prefix\neditable\nsuffix");
        doc.edit(EditKind::Other, 1..2, |ed| {
            ed.lines.insert(1, "inserted".into());
            ed.lines[3].push('!');
        });
    }

    #[test]
    fn test_edit_contract_allows_empty_document_and_boundary_insertions() {
        let mut doc = Document::default();
        doc.editor.set_text("a\nb\nc");
        doc.edit(EditKind::Other, 1..1, |ed| ed.lines.insert(1, "middle".into()));
        doc.edit(EditKind::Other, 4..4, |ed| ed.lines.push("end".into()));
        assert_eq!(doc.text(), "a\nmiddle\nb\nc\nend");
        doc.edit(EditKind::Other, 0..5, |ed| ed.lines.clear());
        assert_eq!(doc.text(), "");
        assert!(doc.undo());
        assert_eq!(doc.text(), "a\nmiddle\nb\nc\nend");
    }

    #[test]
    fn test_undo_redo_and_typing_merge() {
        let mut doc = Document::default();
        for c in "abc".chars() {
            let hint = doc.selection_hint();
            doc.edit(EditKind::Typing, hint, |e| e.insert_char(c));
        }
        assert_eq!(doc.editor.lines, vec!["abc"]);
        assert!(doc.is_modified());
        // 連續輸入合併為一筆
        assert!(doc.undo());
        assert_eq!(doc.editor.lines, vec![""]);
        assert!(!doc.is_modified());
        assert!(doc.redo());
        assert_eq!(doc.editor.lines, vec!["abc"]);

        let hint = doc.selection_hint();
        doc.edit(EditKind::Other, hint, |e| e.insert_newline());
        assert_eq!(doc.editor.lines, vec!["abc", ""]);
        doc.undo();
        assert_eq!(doc.editor.lines, vec!["abc"]);
        assert_eq!(doc.editor.selection.head, TextPosition::new(0, 3));
    }

    #[test]
    fn test_undo_multiline_delete_restores_bookmarks() {
        let mut doc = Document::default();
        doc.replace_all_text("1\n2\n3\n4");
        doc.editor.bookmarks.insert(2);
        doc.editor.selection = Selection::new(TextPosition::new(0, 1), TextPosition::new(2, 1));
        let hint = doc.selection_hint();
        doc.edit(EditKind::Other, hint, |e| e.delete_selection());
        assert_eq!(doc.editor.lines, vec!["1", "4"]);
        doc.undo();
        assert_eq!(doc.editor.lines, vec!["1", "2", "3", "4"]);
        assert!(doc.editor.bookmarks.contains(&2));
    }

    #[test]
    fn test_open_save_roundtrip_utf16_crlf() {
        let p = temp_path("u16.txt");
        let content: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain("a\r\nb".encode_utf16().flat_map(|u| u.to_le_bytes()))
            .collect();
        fs::write(&p, &content).unwrap();
        let mut doc = Document::open(&p, &OpenOptions::default()).unwrap();
        assert_eq!(doc.encoding, encoding_rs::UTF_16LE);
        assert_eq!(doc.newline, NewLineFormat::Dos);
        assert_eq!(doc.editor.lines, vec!["a", "b"]);
        doc.save(None, false).unwrap();
        assert_eq!(fs::read(&p).unwrap(), content);
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn test_encoding_bom_newline_roundtrip_matrix() {
        let dir = crate::test_support::TempDir::new("encoding-matrix");
        let p = dir.path().join("roundtrip.txt");
        let mut cases = 0;
        for (enc, supports_bom) in [
            (encoding_rs::UTF_8, true),
            (encoding_rs::UTF_16LE, true),
            (encoding_rs::UTF_16BE, true),
            (encoding_rs::BIG5, false),
            (encoding_rs::GB18030, false),
            (encoding_rs::SHIFT_JIS, false),
            (encoding_rs::WINDOWS_1252, false),
        ] {
            // The three East Asian encodings share these characters; Windows-1252 has its own fixture.
            let lines = if enc == encoding_rs::WINDOWS_1252 {
                ["café", "naïve", ""]
            } else {
                ["中文", "日本語", ""]
            };
            for bom in [false, true] {
                if bom && !supports_bom {
                    continue;
                }
                // Build the byte oracle independently of MadEdit's encode_text wrapper.
                let fixture_bytes = |text: &str| {
                    let mut bytes = match (enc, bom) {
                        (e, true) if e == encoding_rs::UTF_8 => vec![0xEF, 0xBB, 0xBF],
                        (e, true) if e == encoding_rs::UTF_16LE => vec![0xFF, 0xFE],
                        (e, true) if e == encoding_rs::UTF_16BE => vec![0xFE, 0xFF],
                        _ => Vec::new(),
                    };
                    if enc == encoding_rs::UTF_16LE || enc == encoding_rs::UTF_16BE {
                        for unit in text.encode_utf16() {
                            bytes.extend(if enc == encoding_rs::UTF_16LE {
                                unit.to_le_bytes()
                            } else {
                                unit.to_be_bytes()
                            });
                        }
                    } else {
                        let (body, _, errors) = enc.encode(text);
                        assert!(!errors);
                        bytes.extend_from_slice(&body);
                    }
                    bytes
                };
                for newline in [NewLineFormat::Unix, NewLineFormat::Dos, NewLineFormat::Mac] {
                    cases += 1;
                    let text = lines.join(newline.newline_str());
                    let original = fixture_bytes(&text);
                    fs::write(&p, &original).unwrap();
                    let opts = OpenOptions {
                        default_encoding: enc,
                        enforce_default_encoding: true,
                        ..Default::default()
                    };
                    let mut doc = Document::open(&p, &opts).unwrap();
                    // BOM-less UTF-16 can initially be classified as binary; this matrix edits text.
                    doc.set_mode(EditMode::Text).unwrap();
                    let case = format!("{}, BOM={bom}, {newline:?}", enc.name());
                    assert_eq!(doc.encoding, enc, "{case}");
                    assert_eq!(doc.has_bom, bom, "{case}");
                    assert_eq!(doc.newline, newline, "{case}");
                    assert!(!doc.has_decode_error, "{case}");
                    assert_eq!(doc.editor.lines, lines, "{case}");
                    // Editing invalidates pristine_bytes, exercising the encoder on save.
                    doc.editor.set_caret(1, 0);
                    let hint = doc.selection_hint();
                    doc.edit(EditKind::Other, hint, |e| e.insert_text("A"));
                    doc.save(None, false).unwrap();
                    let changed = [
                        lines[0].to_string(),
                        format!("A{}", lines[1]),
                        String::new(),
                    ]
                    .join(newline.newline_str());
                    assert_eq!(fs::read(&p).unwrap(), fixture_bytes(&changed), "{case}");
                    assert!(doc.undo(), "{case}");
                    doc.save(None, false).unwrap();
                    assert_eq!(fs::read(&p).unwrap(), original, "{case}");
                }
            }
        }
        assert_eq!(cases, 30);
    }

    #[test]
    fn test_generated_edits_undo_redo_match_snapshots() {
        use crate::test_support::Rng;
        for seed in [1, 42, 0x2026_1028] {
            let mut rng = Rng(seed);
            let mut doc = Document::default();
            let mut expected = Vec::<char>::new();
            let mut history = Vec::new();
            let position = |chars: &[char]| {
                let line = chars.iter().filter(|&&c| c == '\n').count();
                let col = chars.iter().rev().take_while(|&&c| c != '\n').count();
                TextPosition::new(line, col)
            };
            for step in 0..150 {
                let start = rng.below(expected.len() + 1);
                let end = (start + rng.below(8)).min(expected.len());
                let added = ["a", "中", "🙂", "\n", "\t", "x\ny"][rng.below(6)];
                let before = expected.iter().collect::<String>();
                let selection =
                    Selection::new(position(&expected[..start]), position(&expected[..end]));
                doc.editor.selection = selection;
                let hint = doc.selection_hint();
                doc.edit(EditKind::Other, hint, |e| e.insert_text(added));
                expected.splice(start..end, added.chars());
                let after = expected.iter().collect::<String>();
                let caret = position(&expected[..start + added.chars().count()]);
                let after_selection = Selection::new(caret, caret);
                assert_eq!(
                    doc.editor.lines.join("\n"),
                    after,
                    "seed={seed}, step={step}"
                );
                assert_eq!(
                    doc.editor.selection, after_selection,
                    "seed={seed}, step={step}"
                );
                if before != after {
                    history.push((before, selection, after, after_selection));
                }
            }
            for (before, selection, _, _) in history.iter().rev() {
                assert!(doc.undo(), "seed={seed}");
                assert_eq!(&doc.editor.lines.join("\n"), before);
                assert_eq!(&doc.editor.selection, selection);
            }
            assert!(!doc.undo());
            assert!(!doc.is_modified());
            for (_, _, after, selection) in &history {
                assert!(doc.redo(), "seed={seed}");
                assert_eq!(&doc.editor.lines.join("\n"), after);
                assert_eq!(&doc.editor.selection, selection);
            }
            assert!(!doc.redo());
            assert!(doc.is_modified());
        }
    }

    #[test]
    #[ignore = "streams a 1 GiB file; run make test-large explicitly"]
    fn test_gib_file_random_access_edit_and_streaming_save() {
        use std::io::{Read, Seek, SeekFrom};
        let dir = crate::test_support::TempDir::new("gib-file");
        let source = dir.path().join("source.bin");
        let target = dir.path().join("saved.bin");
        let len = (1 << 30) + 17;
        let mut file = fs::File::create(&source).unwrap();
        file.set_len(len).unwrap();
        for (at, bytes) in [(0, b"HEAD"), (len / 2, b"MIDL"), (len - 4, b"LAST")] {
            file.seek(SeekFrom::Start(at)).unwrap();
            file.write_all(bytes).unwrap();
        }
        drop(file);
        let mut doc = Document::open(&source, &OpenOptions::default()).unwrap();
        assert!(doc.hex_only);
        assert!(doc.pristine_bytes.is_none());
        assert_eq!(doc.editor.lines, [""]);
        let hex = doc.hex.as_mut().unwrap();
        assert_eq!(hex.len(), len as usize);
        assert_eq!(hex.data.read_slice(len as usize - 4, 4), b"LAST");
        hex.move_to(65534, false);
        hex.type_bytes(b"EDIT");
        hex.move_to(len as usize - 4, false);
        hex.type_bytes(b"TAIL");
        assert_eq!(hex.data.read_slice(65532, 8), b"\0\0EDIT\0\0");
        assert!(doc.is_modified());
        doc.save(Some(&target), false).unwrap();
        assert!(!doc.is_modified());
        assert_eq!(fs::metadata(&target).unwrap().len(), len);
        // Check every byte in bounded chunks, including the large zero-filled gaps.
        let markers = [
            (0, b"HEAD"),
            (65534, b"EDIT"),
            (len as usize / 2, b"MIDL"),
            (len as usize - 4, b"TAIL"),
        ];
        let mut saved = fs::File::open(&target).unwrap();
        let mut actual = vec![0; 1024 * 1024];
        let mut expected = vec![0; actual.len()];
        let mut offset = 0;
        while offset < len as usize {
            let count = actual.len().min(len as usize - offset);
            saved.read_exact(&mut actual[..count]).unwrap();
            expected.fill(0);
            for (at, bytes) in markers {
                for (i, byte) in bytes.iter().enumerate() {
                    if (offset..offset + count).contains(&(at + i)) {
                        expected[at + i - offset] = *byte;
                    }
                }
            }
            assert!(
                actual[..count] == expected[..count],
                "stream mismatch at offset {offset}"
            );
            offset += count;
        }
        assert_eq!(saved.read(&mut [0]).unwrap(), 0);
    }

    #[test]
    fn test_decode_error_blocks_save_and_convert_but_reinterpret_recovers() {
        let dir = crate::test_support::TempDir::new("decode-error");
        let p = dir.path().join("latin.txt");
        // An incomplete UTF-8 sequence is valid Windows-1252, so recovery is lossless.
        let original = b"caf\xE9";
        fs::write(&p, original).unwrap();
        let opts = OpenOptions {
            enforce_default_encoding: true,
            default_encoding: encoding_rs::UTF_8,
            fallback_encoding: encoding_rs::BIG5,
            ..Default::default()
        };
        let mut doc = Document::open(&p, &opts).unwrap();
        assert!(
            doc.has_decode_error,
            "the fixture must exercise the save guard"
        );
        assert!(matches!(
            doc.save(None, false),
            Err(SaveError::DecodeErrors)
        ));
        assert!(doc.convert_encoding(encoding_rs::UTF_8, false).is_err());
        assert_eq!(fs::read(&p).unwrap(), original);
        assert_eq!(doc.encoding, encoding_rs::UTF_8);
        assert!(doc.has_decode_error);
        doc.reinterpret(encoding_rs::WINDOWS_1252).unwrap();
        assert_eq!(doc.editor.lines, vec!["café"]);
        assert!(!doc.has_decode_error);
        assert!(!doc.is_modified());
        doc.save(None, false).unwrap();
        assert_eq!(fs::read(&p).unwrap(), original);
    }

    #[test]
    fn test_unmappable_chars_refuse_save() {
        let p = temp_path("latin.txt");
        let mut doc = Document::new_untitled(encoding_rs::WINDOWS_1252, false, NewLineFormat::Unix, syntax::PLAIN_TEXT);
        doc.replace_all_text("café 中文");
        match doc.save(Some(&p), false) {
            Err(SaveError::Unmappable(u)) => assert_eq!(u.count, 2),
            other => panic!("預期無法表示字元的錯誤，實際為 {other:?}"),
        }
        assert!(!p.exists());
    }

    #[test]
    fn test_hex_mode_roundtrip_and_text_sync() {
        let mut doc = Document::new_untitled(encoding_rs::UTF_8, false, NewLineFormat::Unix, syntax::PLAIN_TEXT);
        doc.replace_all_text("AB\nCD");
        doc.set_mode(EditMode::Hex).unwrap();
        {
            let hb = doc.hex.as_mut().unwrap();
            hb.move_to(0, false);
            hb.type_hex_digit(0x7);
            hb.type_hex_digit(0xA); // 'A' -> 'z'
        }
        doc.set_mode(EditMode::Text).unwrap();
        assert_eq!(doc.editor.lines, vec!["zB", "CD"]);
        // 回到文字模式後，可以一步復原十六進位模式的修改
        doc.undo();
        assert_eq!(doc.editor.lines, vec!["AB", "CD"]);
    }

    #[test]
    fn test_save_failure_keeps_content_and_tmp_name_keeps_extension() {
        let mut doc = Document::default();
        doc.replace_all_text("keep me");
        let bad = std::env::temp_dir().join("madedit_no_such_dir_xyz").join("a.txt");
        assert!(doc.save(Some(&bad), false).is_err());
        assert_eq!(doc.editor.lines, vec!["keep me"]);
        assert!(doc.is_modified());
    }

    #[test]
    fn test_typing_after_save_marks_modified() {
        let p = temp_path("typing_after_save.txt");
        let mut doc = Document::default();
        let hint = doc.selection_hint();
        doc.edit(EditKind::Typing, hint, |e| e.insert_char('a'));
        doc.save(Some(&p), false).unwrap();
        assert!(!doc.is_modified());
        // 存檔後的輸入不可合併進已存檔的復原紀錄
        let hint = doc.selection_hint();
        doc.edit(EditKind::Typing, hint, |e| e.insert_char('b'));
        assert_eq!(doc.editor.lines, vec!["ab"]);
        assert!(doc.is_modified());
        // 復原一次回到存檔時的狀態
        assert!(doc.undo());
        assert_eq!(doc.editor.lines, vec!["a"]);
        assert!(!doc.is_modified());
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn test_atomic_write_failure_keeps_target_and_removes_tmp() {
        let p = temp_path("atomic_fail.txt");
        fs::write(&p, b"original").unwrap();
        // 寫入一半後失敗（模擬寫入途中出錯）：目標檔不可被截斷，也不可留下暫存檔
        let r: std::io::Result<()> = atomic_write(&p, |f| {
            f.write_all(b"half")?;
            Err(std::io::Error::other("中途失敗"))
        });
        assert!(r.is_err());
        assert_eq!(fs::read(&p).unwrap(), b"original");
        let dir = p.parent().unwrap();
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        let leftover = fs::read_dir(dir).unwrap().flatten().any(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.starts_with(&format!(".{name}.madedit-"))
        });
        assert!(!leftover, "暫存檔應已刪除");
        // 成功時取代內容
        atomic_write::<std::io::Error>(&p, |f| f.write_all(b"new")).unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"new");
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn test_save_copy_failure_keeps_existing_target() {
        let p = temp_path("copy_unmappable.txt");
        fs::write(&p, b"existing").unwrap();
        let mut doc = Document::new_untitled(encoding_rs::SHIFT_JIS, false, NewLineFormat::Unix, syntax::PLAIN_TEXT);
        doc.editor.set_text("한국어");
        assert!(matches!(doc.save_copy(&p), Err(SaveError::Unmappable(_))));
        assert_eq!(fs::read(&p).unwrap(), b"existing");
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn test_save_copy_refuses_decode_errors() {
        let p = temp_path("copy_decode_error.txt");
        fs::write(&p, b"existing").unwrap();
        let mut doc = Document::default();
        doc.editor.set_text("A\u{FFFD}B");
        doc.has_decode_error = true;
        assert!(matches!(doc.save_copy(&p), Err(SaveError::DecodeErrors)));
        assert_eq!(fs::read(&p).unwrap(), b"existing");
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn test_large_file_opens_hex_only_and_saves() {
        let p = temp_path("big.bin");
        fs::write(&p, vec![b'x'; 5000]).unwrap();
        let opts = OpenOptions { max_text_size: 1000, ..Default::default() };
        let mut doc = Document::open(&p, &opts).unwrap();
        assert!(doc.hex_only);
        assert_eq!(doc.mode, EditMode::Hex);
        assert!(doc.set_mode(EditMode::Text).is_err());
        doc.hex.as_mut().unwrap().type_bytes(b"Y");
        assert!(doc.is_modified());
        doc.save(None, false).unwrap();
        assert!(!doc.is_modified());
        let data = fs::read(&p).unwrap();
        assert_eq!(data.len(), 5000);
        assert_eq!(data[0], b'Y');
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn test_external_change_detection() {
        let p = temp_path("ext.txt");
        fs::write(&p, "v1").unwrap();
        let mut doc = Document::open(&p, &OpenOptions::default()).unwrap();
        assert!(!doc.changed_on_disk());
        fs::write(&p, "version 2").unwrap();
        assert!(doc.changed_on_disk());
        doc.ignore_disk_change();
        assert!(!doc.changed_on_disk());
        fs::write(&p, "v3!").unwrap();
        doc.reload(&OpenOptions::default()).unwrap();
        assert_eq!(doc.editor.lines, vec!["v3!"]);
        assert!(!doc.is_modified());
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn test_byte_offset_of_multibyte() {
        let mut doc = Document::new_untitled(encoding_rs::UTF_8, true, NewLineFormat::Dos, syntax::PLAIN_TEXT);
        doc.replace_all_text("中a\nb");
        // BOM 3 + "中a" 4 + CRLF 2 + 0
        assert_eq!(doc.byte_offset_of(TextPosition::new(1, 0)), 9);
        assert_eq!(doc.byte_offset_of(TextPosition::new(0, 1)), 6);
    }

    #[test]
    fn test_undo_keeps_bookmarks_added_after_edit() {
        let mut doc = Document::default();
        doc.editor.set_text("a\nb\nc");
        // 在第 0 行插入一行，再於移位後的第 2、3 行加書籤
        doc.editor.set_caret(0, 0);
        let hint = doc.selection_hint();
        doc.edit(EditKind::Other, hint, |e| e.insert_text("new\n"));
        assert_eq!(doc.editor.lines, vec!["new", "a", "b", "c"]);
        doc.editor.set_caret(2, 0);
        doc.editor.toggle_bookmark();
        doc.editor.set_caret(3, 0);
        doc.editor.toggle_bookmark();
        // 復原後書籤保留並隨行號回移
        assert!(doc.undo());
        assert_eq!(doc.editor.lines, vec!["a", "b", "c"]);
        assert_eq!(doc.editor.bookmarks, BTreeSet::from([1, 2]));
        // 復原後取消一個書籤，重做時該取消也保留
        doc.editor.set_caret(1, 0);
        doc.editor.toggle_bookmark();
        assert!(doc.redo());
        assert_eq!(doc.editor.bookmarks, BTreeSet::from([3]));
    }

    #[test]
    fn test_undo_still_restores_bookmarks_moved_by_edit() {
        let mut doc = Document::default();
        doc.editor.set_text("a\nb");
        doc.editor.set_caret(1, 0);
        doc.editor.toggle_bookmark();
        // 刪除第 0 行，書籤隨之移到第 0 行；復原應回到第 1 行
        doc.editor.set_caret(0, 0);
        let hint = doc.selection_hint();
        doc.edit(EditKind::Other, hint, |e| e.delete_lines());
        assert_eq!(doc.editor.bookmarks, BTreeSet::from([0]));
        assert!(doc.undo());
        assert_eq!(doc.editor.bookmarks, BTreeSet::from([1]));
    }

    #[test]
    fn test_undo_keeps_bookmark_toggled_on_edited_line() {
        // 審查報告 2.6 的重現步驟：輸入後在同一行加書籤，復原不可吃掉書籤
        let mut doc = Document::default();
        let hint = doc.selection_hint();
        doc.edit(EditKind::Typing, hint, |e| e.insert_char('x'));
        doc.editor.toggle_bookmark();
        assert!(doc.undo());
        assert_eq!(doc.editor.bookmarks, BTreeSet::from([0]));
    }
}
