//! 列印與預覽：
//! - 文件先轉成與 UI 無關的列印內容（[`PrintContent`]），
//! - 列印時轉為 HTML 交給系統瀏覽器（自動開啟瀏覽器的列印對話框），
//! - 預覽列印則由 App 內的獨立視窗自行分頁繪製（見 `preview.rs`）。

use std::path::PathBuf;
use std::rc::Rc;

use crate::core::document::Document;
use crate::core::hexbuf::BYTES_PER_ROW;
use crate::core::model::EditMode;
use crate::core::settings::PrintOptions;
use crate::core::syntax::SyntaxCache;

/// 頁首頁尾中「頁碼」的佔位字元（私用區字元，預覽時逐頁代換）
pub const PAGE_NO_MARK: char = '\u{E000}';
/// 頁首頁尾中「總頁數」的佔位字元
pub const PAGE_TOTAL_MARK: char = '\u{E001}';

/// 列印內容中一段文字的種類（決定顏色）
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpanKind {
    /// 一般文字；`Some` 為語法突顯色，`None` 為預設黑色
    Text(Option<(u8, u8, u8)>),
    /// 行號
    LineNumber,
    /// 書籤標記
    Bookmark,
    /// 控制字元標記（Tab、空白、行尾）
    Control,
}

/// 列印內容中的一段文字
#[derive(Debug, Clone, PartialEq)]
pub struct PrintSpan {
    pub text: String,
    pub kind: SpanKind,
}

/// 列印內容中的一個邏輯行（可能在頁面上自動換成多列）
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PrintLine {
    pub spans: Vec<PrintSpan>,
}

impl PrintLine {
    /// 加入一段文字（空字串略過）
    fn push(&mut self, text: impl Into<String>, kind: SpanKind) {
        let text = text.into();
        if !text.is_empty() {
            self.spans.push(PrintSpan { text, kind });
        }
    }

    /// 加入一段內文，依選項把 Tab、空白拆出控制字元標記
    fn push_body(&mut self, text: &str, color: Option<(u8, u8, u8)>, opts: &PrintOptions) {
        // 目前累積中的一般文字
        let mut buf = String::new();
        for ch in text.chars() {
            if ch == '\t' && opts.tab_chars {
                self.push(std::mem::take(&mut buf), SpanKind::Text(color));
                self.push("»", SpanKind::Control);
                buf.push('\t');
            } else if ch == ' ' && opts.space_chars {
                self.push(std::mem::take(&mut buf), SpanKind::Text(color));
                self.push("·", SpanKind::Control);
            } else {
                buf.push(ch);
            }
        }
        self.push(buf, SpanKind::Text(color));
    }
}

/// 與 UI 無關的列印內容（HTML 列印與 App 內預覽共用）
#[derive(Debug, Clone, Default)]
pub struct PrintContent {
    /// 十六進位模式的位移標頭列（未啟用時為 `None`）
    pub hex_header: Option<String>,
    /// 位移標頭是否每頁重複（否則僅第一頁）
    pub hex_header_every_page: bool,
    /// 內文各行
    pub lines: Vec<PrintLine>,
    /// 頁首三欄（左、中、右），已展開除頁碼外的變數；未啟用頁首時為 `None`
    pub header: Option<[String; 3]>,
    /// 頁尾三欄（左、中、右）
    pub footer: Option<[String; 3]>,
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// 展開頁首頁尾變數：%f 檔名、%p 完整路徑、%n 頁碼、%s 總頁數、%d 日期、%t 時間
///
/// %n、%s 以佔位字元 [`PAGE_NO_MARK`]、[`PAGE_TOTAL_MARK`] 代入，由輸出端（HTML 或預覽）再行代換。
fn expand_vars(fmt: &str, doc: &Document) -> String {
    let now = chrono::Local::now();
    let name = doc.display_name();
    let path = doc.path.as_deref().map(crate::core::display_path).unwrap_or_else(|| name.clone());
    fmt.replace("%f", &name)
        .replace("%p", &path)
        .replace("%n", &PAGE_NO_MARK.to_string())
        .replace("%s", &PAGE_TOTAL_MARK.to_string())
        .replace("%d", &now.format("%Y-%m-%d").to_string())
        .replace("%t", &now.format("%H:%M:%S").to_string())
}

/// 把文件轉為列印內容
pub fn build_content(doc: &Document, opts: &PrintOptions) -> PrintContent {
    let mut content = PrintContent::default();
    if doc.mode == EditMode::Hex
        && let Some(hb) = &doc.hex
    {
        if opts.hex_offset_header > 0 {
            content.hex_header = Some("Offset    00 01 02 03 04 05 06 07  08 09 0A 0B 0C 0D 0E 0F  Text".to_string());
            content.hex_header_every_page = opts.hex_offset_header == 2;
        }
        // 十六進位模式最多列印 20 萬列，避免超大檔案耗盡記憶體
        let rows = hb.row_count().min(200_000);
        for r in 0..rows {
            let bytes = hb.data.read_slice(r * BYTES_PER_ROW, BYTES_PER_ROW);
            if bytes.is_empty() {
                break;
            }
            let hex: Vec<String> = bytes.iter().enumerate().map(|(i, b)| format!("{}{b:02X}", if i == 8 { " " } else { "" })).collect();
            let text: String = bytes.iter().map(|&b| if (0x20..0x7F).contains(&b) { b as char } else { '.' }).collect();
            let mut line = PrintLine::default();
            line.push(format!("{:08X}  {:<49} {}", r * BYTES_PER_ROW, hex.join(" "), text), SpanKind::Text(None));
            content.lines.push(line);
        }
    } else {
        let lines = &doc.editor.lines;
        let mut cache = SyntaxCache::default();
        let theme = crate::core::syntax::default_theme_name(false);
        // 行號位數（右對齊用）
        let digits = lines.len().to_string().len();
        for (i, text) in lines.iter().enumerate() {
            let mut line = PrintLine::default();
            if opts.line_numbers {
                line.push(format!("{:>w$}", i + 1, w = digits), SpanKind::LineNumber);
                line.push(" ", SpanKind::Text(None));
            }
            if opts.bookmarks && doc.editor.bookmarks.contains(&i) {
                line.push("●", SpanKind::Bookmark);
            }
            let spans = if opts.syntax_colors {
                cache.highlight_full(lines, i, 1, &doc.syntax, theme, false).pop().unwrap_or_default()
            } else {
                Rc::new(Vec::new())
            };
            if spans.is_empty() {
                line.push_body(text, None, opts);
            } else {
                for sp in spans.iter() {
                    // 空白文字區段代表整行同色（純文字檔的快取表示法）
                    if sp.text.is_empty() {
                        line.push_body(text, Some(sp.color_rgb), opts);
                    } else {
                        line.push_body(&sp.text, Some(sp.color_rgb), opts);
                    }
                }
            }
            if opts.end_of_line {
                line.push("↵", SpanKind::Control);
            }
            content.lines.push(line);
        }
    }
    if opts.header {
        content.header = Some([
            expand_vars(&opts.header_left, doc),
            expand_vars(&opts.header_center, doc),
            expand_vars(&opts.header_right, doc),
        ]);
    }
    if opts.footer {
        content.footer = Some([
            expand_vars(&opts.footer_left, doc),
            expand_vars(&opts.footer_center, doc),
            expand_vars(&opts.footer_right, doc),
        ]);
    }
    content
}

/// 頁首／頁尾的三欄 HTML 表格；頁碼由瀏覽器列印時以 CSS counter 產生
fn header_footer_html(cols: &[String; 3]) -> String {
    let cell = |s: &str| {
        escape(s)
            .replace(PAGE_NO_MARK, "<span class=\"pn\"></span>")
            .replace(PAGE_TOTAL_MARK, "<span class=\"pt\"></span>")
    };
    format!(
        "<table class=\"hf\"><tr><td>{}</td><td style=\"text-align:center\">{}</td><td style=\"text-align:right\">{}</td></tr></table>",
        cell(&cols[0]),
        cell(&cols[1]),
        cell(&cols[2])
    )
}

/// 產生列印用 HTML
pub fn document_html(doc: &Document, opts: &PrintOptions, font_family: &str, font_size: f32, auto_print: bool) -> String {
    let content = build_content(doc, opts);
    let mut body = String::new();
    // 位移標頭：每頁重複時放進 thead（瀏覽器會在每頁重印），否則只放在內文開頭
    let hex_hdr = content.hex_header.as_ref().map(|h| format!("<div class=\"hdr\">{}</div>", escape(h)));
    if !content.hex_header_every_page
        && let Some(h) = &hex_hdr
    {
        body.push_str(h);
    }
    for line in &content.lines {
        let mut html = String::new();
        for sp in &line.spans {
            let t = escape(&sp.text);
            match sp.kind {
                SpanKind::Text(None) => html.push_str(&t),
                SpanKind::Text(Some((r, g, b))) => html.push_str(&format!("<span style=\"color:#{r:02x}{g:02x}{b:02x}\">{t}</span>")),
                SpanKind::LineNumber => html.push_str(&format!("<span class=\"ln\">{t}</span>")),
                SpanKind::Bookmark => html.push_str(&format!("<span class=\"bm\">{t}</span>")),
                SpanKind::Control => html.push_str(&format!("<span class=\"cc\">{t}</span>")),
            }
        }
        body.push_str(&format!("<div class=\"l\">{html}</div>\n"));
    }
    let mut header = content.header.as_ref().map(header_footer_html).unwrap_or_default();
    if content.hex_header_every_page
        && let Some(h) = &hex_hdr
    {
        header.push_str(h);
    }
    let footer = content.footer.as_ref().map(header_footer_html).unwrap_or_default();
    let font = if font_family.is_empty() { "monospace".to_string() } else { format!("'{}', monospace", font_family.replace('\'', "")) };
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><title>{title}</title><style>
body {{ font-family: {font}; font-size: {size}px; margin: 0; }}
.l {{ white-space: pre-wrap; word-break: break-all; tab-size: {tab}; }}
.ln {{ color: #999; }} .bm {{ color: #3c78e6; }} .cc {{ color: #aac; }}
.hf {{ width: 100%; font-family: sans-serif; font-size: 10px; color: #555; }}
.hdr {{ white-space: pre; color: #36c; }}
thead {{ display: table-header-group; }} tfoot {{ display: table-footer-group; }}
@page {{ margin: 15mm; }}
body {{ counter-reset: page; }}
.pn::after {{ content: counter(page); }} .pt::after {{ content: counter(pages); }}
</style></head><body{onload}>
<table style="width:100%;border-collapse:collapse"><thead><tr><td>{header}</td></tr></thead>
<tfoot><tr><td>{footer}</td></tr></tfoot><tbody><tr><td>{body}</td></tr></tbody></table>
</body></html>"#,
        title = escape(&doc.display_name()),
        font = font,
        size = font_size,
        tab = crate::core::editor::tab_size(),
        onload = if auto_print { " onload=\"window.print()\"" } else { "" },
        header = header,
        footer = footer,
        body = body,
    )
}

/// 將 HTML 寫入暫存檔並以系統瀏覽器開啟
pub fn open_html(name: &str, html: &str) -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join("MadEdit");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let safe: String = name.chars().map(|c| if c.is_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect();
    let path = dir.join(format!("{safe}.html"));
    std::fs::write(&path, html).map_err(|e| e.to_string())?;
    open_with_system(&path)?;
    Ok(path)
}

/// 以系統預設程式開啟檔案
pub fn open_with_system(path: &std::path::Path) -> Result<(), String> {
    let result = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(path).spawn()
    } else if cfg!(windows) {
        std::process::Command::new("cmd").args(["/C", "start", ""]).arg(path).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(path).spawn()
    };
    result.map(|_| ()).map_err(|e| format!("無法開啟瀏覽器：{e}"))
}

/// Markdown 或 HTML 預覽頁
pub fn preview_html(doc: &Document, markdown: bool) -> String {
    let text = doc.text();
    if markdown {
        format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><title>{}</title><style>body{{font-family:sans-serif;max-width:860px;margin:2em auto;line-height:1.6;padding:0 1em}}pre,code{{background:#f4f4f4}}pre{{padding:.6em;overflow:auto}}table{{border-collapse:collapse}}td,th{{border:1px solid #ccc;padding:4px 8px}}</style></head><body>{}</body></html>",
            escape(&doc.display_name()),
            crate::core::textops::markdown_to_html(&text)
        )
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_document_html_escapes_and_numbers_lines() {
        let mut doc = Document::default();
        doc.editor.set_text("a<b\nc");
        let opts = PrintOptions { syntax_colors: false, header: false, footer: false, ..Default::default() };
        let html = document_html(&doc, &opts, "", 12.0, true);
        assert!(html.contains("a&lt;b"));
        assert!(html.contains("<span class=\"ln\">1</span>"));
        assert!(html.contains("window.print()"));
    }
}
