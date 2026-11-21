//! App 內的 Markdown 預覽視窗：以 `egui_commonmark` 繪製 Markdown，
//! 其中 ```mermaid 區塊交給 `fermaid` 轉成 SVG，再由 `egui_extras` 的 SVG 載入器顯示。
//!
//! 預覽內容會隨文件版本（`Document::version`）自動更新。

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, Color32, RichText, emath::Float as _};
use egui::load::{BytesPoll, ImageLoadResult, ImageLoader, ImagePoll, LoadError, SizeHint};
use egui::mutex::Mutex;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};

use super::form;
use super::i18n::{self, Lang};

/// 文件被切成的片段：一般 Markdown 或 Mermaid 圖表原始碼
#[derive(Debug, Clone, PartialEq)]
pub enum Segment {
    /// 交給 egui_commonmark 繪製的 Markdown 文字
    Markdown(String),
    /// Mermaid 圖表原始碼（不含圍欄行）
    Mermaid(String),
}

/// 片段與其在文件中的起始行（從 0 起算；Mermaid 為開頭圍欄所在行）
#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    pub line: usize,
    pub seg: Segment,
}

/// 判斷一行是否為 mermaid 圍欄開頭，回傳（圍欄字元, 圍欄長度）
fn mermaid_fence_open(line: &str) -> Option<(char, usize)> {
    let t = line.trim_start();
    // 縮排超過 3 個空白就是縮排程式碼區塊，不是圍欄
    if line.len() - t.len() > 3 {
        return None;
    }
    let ch = t.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let n = t.chars().take_while(|c| *c == ch).count();
    if n < 3 {
        return None;
    }
    // 圍欄後的第一個單字必須是 mermaid
    let info = t[n..].trim();
    (info.split_whitespace().next() == Some("mermaid")).then_some((ch, n))
}

/// 判斷一行是否為關閉圍欄（同字元、長度不少於開頭、之後只能有空白）
fn is_fence_close(line: &str, ch: char, n: usize) -> bool {
    let t = line.trim();
    t.chars().count() >= n && t.chars().all(|c| c == ch)
}

/// 將 Markdown 文字切成一般 Markdown 與 Mermaid 片段，並記錄各片段的起始行。
/// 其他語言的程式碼區塊（例如內容剛好寫了 mermaid 圍欄的 ```text）不受影響，
/// 因為只有開頭圍欄標示 mermaid 才會被擷取，且開頭圍欄在其他程式碼區塊內不會被辨識。
pub fn split_mermaid(text: &str) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut md = String::new();
    // 目前片段的起始行、下一個要讀的行號
    let mut md_line = 0usize;
    let mut line_no = 0usize;
    // 目前是否位於「非 mermaid」的程式碼區塊內（內部的 mermaid 字樣要忽略）
    let mut other_fence: Option<(char, usize)> = None;
    let mut lines = text.split_inclusive('\n');
    while let Some(line) = lines.next() {
        let this_line = line_no;
        line_no += 1;
        let body = line.trim_end_matches(['\r', '\n']);
        if let Some((ch, n)) = other_fence {
            if is_fence_close(body, ch, n) {
                other_fence = None;
            }
            md.push_str(line);
            continue;
        }
        if let Some((ch, n)) = mermaid_fence_open(body) {
            if !md.is_empty() {
                out.push(Piece { line: md_line, seg: Segment::Markdown(std::mem::take(&mut md)) });
            }
            let mut src = String::new();
            // 讀到關閉圍欄為止；沒有關閉圍欄時視為到文件結尾
            for l in lines.by_ref() {
                line_no += 1;
                if is_fence_close(l.trim_end_matches(['\r', '\n']), ch, n) {
                    break;
                }
                src.push_str(l);
            }
            out.push(Piece { line: this_line, seg: Segment::Mermaid(src) });
            continue;
        }
        // 一般程式碼區塊開頭：記下來，避免區塊內容被誤判
        let t = body.trim_start();
        if body.len() - t.len() <= 3
            && let Some(ch) = t.chars().next().filter(|c| *c == '`' || *c == '~')
        {
            let n = t.chars().take_while(|c| *c == ch).count();
            if n >= 3 {
                other_fence = Some((ch, n));
            }
        }
        if md.is_empty() {
            md_line = this_line;
        }
        md.push_str(line);
    }
    if !md.is_empty() {
        out.push(Piece { line: md_line, seg: Segment::Markdown(md) });
    }
    out
}

/// 把一段 Markdown 依頂層區塊切開，回傳（起始行, 結尾下一行, 區塊文字）。
/// 分開繪製後才能量到每個區塊在預覽中的位置，用來和編輯區同步捲動。
/// 連結參照定義（`[a]: url`）可能在別的區塊，所以附加到每個區塊後面；
/// 含註腳定義時定義與引用會跨區塊，改為整段不切。
pub fn split_blocks(md: &str) -> Vec<(usize, usize, String)> {
    use pulldown_cmark::{Event, Options, Parser, Tag};
    // 與 egui_commonmark 解析時的選項一致，表格等才不會被切壞
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_DEFINITION_LIST;
    let parser = Parser::new_ext(md, opts);
    let defs = reference_definitions(&parser);
    let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
    let (mut depth, mut start) = (0usize, 0usize);
    for (ev, r) in parser.into_offset_iter() {
        match ev {
            Event::Start(Tag::FootnoteDefinition(_)) => return vec![(0, md.lines().count(), md.to_string())],
            Event::Start(_) => {
                if depth == 0 {
                    start = r.start;
                }
                depth += 1;
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    ranges.push(start..r.end);
                }
            }
            _ if depth == 0 => ranges.push(r),
            _ => {}
        }
    }
    // 起始行用累加方式計算，避免每個區塊都從頭數換行
    let (mut line, mut pos) = (0usize, 0usize);
    ranges
        .into_iter()
        .map(|r| {
            line += md[pos..r.start].matches('\n').count();
            pos = r.start;
            let end_line = line + md[r.clone()].lines().count();
            let mut text = md[r.clone()].trim_end_matches('\n').to_string();
            if !defs.is_empty() {
                text.push_str("\n\n");
                text.push_str(&defs);
            }
            (line, end_line, text)
        })
        .collect()
}

/// 參照定義可能位於 Mermaid 區塊另一側，先保留整份文件的定義供各段使用。
fn reference_definitions(parser: &pulldown_cmark::Parser<'_>) -> String {
    let mut defs = String::new();
    for (label, d) in parser.reference_definitions().iter() {
        defs.push_str(&format!("[{label}]: <{}>", d.dest.replace('&', "&amp;").replace('<', "%3C").replace('>', "%3E")));
        if let Some(t) = &d.title {
            defs.push_str(&format!(" \"{}\"", t.replace('&', "&amp;").replace('\\', "\\\\").replace('"', "\\\"")));
        }
        defs.push('\n');
    }
    defs
}

/// egui_commonmark draws the events inside a block quote one by one without its table
/// handling, so a quoted table comes out as one run of cell text. For a top-level quote
/// that holds a table, return its content with the `>` markers removed; the preview then
/// draws the quote bar itself and renders the content as ordinary Markdown.
fn unquote_table_block(block: &str) -> Option<String> {
    use pulldown_cmark::{Event, Options, Parser, Tag};
    let mut events = Parser::new_ext(block, Options::ENABLE_TABLES);
    if !matches!(events.next(), Some(Event::Start(Tag::BlockQuote(_)))) {
        return None;
    }
    if !events.any(|e| matches!(e, Event::Start(Tag::Table(_)))) {
        return None;
    }
    // Strip one quote level per line: up to three spaces of indent, `>`, and one optional
    // space. Lazy continuation lines have no marker and are kept as they are.
    let lines: Vec<&str> = block
        .lines()
        .map(|line| {
            let rest = line.trim_start_matches(' ');
            match rest.strip_prefix('>') {
                Some(quoted) if line.len() - rest.len() <= 3 => quoted.strip_prefix(' ').unwrap_or(quoted),
                _ => line,
            }
        })
        .collect();
    Some(lines.join("\n"))
}

/// 是否為相對路徑圖片位址：沒有協定（`xxx://`、`data:`）、不是絕對路徑、不是 Windows 磁碟機路徑
fn is_relative_image_url(url: &str) -> bool {
    let b = url.as_bytes();
    !url.is_empty()
        && !url.contains("://")
        && !url.starts_with("data:")
        && !url.starts_with(['/', '\\', '#'])
        && !(b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
}

/// 將本機圖片轉成明確的 file URI；百分比編碼由既有非同步檔案載入器處理。
fn image_uri(url: &str, base_dir: Option<&Path>) -> String {
    if url.contains("://") || url.starts_with("data:") || url.starts_with('#') {
        return url.to_string();
    }
    let full = if is_relative_image_url(url) {
        let Some(base) = base_dir else { return url.to_string() };
        base.join(url).to_string_lossy().into_owned()
    } else {
        url.to_string()
    }.replace('\\', "/");
    if full.starts_with("//") {
        format!("file:{full}")
    } else if full.as_bytes().get(1) == Some(&b':') {
        format!("file:///{full}")
    } else {
        format!("file://{full}")
    }
}

/// 只改寫圖片，不改動共用參照定義的文字連結；保留原始替代文字的格式與跳脫字元。
/// 回傳繪製用的 Markdown 與實際圖片 URI，供錯誤提示及重新載入使用。
pub fn rewrite_image_paths(md: &str, base_dir: Option<&Path>) -> (String, Vec<String>) {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};
    let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    let mut uris = Vec::new();
    let mut events = Parser::new(md).into_offset_iter();
    while let Some((ev, range)) = events.next() {
        let Event::Start(Tag::Image { dest_url, title, .. }) = ev else { continue };
        let uri = image_uri(&dest_url, base_dir);
        uris.push(uri.clone());
        if uri == dest_url.as_ref() {
            continue;
        }
        // 子事件的來源範圍包含格式標記、程式碼與巢狀括號，可準確取得替代文字結尾；
        // 不用搜尋解碼後的網址，避免標題含相同網址或網址有跳脫字元時換錯位置。
        let mut label_end = range.start + 2;
        let mut depth = 1;
        for (ev, r) in events.by_ref() {
            match ev {
                Event::Start(Tag::Image { .. }) => depth += 1,
                Event::End(TagEnd::Image) => {
                    depth -= 1;
                    if depth == 0 { break; }
                }
                _ => {}
            }
            label_end = label_end.max(r.end);
        }
        let title = if title.is_empty() { String::new() } else {
            format!(" \"{}\"", title.replace('&', "&amp;").replace('\\', "\\\\").replace('"', "&quot;").replace('\n', "&#10;").replace('\r', "&#13;"))
        };
        let escaped_uri = uri.replace('&', "&amp;").replace('<', "%3C").replace('>', "%3E");
        edits.push((range.clone(), format!("{}](<{escaped_uri}>{title})", &md[range.start..label_end])));
    }
    let mut out = md.to_string();
    // 由後往前替換，前面的位置才不會位移
    for (r, new) in edits.into_iter().rev() {
        out.replace_range(r, &new);
    }
    (out, uris)
}

/// 渲染完成、可顯示的 Mermaid 圖
struct Diagram {
    /// 原始碼相同且主題、排版寬度未變時，可直接沿用轉換結果。
    source: String,
    /// SVG 載入器的快取識別（含內容雜湊，內容或主題變更即換新）
    uri: String,
    width: f32,
    height: f32,
}

/// 單一片段的繪製資料
enum Content {
    Markdown(String),
    /// A block quote holding a table: the quote content without its `>` markers
    Quote(String),
    Diagram(Arc<Diagram>),
    /// Mermaid 語法錯誤：位置、訊息與原始碼（位置文字於繪製時依介面語言組合）
    Error { line: u32, column: u32, message: String, source: String },
}

/// 一個繪製項目與它在原始文件中的起始行（同步捲動用）
struct Item {
    line: usize,
    end_line: usize,
    content: Content,
}

/// 縮放範圍與步進
const ZOOM_MIN: f32 = 0.5;
const ZOOM_MAX: f32 = 3.0;
const ZOOM_STEP: f32 = 1.1;
/// 預覽一般內文在 100% 縮放時的字級。
const BODY_FONT_SIZE: f32 = 16.0;
/// 符合寬度時，重新排版用的寬度以此為單位量化，避免拖曳視窗邊緣時每個像素都重繪
const WIDTH_BUCKET: f32 = 32.0;
/// Mermaid 圖表的 uri 前綴，只有這類 uri 交給 [`DiagramLoader`]
pub(super) const DIAGRAM_URI_PREFIX: &str = "bytes://mermaid-";
/// 圖表點陣化倍率上限（避免高倍縮放時產生過大的貼圖）
const RASTER_MAX: f32 = 6.0;

/// 依編輯區第一行，在（起始行, y）錨點之間線性內插出預覽應捲到的 y。
/// 錨點需依行號遞增排序；沒有錨點時回傳 None。
pub fn scroll_target(anchors: &[(usize, f32)], line: usize, fraction: f32) -> Option<f32> {
    if anchors.is_empty() { return None; }
    let Some(i) = anchors.partition_point(|a| a.0 <= line).checked_sub(1) else { return Some(0.0) };
    let (l0, y0) = anchors[i];
    let Some(&(l1, y1)) = anchors.get(i + 1) else { return Some(y0) };
    if l1 <= l0 {
        return Some(y0);
    }
    Some(y0 + (y1 - y0) * ((line - l0) as f32 + fraction.clamp(0.0, 1.0)) / (l1 - l0) as f32)
}

/// 將預覽頂端反向換算成來源行與行內進度；同高錨點取最後一個，避開空白／隱藏內容。
fn source_position(anchors: &[(usize, f32)], y: f32) -> Option<(usize, f32)> {
    let i = anchors.partition_point(|a| a.1 <= y).checked_sub(1);
    let Some(i) = i else { return (!anchors.is_empty()).then_some((0, 0.0)) };
    let (l0, y0) = anchors[i];
    let Some(&(l1, y1)) = anchors.get(i + 1) else { return Some((l0, 0.0)) };
    let line = l0 as f32 + (l1 - l0) as f32 * ((y - y0) / (y1 - y0)).clamp(0.0, 1.0);
    Some((line.floor() as usize, line.fract()))
}

/// Markdown 預覽狀態（每份文件各自一份，只顯示目前選取的預覽）
pub struct MarkdownPreview {
    /// 已建立內容時的條件；任一變動就重建
    built: Option<BuildKey>,
    items: Vec<Item>,
    cache: CommonMarkCache,
    /// Shared with the 'static math callback egui_commonmark takes.
    math: std::rc::Rc<super::mdmath::MathCache>,
    image_uris: Vec<String>,
    /// 文件結尾也要有錨點，最後一個長段落／圖表才能在區塊內繼續捲動。
    source_lines: usize,
    /// 縮放比例（文字與圖表一起縮放）
    zoom: f32,
    /// 符合寬度：圖表依視窗寬度重新排版，仍太寬才縮小
    fit_width: bool,
    /// 擺放方式：獨立分頁、編輯區右側或下方
    pub placement: Placement,
    /// 剛切換成並排，下一幀把面板設回編輯區的一半大小
    pub(super) reset_size: bool,
    /// 上幀可用長度與面板外框長度；維持各半時，原生視窗尺寸改變也要重新平分。
    pub(super) dock_geometry: Option<(f32, f32)>,
    pub(super) resize_started: bool,
    pub(super) resize_to: Option<f32>,
    /// 編輯區與預覽雙向同步捲動，切換擺放方式時保留使用者選擇。
    sync_scroll: bool,
    /// 上一幀的內容可用寬度（下一次重建時決定圖表排版寬度）
    avail_w: f32,
    /// 上一幀量到的（起始行, y）錨點
    pub(super) anchors: Vec<(usize, f32)>,
    /// 上次觀察到的編輯區位置；變動才由編輯區主導同步。
    pub(super) synced_line: Option<usize>,
    synced_fraction: f32,
    /// 內容、縮放、面板寬度或圖片載入改變排版後，使用新量到的錨點再次同步。
    sync_pending: bool,
    /// 預覽主導時保留精確來源位置，避免編輯區整列捲動的取整誤差把預覽拉回。
    preview_source: Option<(usize, f32)>,
    last_offset: Option<f32>,
    last_viewport_size: egui::Vec2,
    #[cfg(test)]
    pub(super) scroll_position: (f32, f32),
    /// 並排時上一幀預覽面板所在範圍（主視窗據此避免 Ctrl+滾輪同時縮放編輯區字級）
    pub(super) rect: egui::Rect,
}

/// 重建預覽內容的條件
#[derive(Debug, Clone, PartialEq)]
struct BuildKey {
    version: u64,
    dark: bool,
    base_dir: Option<PathBuf>,
    /// 圖表排版寬度（量化後；0 表示不限）
    width: u32,
}

/// 預覽的擺放方式（類似 Visual Studio 的分割視窗）
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Placement {
    /// 主視窗內的獨立預覽分頁
    Tab,
    /// 左右並排：預覽在編輯區右側
    Right,
    /// 上下並排：預覽在編輯區下方
    Bottom,
}

/// 預覽視窗回報給 App 的動作
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MdAction {
    None,
    Close,
    ScrollTo { line: usize, fraction: f32 },
}

impl MarkdownPreview {
    pub fn new() -> Self {
        Self {
            built: None,
            items: Vec::new(),
            cache: CommonMarkCache::default(),
            math: Default::default(),
            image_uris: Vec::new(),
            source_lines: 0,
            zoom: 1.0,
            fit_width: true,
            placement: Placement::Tab,
            reset_size: false,
            dock_geometry: None,
            resize_started: false,
            resize_to: None,
            sync_scroll: true,
            avail_w: 0.0,
            anchors: Vec::new(),
            synced_line: None,
            synced_fraction: 0.0,
            sync_pending: true,
            preview_source: None,
            last_offset: None,
            last_viewport_size: egui::Vec2::ZERO,
            #[cfg(test)]
            scroll_position: (0.0, 0.0),
            rect: egui::Rect::NOTHING,
        }
    }

    /// 符合寬度時給 fermaid 的排版寬度（量化後的像素，已扣除縮放）；不限制則為 0
    fn layout_width(&self) -> u32 {
        if !self.fit_width || self.avail_w <= 0.0 {
            return 0;
        }
        (((self.avail_w / self.zoom) / WIDTH_BUCKET).floor().max(1.0) * WIDTH_BUCKET) as u32
    }

    /// 文件版本、主題、資料夾或圖表排版寬度改變時才重建內容（Mermaid 渲染只在此發生，平時每幀不重算）
    pub fn refresh(&mut self, ctx: &egui::Context, text: &str, version: u64, dark: bool, base_dir: Option<PathBuf>) {
        // 另存新檔改變資料夾時，相對路徑圖片也要重新解析，所以資料夾也是重建條件
        let key = BuildKey { version, dark, base_dir, width: self.layout_width() };
        if self.built.as_ref() == Some(&key) {
            return;
        }
        // 只索引上一版仍在文件中的圖表，不累積編輯歷史；內文、行號或資料夾變動不影響圖表。
        // 主題或排版寬度改變則全部重算，避免沿用不合目前顯示條件的 SVG。
        let diagrams: HashMap<&str, &Arc<Diagram>> = if self.built.as_ref().is_some_and(|old| old.dark == key.dark && old.width == key.width) {
            self.items.iter().filter_map(|item| match &item.content {
                Content::Diagram(d) => Some((d.source.as_str(), d)),
                _ => None,
            }).collect()
        } else {
            HashMap::new()
        };
        // SVG 載入器（resvg）不支援 CSS 變數，請 fermaid 直接依深淺色調色盤輸出實際顏色
        let mut palette = if dark { fermaid::Palette::dark() } else { fermaid::Palette::light() };
        // Edge labels sit on patches of `bg`; use the preview's own fill so they do not show as boxes.
        let fill = preview_fill(dark);
        palette.bg = format!("#{:02x}{:02x}{:02x}", fill.r(), fill.g(), fill.b());
        let opts = fermaid::RenderOptions {
            max_width: (key.width > 0).then_some(key.width as f64),
            palette: Some(palette),
            ..Default::default()
        };
        let mut items = Vec::new();
        let source_lines = text.lines().count();
        let mut image_uris = Vec::new();
        let has_images = text.contains("![");
        let definitions = if has_images { reference_definitions(&pulldown_cmark::Parser::new(text)) } else { String::new() };
        for piece in split_mermaid(text) {
            match piece.seg {
                Segment::Markdown(m) => {
                    let piece_end = piece.line + m.lines().count();
                    let m = if definitions.is_empty() { m } else { format!("{m}\n\n{definitions}") };
                    for (off, end, block) in split_blocks(&m) {
                        // 先記錄原文行號再改寫圖片，參照標題中的換行不會改變同步位置。
                        let block = if has_images && block.contains("![") {
                            let (block, uris) = rewrite_image_paths(&block, key.base_dir.as_deref());
                            image_uris.extend(uris);
                            block
                        } else { block };
                        let content = match unquote_table_block(&block) {
                            Some(inner) => Content::Quote(inner),
                            None => Content::Markdown(block),
                        };
                        items.push(Item { line: piece.line + off, end_line: (piece.line + end).min(piece_end), content });
                    }
                }
                Segment::Mermaid(src) => {
                    let end_line = (piece.line + src.lines().count() + 2).min(source_lines);
                    if let Some(d) = diagrams.get(src.as_str()) {
                        items.push(Item { line: piece.line, end_line, content: Content::Diagram(Arc::clone(d)) });
                        continue;
                    }
                    let content = match fermaid::render(&src, &opts) {
                        Ok(r) => {
                            let svg = r.svg;
                            let mut h = std::collections::hash_map::DefaultHasher::new();
                            svg.hash(&mut h);
                            let uri = format!("{DIAGRAM_URI_PREFIX}{:016x}.svg", h.finish());
                            // 交給 egui 的位元組快取；同一內容的 uri 不變，已點陣化的圖會直接沿用
                            ctx.include_bytes(uri.clone(), svg.into_bytes());
                            Content::Diagram(Arc::new(Diagram {
                                source: src,
                                uri,
                                width: r.width as f32,
                                height: r.height as f32,
                            }))
                        }
                        Err(e) => Content::Error { line: e.line, column: e.column, message: e.message, source: src },
                    };
                    items.push(Item { line: piece.line, end_line, content });
                }
            }
        }
        // 只釋放這次不再使用的舊圖（點陣化很花時間，內容沒變的圖要保留快取），避免編輯過程累積
        let keep: std::collections::HashSet<&str> = items.iter().filter_map(|it| match &it.content {
            Content::Diagram(d) => Some(d.uri.as_str()),
            _ => None,
        }).collect();
        for it in &self.items {
            if let Content::Diagram(d) = &it.content
                && !keep.contains(d.uri.as_str())
            {
                ctx.forget_image(&d.uri);
            }
        }
        image_uris.sort();
        image_uris.dedup();
        for uri in &self.image_uris {
            if image_uris.binary_search(uri).is_err() {
                ctx.forget_image(uri);
            }
        }
        self.image_uris = image_uris;
        self.items = items;
        if self.built.as_ref().is_some_and(|old| old.dark != key.dark) {
            self.math.clear(ctx);
        }
        self.built = Some(key);
        self.source_lines = source_lines;
        self.anchors.clear();
        self.sync_pending = true;
    }

    /// 是否與編輯區並排（停靠在主視窗內）
    pub fn docked(&self) -> bool {
        self.placement != Placement::Tab
    }

    /// 切換擺放方式；並排時設為編輯區的一半大小，同步選擇保持不變。
    pub fn set_placement(&mut self, placement: Placement) {
        self.placement = placement;
        self.synced_line = None;
        self.preview_source = None;
        self.last_offset = None;
        self.reset_size = self.docked();
        self.dock_geometry = None;
        self.resize_started = false;
        self.resize_to = None;
    }

    /// 回記編輯區實際可達的整列位置；下幀不把這次被動同步誤認為新的編輯區捲動。
    pub fn acknowledge_scroll(&mut self, line: usize, fraction: f32) {
        self.synced_line = Some(line);
        self.synced_fraction = fraction;
    }

    fn reload_images(&mut self, ctx: &egui::Context) {
        for uri in &self.image_uris {
            ctx.forget_image(uri);
        }
        self.sync_pending = true;
        ctx.request_repaint();
    }

    /// 工具列：縮放、符合寬度、並排顯示、同步捲動；回傳是否按下關閉
    fn toolbar(&mut self, ui: &mut egui::Ui, lang: Lang) -> bool {
        let t = |k: &str, zh: &'static str| i18n::translate(lang, k, zh);
        let mut close = false;
        ui.horizontal_wrapped(|ui| {
            if form::icon_button(ui, egui::Button::new("−"), t("md.zoom_out", "縮小（Ctrl+滾輪）"), t("md.zoom_out", "縮小（Ctrl+滾輪）")).clicked() {
                self.zoom = (self.zoom / ZOOM_STEP).max(ZOOM_MIN);
            }
            if ui.button(format!("{:.0}%", self.zoom * 100.0)).on_hover_text(t("md.zoom_reset", "還原 100%")).clicked() {
                self.zoom = 1.0;
            }
            if form::icon_button(ui, egui::Button::new("+"), t("md.zoom_in", "放大（Ctrl+滾輪）"), t("md.zoom_in", "放大（Ctrl+滾輪）")).clicked() {
                self.zoom = (self.zoom * ZOOM_STEP).min(ZOOM_MAX);
            }
            ui.separator();
            ui.checkbox(&mut self.fit_width, t("md.fit_width", "符合寬度")).on_hover_text(t("md.fit_width_tip", "圖表依視窗寬度重新排版"));
            ui.separator();
            let choices = [
                (Placement::Tab, t("md.tab", "獨立分頁"), t("md.tab_tip", "以主視窗內的獨立分頁顯示預覽")),
                (Placement::Right, t("md.right", "左右並排"), t("md.right_tip", "預覽放在編輯區右側，各佔一半")),
                (Placement::Bottom, t("md.bottom", "上下並排"), t("md.bottom_tip", "預覽放在編輯區下方，各佔一半")),
            ];
            for (p, label, tip) in choices {
                if ui.selectable_label(self.placement == p, label).on_hover_text(tip).clicked() && self.placement != p {
                    self.set_placement(p);
                }
            }
            if ui.checkbox(&mut self.sync_scroll, t("md.sync", "同步捲動")).on_hover_text(t("md.sync_tip", "捲動編輯區或預覽時，同步另一側的位置")).changed() {
                self.synced_line = None;
                self.preview_source = None;
            }
            let lbl_close = t("md.close", "關閉預覽");
            close = form::icon_button(ui, egui::Button::new("×"), lbl_close, lbl_close).clicked();
            if !self.image_uris.is_empty() && ui.button(t("md.reload_images", "重新載入圖片")).on_hover_text(t("md.reload_images_tip", "圖片檔案更新或補回後，重新讀取圖片")).clicked() {
                self.reload_images(ui.ctx());
            }
        });
        close
    }

    /// 繪製預覽內容；`top_line` 是編輯區目前第一個可見的行（從 0 起算）
    pub fn ui(&mut self, ui: &mut egui::Ui, top_line: usize, fraction: f32, lang: Lang) -> MdAction {
        self.rect = ui.max_rect();
        // 預覽分頁的 Esc 只關預覽；主視窗的關閉要求仍交給 App 處理未儲存確認。
        if !self.docked() && ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
            return MdAction::Close;
        }
        let close = egui::Panel::top("md-preview-toolbar").show(ui, |ui| {
            let close = self.toolbar(ui, lang);
            let errors: Vec<_> = self.image_uris.iter().filter_map(|uri| {
                let message = if !uri.contains("://") && !uri.starts_with("data:") {
                    Some(i18n::translate(lang, "md.save_first", "請先儲存文件，再載入相對路徑圖片。").to_string())
                } else {
                    ui.ctx().try_load_texture(uri, Default::default(), SizeHint::default()).err().map(|e| e.to_string())
                };
                message.map(|m| (uri, m))
            }).collect();
            if !errors.is_empty() {
                ui.collapsing(RichText::new(i18n::fill(i18n::translate(lang, "md.images_failed", "{n} 張圖片載入失敗"), Some(errors.len()), None)).color(ui.visuals().error_fg_color), |ui| {
                    for (uri, message) in errors {
                        ui.label(uri);
                        ui.label(message);
                    }
                });
            }
            close
        }).inner;
        if close {
            return MdAction::Close;
        }

        // Ctrl+滾輪（或觸控板捏合）縮放；並排時只在滑鼠位於預覽上才縮放，避免與編輯區字級縮放衝突
        let delta = ui.input(|i| i.zoom_delta());
        if delta != 1.0 && (!self.docked() || ui.rect_contains_pointer(ui.max_rect())) {
            self.zoom = (self.zoom * delta).clamp(ZOOM_MIN, ZOOM_MAX);
        }

        // 編輯區第一行改變時，把預覽捲到對應位置
        let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
        let editor_changed = self.synced_line != Some(top_line) || self.synced_fraction != fraction;
        if editor_changed { self.preview_source = None; }
        let mut following_editor = false;
        if self.sync_scroll && (self.sync_pending || editor_changed) {
            let (line, progress) = self.preview_source.unwrap_or((top_line, fraction));
            if let Some(y) = scroll_target(&self.anchors, line, progress) {
                area = area.vertical_scroll_offset(y);
                self.acknowledge_scroll(top_line, fraction);
                self.sync_pending = false;
                following_editor = true;
            }
        }

        let zoom = self.zoom;
        let mut anchors = Vec::with_capacity(self.items.len() * 2 + 1);
        let mut avail_w = self.avail_w;
        // 預覽分頁與並排面板共用高對比底色，避免受到外層面板底色影響。
        let fill = egui::Frame::new().fill(preview_fill(ui.visuals().dark_mode));
        let output = egui::CentralPanel::default().frame(fill).show(ui, |ui| area.show(ui, |ui| {
            high_contrast(ui.visuals_mut());
            // 文字隨縮放放大（圖表另外依尺寸縮放）
            let offset = BODY_FONT_SIZE - ui.style().text_styles[&egui::TextStyle::Body].size;
            for ts in ui.style_mut().text_styles.values_mut() {
                ts.size = (ts.size + offset) * zoom;
            }
            avail_w = ui.available_width();
            let origin = ui.cursor().min.y;
            let mut prev_markdown = false;
            for item in &self.items {
                // 區塊分開繪製後，補回原本的段落間距，再記錄實際起始位置。
                let markdown = matches!(item.content, Content::Markdown(_) | Content::Quote(_));
                if prev_markdown && markdown {
                    ui.add_space(ui.text_style_height(&egui::TextStyle::Body));
                }
                anchors.push((item.line, ui.cursor().min.y - origin));
                match &item.content {
                    Content::Markdown(m) | Content::Quote(m) => {
                        // 每個區塊各自一個 id：egui_commonmark 的表格 id 只在單次 show 內遞增，
                        // 多個區塊共用同一個 ui id 時，不同表格會共用 Grid 欄寬而文字互相覆蓋
                        ui.push_id(item.line, |ui| {
                            // Grid stripes span every column at its minimum width (interact_size.x,
                            // 40 pt), but the table frame only wraps the last column's real
                            // content, so short cells made stripes stick out past the frame.
                            // Columns then hug their text; egui_commonmark forces item_spacing.x
                            // to 0, so the column gap cannot be widened from here.
                            ui.spacing_mut().interact_size.x = 0.0;
                            let cache = std::rc::Rc::clone(&self.math);
                            let math = move |ui: &mut egui::Ui, tex: &str, inline: bool| cache.show(ui, tex, inline);
                            let viewer = CommonMarkViewer::new().explicit_image_uri_scheme(true).render_math_fn(Some(&math));
                            if matches!(item.content, Content::Quote(_)) {
                                quote_frame(ui, |ui| {
                                    viewer.show(ui, &mut self.cache, m);
                                });
                            } else {
                                viewer.show(ui, &mut self.cache, m);
                            }
                        });
                    }
                    Content::Diagram(d) => {
                        // 依縮放比例顯示；符合寬度時，仍比視窗寬才縮小
                        let mut scale = zoom;
                        if self.fit_width && d.width * scale > avail_w {
                            scale = avail_w / d.width;
                        }
                        let size = egui::vec2(d.width * scale, d.height * scale);
                        // 點陣化倍率取 0.5 的級距（無條件進位），縮放或拉寬視窗時不必每幀重新點陣化，
                        // 只差一點的倍率直接由 GPU 縮放貼圖
                        let raster = ((scale * ui.ctx().pixels_per_point() * 2.0).ceil() / 2.0).clamp(0.5, RASTER_MAX);
                        let hint = egui::load::SizeHint::Scale(raster.ord());
                        match ui.ctx().try_load_texture(&d.uri, Default::default(), hint) {
                            Ok(egui::load::TexturePoll::Ready { texture }) => {
                                ui.add(egui::Image::from_texture(texture).fit_to_exact_size(size));
                            }
                            Ok(egui::load::TexturePoll::Pending { .. }) => {
                                ui.allocate_ui(size, |ui| ui.spinner());
                            }
                            Err(e) => {
                                ui.label(RichText::new(format!("{}{e}", i18n::translate(lang, "md.diagram_failed", "圖表顯示失敗："))).color(ui.visuals().error_fg_color));
                            }
                        }
                        ui.add_space(8.0);
                    }
                    Content::Error { line, column, message, source } => {
                        let at = i18n::fill(i18n::translate(lang, "md.mermaid_error", "Mermaid 語法錯誤：第 {n} 行第 {q} 欄："), Some(*line as usize), Some(&column.to_string()));
                        ui.label(RichText::new(format!("{at}{message}")).color(ui.visuals().error_fg_color));
                        ui.code(source);
                    }
                }
                anchors.push((item.end_line, ui.cursor().min.y - origin));
                prev_markdown = markdown;
            }
            if !self.items.is_empty() {
                anchors.push((self.source_lines, ui.cursor().min.y - origin));
            }
        }));
        let offset = output.inner.state.clamped_offset().y;
        #[cfg(test)]
        { self.scroll_position = (offset, (output.inner.content_size.y - output.inner.inner_rect.height()).max(0.0)); }
        let layout_changed = output.inner.inner_rect.size() != self.last_viewport_size
            || anchors.len() != self.anchors.len()
            || anchors.iter().zip(&self.anchors).any(|(a, b)| a.0 != b.0 || (a.1 - b.1).abs() > 0.5);
        if layout_changed {
            self.sync_pending = true;
            if self.sync_scroll { ui.ctx().request_repaint(); }
        }
        // 只有預覽自行移動才反向同步：排版重算與程式設定 offset 都不能回送，避免兩側互拉。
        let moved = self.last_offset.is_some_and(|last| (offset - last).abs() > 0.01);
        self.last_offset = Some(offset);
        self.last_viewport_size = output.inner.inner_rect.size();
        self.anchors = anchors;
        // 寬度或縮放改變會讓圖表排版寬度跨級，下一幀由 refresh 重建
        self.avail_w = avail_w;
        if self.sync_scroll
            && moved
            && !following_editor
            && !layout_changed
            && let Some((line, fraction)) = source_position(&self.anchors, offset)
        {
            self.preview_source = Some((line, fraction));
            ui.ctx().request_repaint();
            return MdAction::ScrollTo { line, fraction };
        }
        MdAction::None
    }
}

/// Background of the preview tab and side panel: pure black or white for the highest contrast.
/// Mermaid diagrams paint their edge-label patches in the same color, so both must use this.
/// Same look as egui_commonmark's block quote: a 10 pt indent with a bar on its left.
fn quote_frame(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    let bar = ui.painter().add(egui::Shape::Noop);
    let rect = egui::Frame::new()
        .outer_margin(egui::Margin { left: 10, ..Default::default() })
        .show(ui, add_contents)
        .response
        .rect;
    let stroke = egui::Stroke::new(3.0, ui.visuals().weak_text_color());
    ui.painter().set(bar, egui::Shape::line_segment([rect.left_top() + egui::vec2(0.0, 5.0), rect.left_bottom() - egui::vec2(0.0, 5.0)], stroke));
}

fn preview_fill(dark: bool) -> Color32 {
    if dark { Color32::BLACK } else { Color32::WHITE }
}

/// 預覽內容的高對比配色：深色為純黑底、淺色為純白底；內文、粗體與標題都接近純黑／純白，
/// 程式碼、表格斑馬紋與引用等次要底色只比底色稍亮／稍暗，仍分得出區塊
fn high_contrast(v: &mut egui::Visuals) {
    let (text, strong, faint, code) = if v.dark_mode {
        (Color32::from_gray(225), Color32::WHITE, Color32::from_gray(22), Color32::from_gray(36))
    } else {
        (Color32::from_gray(28), Color32::BLACK, Color32::from_gray(244), Color32::from_gray(234))
    };
    v.panel_fill = if v.dark_mode { Color32::BLACK } else { Color32::WHITE };
    v.widgets.noninteractive.fg_stroke.color = text;
    v.widgets.active.fg_stroke.color = strong;
    // 引用與次要說明文字（egui 預設把內文透明度減半，在預覽中太淡）
    v.weak_text_color = Some(text.gamma_multiply(0.85));
    v.faint_bg_color = faint;
    v.extreme_bg_color = code;
    v.code_bg_color = code;
}

/// Mermaid 圖表專用的 SVG 載入器。
/// egui_extras 的 SVG 載入器要顯示文字就得載入全部系統字型（數百毫秒），
/// 這裡的字型資料庫只放一個字型，圖表文字都用它繪製。
pub struct DiagramLoader {
    options: resvg::usvg::Options<'static>,
    /// （uri, 尺寸提示）→ 點陣化結果
    cache: Mutex<HashMap<(String, SizeHint), Rasterized>>,
}

/// 帶查詢串的遠端 SVG 載入器（例如 GitHub 徽章 `badge.svg?branch=main`）。
/// egui 內建 SvgLoader 以 URI 副檔名判斷，但其 `has_extension` 只按 `#` 分割、不處理 `?`，
/// 帶查詢串時副檔名會被判成 `svg?branch=main` 而拒載；此載入器補位：去掉查詢串後
/// 以 `.svg` 結尾的 http(s) URI 由這裡取位元組並用 resvg 點陣化。
pub struct RemoteSvgLoader {
    options: resvg::usvg::Options<'static>,
    cache: Mutex<HashMap<(String, SizeHint), Rasterized>>,
}

type Rasterized = Result<Arc<egui::ColorImage>, String>;

impl RemoteSvgLoader {
    pub const ID: &'static str = egui::generate_loader_id!(RemoteSvgLoader);

    pub fn install(ctx: &egui::Context, font: Option<(String, fontique::Blob<u8>)>) {
        let mut options = resvg::usvg::Options::default();
        if let Some((name, blob)) = font {
            let db = options.fontdb_mut();
            db.load_font_source(resvg::usvg::fontdb::Source::Binary(Arc::new(blob)));
            db.set_sans_serif_family(&name);
            db.set_monospace_family(&name);
        }
        ctx.add_image_loader(Arc::new(Self { options, cache: Mutex::new(HashMap::new()) }));
    }
}

impl ImageLoader for RemoteSvgLoader {
    fn id(&self) -> &str {
        Self::ID
    }

    fn load(&self, ctx: &egui::Context, uri: &str, size_hint: SizeHint) -> ImageLoadResult {
        // 只接「帶查詢串、去查詢後以 .svg 結尾」的 URI；其餘（含無查詢串的 .svg）交給內建載入器
        let base = uri.split('#').next().unwrap_or(uri);
        let Some((path, query)) = base.split_once('?') else {
            return Err(LoadError::NotSupported);
        };
        if query.is_empty() || !path.to_ascii_lowercase().ends_with(".svg") {
            return Err(LoadError::NotSupported);
        }
        let key = (uri.to_owned(), size_hint);
        let result = if let Some(r) = self.cache.lock().get(&key) {
            r.clone()
        } else {
            match ctx.try_load_bytes(uri) {
                Ok(BytesPoll::Ready { bytes, .. }) => {
                    let r = egui_extras::image::load_svg_bytes_with_size(&bytes, size_hint, &self.options).map(Arc::new);
                    self.cache.lock().insert(key, r.clone());
                    r
                }
                Ok(BytesPoll::Pending { size }) => return Ok(ImagePoll::Pending { size }),
                Err(e) => return Err(e),
            }
        };
        result.map(|image| ImagePoll::Ready { image }).map_err(LoadError::Loading)
    }

    fn forget(&self, uri: &str) {
        self.cache.lock().retain(|(u, _), _| u != uri);
    }

    fn forget_all(&self) {
        self.cache.lock().clear();
    }

    fn byte_size(&self) -> usize {
        self.cache.lock().values().filter_map(|r| r.as_ref().ok()).map(|img| img.pixels.len() * 4).sum()
    }
}

impl DiagramLoader {
    pub const ID: &'static str = egui::generate_loader_id!(DiagramLoader);

    /// 安裝到 egui（呼叫端先以 `ID` 確認尚未安裝）；`font` 為（家族名稱, 字型檔內容），沒有字型時圖表不顯示文字
    pub fn install(ctx: &egui::Context, font: Option<(String, fontique::Blob<u8>)>) {
        let mut options = resvg::usvg::Options::default();
        if let Some((name, blob)) = font {
            let db = options.fontdb_mut();
            // 直接共用 fontique 已載入的字型檔，不另外複製
            db.load_font_source(resvg::usvg::fontdb::Source::Binary(Arc::new(blob)));
            // fermaid 的字型清單以 sans-serif／monospace 結尾，兩者都對應到這個字型
            db.set_sans_serif_family(&name);
            db.set_monospace_family(&name);
        }
        ctx.add_image_loader(Arc::new(Self { options, cache: Mutex::new(HashMap::new()) }));
    }
}

impl ImageLoader for DiagramLoader {
    fn id(&self) -> &str {
        Self::ID
    }

    fn load(&self, ctx: &egui::Context, uri: &str, size_hint: SizeHint) -> ImageLoadResult {
        // 其他圖片（Markdown 內的 png、svg 等）交給 egui_extras 的載入器
        if !uri.starts_with(DIAGRAM_URI_PREFIX) {
            return Err(LoadError::NotSupported);
        }
        let key = (uri.to_owned(), size_hint);
        let result = if let Some(r) = self.cache.lock().get(&key) {
            r.clone()
        } else {
            match ctx.try_load_bytes(uri) {
                Ok(BytesPoll::Ready { bytes, .. }) => {
                    let r = egui_extras::image::load_svg_bytes_with_size(&bytes, size_hint, &self.options).map(Arc::new);
                    self.cache.lock().insert(key, r.clone());
                    r
                }
                Ok(BytesPoll::Pending { size }) => return Ok(ImagePoll::Pending { size }),
                Err(e) => return Err(e),
            }
        };
        result.map(|image| ImagePoll::Ready { image }).map_err(LoadError::Loading)
    }

    fn forget(&self, uri: &str) {
        self.cache.lock().retain(|(u, _), _| u != uri);
    }

    fn forget_all(&self) {
        self.cache.lock().clear();
    }

    fn byte_size(&self) -> usize {
        self.cache.lock().values().filter_map(|r| r.as_ref().ok()).map(|img| img.pixels.len() * 4).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIAGRAM_A: &str = "```mermaid\nflowchart LR\nA[開始] --> B[結束]\n```\n";
    const DIAGRAM_B: &str = "```mermaid\nflowchart LR\nX[另一張] --> Y[結束]\n```\n";

    fn diagrams(preview: &MarkdownPreview) -> Vec<Arc<Diagram>> {
        preview.items.iter().filter_map(|item| match &item.content {
            Content::Diagram(d) => Some(Arc::clone(d)),
            _ => None,
        }).collect()
    }

    /// 以物件身分確認未重新轉換，而非只比對重新轉換後也可能相同的 SVG。
    #[test]
    fn test_refresh_reuses_diagrams_after_text_move_and_save_as() {
        let ctx = egui::Context::default();
        let mut preview = MarkdownPreview::new();
        let text = format!("# 原標題\n\n![圖](images/a.png)\n\n{DIAGRAM_A}\n{DIAGRAM_B}");
        preview.refresh(&ctx, &text, 0, false, Some(PathBuf::from("/old")));
        let before = diagrams(&preview);
        assert_eq!(before.len(), 2);
        let changed = format!("# 新標題\n\n新增段落\n\n![圖](images/a.png)\n\n{DIAGRAM_B}\n{DIAGRAM_A}");
        preview.refresh(&ctx, &changed, 1, false, Some(PathBuf::from("/new")));
        let after = diagrams(&preview);
        assert_eq!(after.len(), 2);
        assert!(Arc::ptr_eq(&before[0], &after[1]));
        assert!(Arc::ptr_eq(&before[1], &after[0]));
        let lines: Vec<_> = preview.items.iter().filter(|item| matches!(item.content, Content::Diagram(_))).map(|item| item.line).collect();
        assert_eq!(lines, vec![6, 11], "重用圖表仍須更新同步捲動使用的來源行號");
        assert!(preview.items.iter().any(|item| matches!(&item.content, Content::Markdown(m) if m.contains("file:///new/images/a.png"))));
    }

    #[test]
    fn test_refresh_only_rebuilds_changed_diagram_and_releases_old_svg() {
        let ctx = egui::Context::default();
        let mut preview = MarkdownPreview::new();
        let text = format!("{DIAGRAM_A}\n{DIAGRAM_B}");
        preview.refresh(&ctx, &text, 0, false, None);
        let before = diagrams(&preview);
        assert_eq!(before.len(), 2);
        preview.refresh(&ctx, &text.replace("開始", "修改後"), 1, false, None);
        let after = diagrams(&preview);
        assert_eq!(after.len(), 2);
        assert!(!Arc::ptr_eq(&before[0], &after[0]));
        assert_ne!(before[0].uri, after[0].uri);
        assert!(Arc::ptr_eq(&before[1], &after[1]));
        assert!(!matches!(ctx.try_load_bytes(&before[0].uri), Ok(BytesPoll::Ready { .. })), "舊圖應釋放");
        for d in after {
            assert!(matches!(ctx.try_load_bytes(&d.uri), Ok(BytesPoll::Ready { .. })), "新圖與未修改的圖都應保留");
        }
    }

    #[test]
    fn test_diagram_label_patches_use_the_preview_background() {
        let ctx = egui::Context::default();
        let mut preview = MarkdownPreview::new();
        preview.avail_w = 768.0;
        let text = "```mermaid\nflowchart LR\n    A -->|missing| B\n```\n";
        // Only the dark palette differs: fermaid's light background is already white.
        preview.refresh(&ctx, text, 0, true, None);
        let uri = diagrams(&preview).remove(0).uri.clone();
        let Ok(BytesPoll::Ready { bytes, .. }) = ctx.try_load_bytes(&uri) else { panic!("diagram not loaded") };
        let svg = String::from_utf8_lossy(&bytes).to_lowercase();
        assert_eq!(preview_fill(true), Color32::BLACK);
        assert!(svg.contains("#000000"), "label patches should be black");
        // fermaid's own dark background, which showed as gray boxes on the black preview.
        assert!(!svg.contains("#1e1e1e"));
    }

    #[test]
    fn test_refresh_rebuilds_diagram_for_theme_and_layout_width() {
        let ctx = egui::Context::default();
        let mut preview = MarkdownPreview::new();
        preview.avail_w = 768.0;
        preview.refresh(&ctx, DIAGRAM_A, 0, false, None);
        let light = diagrams(&preview).remove(0);
        preview.refresh(&ctx, DIAGRAM_A, 0, true, None);
        let dark = diagrams(&preview).remove(0);
        assert!(!Arc::ptr_eq(&light, &dark));
        assert_ne!(light.uri, dark.uri);
        preview.avail_w = 800.0;
        preview.refresh(&ctx, DIAGRAM_A, 0, true, None);
        let resized = diagrams(&preview).remove(0);
        assert!(!Arc::ptr_eq(&dark, &resized));
        preview.avail_w = 801.0;
        preview.refresh(&ctx, DIAGRAM_A, 0, true, None);
        assert!(Arc::ptr_eq(&resized, &diagrams(&preview)[0]), "同一寬度級距不重算");
    }

    #[test]
    fn test_refresh_diagram_error_recovery_and_removal() {
        let ctx = egui::Context::default();
        let mut preview = MarkdownPreview::new();
        preview.refresh(&ctx, DIAGRAM_A, 0, false, None);
        let before = diagrams(&preview).remove(0);
        preview.refresh(&ctx, "```mermaid\nnot-a-diagram\n```\n", 1, false, None);
        assert!(matches!(preview.items[0].content, Content::Error { .. }));
        assert!(!matches!(ctx.try_load_bytes(&before.uri), Ok(BytesPoll::Ready { .. })));
        preview.refresh(&ctx, DIAGRAM_A, 2, false, None);
        let recovered = diagrams(&preview).remove(0);
        assert!(!Arc::ptr_eq(&before, &recovered), "已移除的圖表不保留歷史快取");
        preview.refresh(&ctx, "只剩一般內文\n", 3, false, None);
        assert!(diagrams(&preview).is_empty());
        assert!(!matches!(ctx.try_load_bytes(&recovered.uri), Ok(BytesPoll::Ready { .. })));
    }

    /// 以本機文件比較首次建立與連續修改內文的耗時，不把使用者文件納入版控。
    /// 執行：MADEDIT_MD_BENCH_FILE=文件 cargo test --release benchmark_preview_refresh -- --ignored --nocapture
    #[test]
    #[ignore]
    fn benchmark_preview_refresh() {
        use std::time::Instant;
        let path = PathBuf::from(std::env::var_os("MADEDIT_MD_BENCH_FILE").expect("請設定 MADEDIT_MD_BENCH_FILE"));
        let text = std::fs::read_to_string(&path).unwrap();
        let base_dir = path.parent().map(Path::to_path_buf);
        let diagrams = split_mermaid(&text).iter().filter(|p| matches!(p.seg, Segment::Mermaid(_))).count();
        let mut first = Vec::new();
        let mut edit = Vec::new();
        // 先暖機，再取多次樣本，降低檔案快取與首次初始化造成的偏差。
        for sample in 0..24 {
            let ctx = egui::Context::default();
            let mut preview = MarkdownPreview::new();
            let start = Instant::now();
            preview.refresh(&ctx, &text, 0, false, base_dir.clone());
            let elapsed = start.elapsed();
            std::hint::black_box(&preview.items);
            if sample >= 3 {
                first.push(elapsed);
            }
            // 模擬首次排版完成，取得預覽寬度之後的連續編輯。
            preview.avail_w = 768.0;
            preview.refresh(&ctx, &text, 0, false, base_dir.clone());
            let changed = format!("測試內文 {sample}\n\n{text}");
            let start = Instant::now();
            preview.refresh(&ctx, &changed, 1, false, base_dir.clone());
            let elapsed = start.elapsed();
            std::hint::black_box(&preview.items);
            if sample >= 3 {
                edit.push(elapsed);
            }
        }
        println!("Fixture: {} bytes, {diagrams} Mermaid diagrams", text.len());
        crate::test_support::report_benchmark("markdown_refresh.open", first);
        crate::test_support::report_benchmark("markdown_refresh.edit", edit);
    }

    #[test]
    fn test_split_extracts_mermaid_block() {
        let segs = split_mermaid("# 標題\n\n```mermaid\ngraph LR\n  A-->B\n```\n\n結尾\n");
        assert_eq!(
            segs,
            vec![
                Piece { line: 0, seg: Segment::Markdown("# 標題\n\n".into()) },
                Piece { line: 2, seg: Segment::Mermaid("graph LR\n  A-->B\n".into()) },
                Piece { line: 6, seg: Segment::Markdown("\n結尾\n".into()) },
            ]
        );
    }

    #[test]
    fn test_split_ignores_mermaid_inside_other_code_block() {
        let text = "```text\n```mermaid\nx\n```\n";
        // 第二行落在 text 圍欄內，整份都是一般 Markdown
        assert_eq!(split_mermaid(text), vec![Piece { line: 0, seg: Segment::Markdown(text.into()) }]);
    }

    #[test]
    fn test_split_unclosed_fence_and_tilde_and_crlf() {
        assert_eq!(split_mermaid("~~~mermaid\r\npie\r\n  \"a\": 1\r\n"), vec![Piece { line: 0, seg: Segment::Mermaid("pie\r\n  \"a\": 1\r\n".into()) }]);
        assert_eq!(split_mermaid("沒有圖\n"), vec![Piece { line: 0, seg: Segment::Markdown("沒有圖\n".into()) }]);
        assert!(split_mermaid("").is_empty());
    }

    #[test]
    fn test_split_requires_mermaid_info_word() {
        let text = "```mermaidx\nfoo\n```\n";
        assert_eq!(split_mermaid(text), vec![Piece { line: 0, seg: Segment::Markdown(text.into()) }]);
    }

    #[test]
    fn test_rewrite_relative_image_uses_document_dir() {
        let base = Path::new("/home/u/docs");
        let (out, _) = rewrite_image_paths("看圖 ![圖](img/a.png \"標題\") 結束", Some(base));
        assert_eq!(out, "看圖 ![圖](<file:///home/u/docs/img/a.png> \"標題\") 結束");
        // 已用角括號包起來的位址不會變成兩層
        let (out, _) = rewrite_image_paths("![x](<pic 1.png>)", Some(base));
        assert_eq!(out, "![x](<file:///home/u/docs/pic 1.png>)");
    }

    #[test]
    fn test_rewrite_keeps_absolute_remote_and_unsaved() {
        let base = Path::new("/d");
        for md in ["![a](https://x.com/a.png)", "![a](file:///x.png)", "![a](data:image/png;base64,AAAA)"] {
            assert_eq!(rewrite_image_paths(md, Some(base)).0, md);
        }
        assert_eq!(rewrite_image_paths("![a](/abs/a.png)", Some(base)).1, vec!["file:///abs/a.png"]);
        assert_eq!(rewrite_image_paths("![a](C:\\x.png)", Some(base)).1, vec!["file:///C:/x.png"]);
        // 尚未存檔：不改寫
        assert_eq!(rewrite_image_paths("![a](a.png)", None).0, "![a](a.png)");
        // 連結（非圖片）不受影響
        assert_eq!(rewrite_image_paths("[a](a.md)", Some(base)).0, "[a](a.md)");
    }

    #[test]
    fn test_rewrite_reference_images_and_escaped_labels_without_changing_links() {
        use pulldown_cmark::{Event, Parser, Tag};
        for md in [
            "![a](img/a.png \"img/a.png\")",
            "![*粗體* `x]y` 和 [括號]](img/a.png)",
            r"![a\]b](img/a\(1\).png)",
            "![](img/a.png)",
            "![  ](img/a.png)",
            "![a][r]\n\n[r]: img/a.png \"標題\"\n\n[連結][r]",
            "![r][]\n\n[r]: img/a.png",
            "![r]\n\n[r]: img/a.png",
        ] {
            let image = |text: &str| Parser::new(text).find_map(|ev| match ev {
                Event::Start(Tag::Image { dest_url, title, .. }) => Some((dest_url.to_string(), title.to_string())),
                _ => None,
            }).unwrap();
            let (before_uri, before_title) = image(md);
            let (out, uris) = rewrite_image_paths(md, Some(Path::new("/文件 空白")));
            let (uri, title) = image(&out);
            assert_eq!(uri, format!("file:///文件 空白/{before_uri}"), "{md} → {out}");
            assert_eq!(title, before_title);
            assert_eq!(uris, vec![uri]);
            for ev in Parser::new(&out) {
                if let Event::Start(Tag::Link { dest_url, .. }) = ev {
                    assert_eq!(dest_url.as_ref(), "img/a.png", "共用參照定義的文字連結不能被改寫");
                }
            }
        }
    }

    #[test]
    fn test_reference_image_definition_across_mermaid_keeps_source_lines() {
        let ctx = egui::Context::default();
        let mut preview = MarkdownPreview::new();
        let text = format!("![圖][r]\n\n{DIAGRAM_A}\n[r]: <中文 圖.png> \"標題\"\n");
        preview.refresh(&ctx, &text, 0, false, Some(PathBuf::from("/文件")));
        assert_eq!(preview.image_uris, vec!["file:///文件/中文 圖.png"]);
        assert_eq!(preview.items.iter().find(|i| matches!(i.content, Content::Diagram(_))).unwrap().line, 2);
        assert!(matches!(&preview.items[0].content, Content::Markdown(m) if m.contains("<file:///文件/中文 圖.png>")));
    }

    #[test]
    fn test_reload_images_recovers_missing_file_and_reads_external_update() {
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        let dir = std::env::temp_dir().join(format!("madedit-images-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("中文 圖%25.png");
        let mut preview = MarkdownPreview::new();
        preview.refresh(&ctx, "![圖](<中文%20圖%2525.png>)", 0, false, Some(dir.clone()));
        let uri = preview.image_uris[0].clone();
        let load = || {
            for _ in 0..200 {
                match ctx.try_load_image(&uri, SizeHint::default()) {
                    Ok(ImagePoll::Pending { .. }) => std::thread::sleep(std::time::Duration::from_millis(5)),
                    result => return result,
                }
            }
            panic!("圖片載入逾時");
        };
        assert!(load().is_err(), "缺少的圖片應回報錯誤");
        for size in [[7, 9], [11, 13]] {
            image::RgbaImage::from_pixel(size[0], size[1], image::Rgba([120, 80, 40, 255])).save(&path).unwrap();
            preview.reload_images(&ctx);
            let Ok(ImagePoll::Ready { image }) = load() else { panic!("重新載入失敗"); };
            assert_eq!(image.size, [size[0] as usize, size[1] as usize]);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    struct PreviewTest {
        preview: MarkdownPreview,
        text: String,
        version: u64,
        top_line: usize,
        fraction: f32,
    }

    fn preview_harness(text: String, top_line: usize) -> egui_kittest::Harness<'static, PreviewTest> {
        let mut preview = MarkdownPreview::new();
        preview.set_placement(Placement::Right);
        egui_kittest::Harness::builder().allow_missing_glyphs().with_size([600.0, 500.0]).build_ui_state(|ui, s: &mut PreviewTest| {
            s.preview.refresh(ui.ctx(), &s.text, s.version, false, None);
            if let MdAction::ScrollTo { line, .. } = s.preview.ui(ui, s.top_line, s.fraction, Lang::ZhTw) {
                // 模擬未自動換行的編輯區，只有整行位置；預覽仍應保留自己的精確位置。
                s.top_line = line;
                s.fraction = 0.0;
                s.preview.acknowledge_scroll(s.top_line, s.fraction);
            }
        }, PreviewTest { preview, text, version: 0, top_line, fraction: 0.0 })
    }

    fn assert_preview_at_source_line(h: &egui_kittest::Harness<'_, PreviewTest>) {
        let s = h.state();
        let y = s.preview.anchors.iter().find(|a| a.0 == s.top_line).unwrap().1;
        let (actual, max) = s.preview.scroll_position;
        assert!((actual - y.min(max)).abs() < 1.0, "實際位置 {actual}，來源錨點 {y}，最大位置 {max}");
    }

    #[test]
    fn test_scroll_resyncs_after_edit_zoom_resize_and_toggle() {
        use egui_kittest::kittest::Queryable;
        let text: String = (0..100).map(|i| format!("## 標題 {i}\n\n段落 {i}\n\n")).collect();
        let mut h = preview_harness(text, 80);
        h.run_steps(5);
        assert_preview_at_source_line(&h);
        let before = h.state().preview.scroll_position.0;
        h.state_mut().text = h.state().text.replacen("段落 0", &"much longer text ".repeat(150), 1);
        h.state_mut().version += 1;
        h.run_steps(5);
        assert_preview_at_source_line(&h);
        assert!(h.state().preview.scroll_position.0 > before + 100.0);
        h.state_mut().preview.zoom = 1.6;
        h.run_steps(5);
        assert_preview_at_source_line(&h);
        h.set_size(egui::vec2(400.0, 500.0));
        h.run_steps(5);
        assert_preview_at_source_line(&h);
        h.get_by_label("同步捲動").click();
        h.run_steps(2);
        h.state_mut().top_line = 120;
        h.run_steps(3);
        assert_eq!(h.state().preview.synced_line, None);
        h.get_by_label("同步捲動").click();
        h.run_steps(5);
        assert_preview_at_source_line(&h);
    }

    #[test]
    fn test_scroll_reaches_end_of_last_long_block() {
        let text = format!("| 欄位 |\n|---|\n{}", "| 內容 |\n".repeat(150));
        let mut h = preview_harness(text, 140);
        h.run_steps(5);
        let (actual, max) = h.state().preview.scroll_position;
        assert!(actual > 1000.0, "不能停在最後一個區塊開頭：{actual}");
        assert!((actual - max).abs() < 1.0, "接近文件末尾時應到達捲軸底端");
    }

    #[test]
    fn test_scroll_uses_wrapped_progress_without_including_blank_lines() {
        let mut h = preview_harness(format!("{}\n\nend\n", "long text ".repeat(1500)), 0);
        h.run_steps(5);
        h.state_mut().fraction = 0.5;
        h.run_steps(5);
        let pv = &h.state().preview;
        let paragraph_end = pv.anchors.iter().find(|a| a.0 == 1).unwrap().1;
        let (actual, max) = pv.scroll_position;
        assert!(paragraph_end > 1000.0);
        assert!((actual - (paragraph_end * 0.5).min(max)).abs() < 1.0, "長行走到一半時須對應段落一半，不能把段落後的空白行也算進去");
    }

    #[test]
    fn test_scroll_resyncs_after_image_load_with_same_editor_line() {
        use egui_kittest::kittest::Queryable;
        let path = std::env::temp_dir().join(format!("madedit-image-reflow-{}.png", std::process::id()));
        let uri = image_uri(&path.to_string_lossy(), None);
        let text = format!("![圖](<{uri}>)\n\n{}", (0..100).map(|i| format!("## 標題 {i}\n\n段落 {i}\n\n")).collect::<String>());
        let mut h = preview_harness(text, 82);
        h.run_steps(5);
        assert_preview_at_source_line(&h);
        let before = h.state().preview.scroll_position.0;
        image::RgbaImage::from_pixel(400, 300, image::Rgba([120, 80, 40, 255])).save(&path).unwrap();
        h.get_by_label("重新載入圖片").click();
        h.run_steps(1);
        let mut loaded = false;
        for _ in 0..200 {
            if matches!(h.ctx.try_load_image(&uri, SizeHint::default()), Ok(ImagePoll::Ready { .. })) {
                loaded = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(loaded, "補回的圖片應能重新載入");
        h.run_steps(5);
        assert_preview_at_source_line(&h);
        assert!(h.state().preview.scroll_position.0 > before + 200.0, "圖片高度改變後須重新同步");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn test_split_blocks_records_start_lines() {
        let md = "# 標題\n\n段落一\n續行\n\n- a\n- b\n\n```rust\nfn x() {}\n```\n";
        let b = split_blocks(md);
        let lines: Vec<usize> = b.iter().map(|x| x.0).collect();
        assert_eq!(lines, vec![0, 2, 5, 8]);
        assert_eq!(b[1].1, 4);
        assert_eq!(b[1].2, "段落一\n續行");
        assert!(b[3].2.starts_with("```rust"));
    }

    #[test]
    fn test_split_blocks_keeps_table_whole_and_appends_ref_defs() {
        let md = "| a | b |\n|---|---|\n| 1 | 2 |\n\n見 [文件][r]\n\n[r]: https://x.com \"標題\"\n";
        let b = split_blocks(md);
        assert_eq!(b.len(), 2, "{b:?}");
        assert!(b[0].2.starts_with("| a | b |") && b[0].2.contains("| 1 | 2 |"));
        assert!(b[1].2.contains("[r]: <https://x.com> \"標題\""), "{}", b[1].2);
    }

    #[test]
    fn test_quoted_table_is_unquoted_for_rendering() {
        let md = "> **說明**：段落\n>\n> | a | b |\n> | :--- | --- |\n> | 1 | 2 |\n";
        let inner = unquote_table_block(md).expect("quote with a table");
        assert_eq!(inner, "**說明**：段落\n\n| a | b |\n| :--- | --- |\n| 1 | 2 |");
        assert!(unquote_table_block("> 只有文字\n> 第二行\n").is_none());
        assert!(unquote_table_block("| a |\n|---|\n| 1 |\n").is_none());
    }

    #[test]
    fn test_split_blocks_footnotes_not_split() {
        let md = "文字[^1]\n\n[^1]: 註腳\n";
        assert_eq!(split_blocks(md), vec![(0, 3, md.to_string())]);
    }

    #[test]
    fn test_scroll_target_interpolates_between_anchors() {
        let a = [(0, 0.0), (10, 100.0), (20, 400.0)];
        assert_eq!(scroll_target(&a, 0, 0.0), Some(0.0));
        assert_eq!(scroll_target(&a, 5, 0.0), Some(50.0));
        assert_eq!(scroll_target(&a, 15, 0.0), Some(250.0));
        assert_eq!(scroll_target(&a, 5, 0.5), Some(55.0));
        // 超過最後一個錨點就停在最後錨點
        assert_eq!(scroll_target(&a, 99, 0.0), Some(400.0));
        assert_eq!(scroll_target(&[], 3, 0.0), None);
        // 文件開頭的空白行回到頂端，不停留在上一個捲動位置。
        assert_eq!(scroll_target(&[(2, 30.0)], 0, 0.0), Some(0.0));
    }

    #[test]
    fn test_source_position_handles_gaps_duplicate_anchors_and_boundaries() {
        let anchors = [(2, 30.0), (4, 90.0), (4, 120.0), (10, 300.0), (12, 300.0)];
        assert_eq!(source_position(&[], 0.0), None);
        assert_eq!(source_position(&anchors, 0.0), Some((0, 0.0)));
        assert_eq!(source_position(&anchors, 30.0), Some((2, 0.0)));
        assert_eq!(source_position(&anchors, 45.0), Some((2, 0.5)));
        assert_eq!(source_position(&anchors, 100.0), Some((4, 0.0)));
        assert_eq!(source_position(&anchors, 210.0), Some((7, 0.0)));
        assert_eq!(source_position(&anchors, 300.0), Some((12, 0.0)));
        assert_eq!(source_position(&anchors, 1000.0), Some((12, 0.0)));
    }

    #[test]
    fn test_tab_preview_reverse_scroll_and_reflow_keep_reading_position() {
        let text: String = (0..100).map(|i| format!("## Heading {i}\n\nParagraph {i}\n\n")).collect();
        let mut h = preview_harness(text, 0);
        h.state_mut().preview.set_placement(Placement::Tab);
        h.run_steps(8);
        // 分頁與並排使用相同 UI，直接送入內容區事件以驗證閱讀位置。
        h.hover_at(egui::pos2(250.0, 300.0));
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0.0, -600.0),
            phase: egui::TouchPhase::Move, source: egui::MouseWheelSource::Wheel, modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(30);
        assert!(h.state().top_line > 10, "獨立預覽也須回報來源位置");
        let source = h.state().preview.preview_source.expect("須保留預覽精確位置");
        let editor_line = h.state().top_line;
        let offset = h.state().preview.scroll_position.0;
        h.run_steps(10);
        assert!((h.state().preview.scroll_position.0 - offset).abs() < 0.1);
        h.state_mut().preview.zoom = 1.5;
        h.run_steps(5);
        let pv = &h.state().preview;
        let target = scroll_target(&pv.anchors, source.0, source.1).unwrap();
        assert!((pv.scroll_position.0 - target.min(pv.scroll_position.1)).abs() < 1.0, "重新排版後須保留預覽閱讀位置");
        assert_eq!(h.state().top_line, editor_line, "縮放不能當成使用者捲動而回送來源位置");
        h.set_size(egui::vec2(450.0, 600.0));
        h.run_steps(5);
        assert_eq!(h.state().top_line, editor_line, "調整視窗大小不能回送來源位置");
    }

    #[test]
    fn test_preview_scrollbar_drag_updates_source_and_stays_still_after_release() {
        use egui_kittest::kittest::Queryable;
        let text: String = (0..100).map(|i| format!("## Heading {i}\n\nParagraph {i}\n\n")).collect();
        let mut h = preview_harness(text, 0);
        h.run_steps(8);
        let bar = h.get_by_role(egui::accesskit::Role::ScrollBar).rect();
        let start = egui::pos2(bar.center().x, bar.top() + 10.0);
        let end = egui::pos2(bar.center().x, bar.center().y);
        h.hover_at(start);
        h.drag_at(start);
        h.run_steps(2);
        h.hover_at(end);
        h.run_steps(2);
        h.drop_at(end);
        h.run_steps(5);
        assert!(h.state().top_line > 100, "拖曳預覽捲軸須回送來源位置");
        let offset = h.state().preview.scroll_position.0;
        h.run_steps(10);
        assert!((h.state().preview.scroll_position.0 - offset).abs() < 0.1, "放開捲軸後不能被來源位置拉回");
    }

    #[test]
    fn test_render_with_palette_has_no_css_vars() {
        for p in [fermaid::Palette::light(), fermaid::Palette::dark()] {
            let opts = fermaid::RenderOptions { palette: Some(p), ..Default::default() };
            let r = fermaid::render("flowchart LR\n  A[開始] --> B[結束]", &opts).unwrap();
            assert!(!r.svg.contains("var(--"), "SVG 仍含 CSS 變數");
        }
    }

    /// 圖表載入器只放一個字型也能畫出文字：有字型時的點陣圖要比沒有字型時多出文字筆畫
    /// 帶查詢串的遠端 SVG 由 RemoteSvgLoader 點陣化；無查詢串的 .svg 不歸它管
    #[test]
    fn test_remote_svg_loader_handles_query_strings() {
        let ctx = egui::Context::default();
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="#f0f"/></svg>"##;
        let uri = "https://example.com/badge.svg?branch=main";
        ctx.include_bytes(uri.to_string(), svg.as_bytes().to_vec());
        RemoteSvgLoader::install(&ctx, None);
        match ctx.try_load_image(&uri.to_string(), SizeHint::default()).unwrap() {
            ImagePoll::Ready { image } => assert_eq!(image.size, [20, 10]),
            ImagePoll::Pending { .. } => panic!("本地 include_bytes 應同步點陣化"),
        }
        // 無查詢串的 .svg 交給內建 SvgLoader（本載入器拒載）
        let plain = "https://example.com/plain.svg";
        ctx.include_bytes(plain.to_string(), svg.as_bytes().to_vec());
        assert!(
            ctx.try_load_image(&plain.to_string(), SizeHint::default()).is_err(),
            "無查詢串的 .svg 不應由 RemoteSvgLoader 處理"
        );
    }

    #[test]
    fn test_diagram_loader_draws_text_with_single_font() {
        let Some(font) = crate::app::fonts::FontManager::new().diagram_font() else {
            return; // 測試環境沒有任何候選字型
        };
        let opts = fermaid::RenderOptions { palette: Some(fermaid::Palette::light()), ..Default::default() };
        let svg = fermaid::render("flowchart LR\n  A[開始 Start] --> B[結束]", &opts).unwrap().svg;
        let raster = |font| {
            let ctx = egui::Context::default();
            DiagramLoader::install(&ctx, font);
            let uri = format!("{DIAGRAM_URI_PREFIX}test.svg");
            ctx.include_bytes(uri.clone(), svg.clone().into_bytes());
            match ctx.try_load_image(&uri, SizeHint::default()).unwrap() {
                ImagePoll::Ready { image } => image,
                ImagePoll::Pending { .. } => panic!("應同步完成點陣化"),
            }
        };
        let with = raster(Some(font));
        let without = raster(None);
        assert_eq!(with.size, without.size);
        let diff = with.pixels.iter().zip(&without.pixels).filter(|(a, b)| a != b).count();
        assert!(diff > 100, "文字未繪製（差異像素 {diff}）");
    }
}
