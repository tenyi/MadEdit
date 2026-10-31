//! 語法突顯：syntect 增量解析與逐行快取

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use syntect::highlighting::{
    FontStyle, HighlightIterator, HighlightState, Highlighter, Theme, ThemeSet,
};
use syntect::parsing::{ParseState, ScopeStack, ScopeStackOp, SyntaxDefinition, SyntaxReference, SyntaxSet};

/// 純文字（不突顯）
pub const PLAIN_TEXT: &str = "Plain Text";

/// 每隔多少行保存一次解析狀態檢查點
const CHECKPOINT_INTERVAL: usize = 32;

/// 行位元組數達此值就在該行後保存檢查點（長行不再等 32 行才存，起算點貼著長行）
const CHECKPOINT_MIN_BYTES: usize = 4096;

/// 檢查點數量上限，超過時只保留整段界線的檢查點
const CHECKPOINT_CAP: usize = 8192;

/// 需要往前補解析的行數超過此值時，改從目標行附近重新起算（避免大檔跳轉時長時間卡住）
const MAX_CATCH_UP_LINES: usize = 20_000;

/// 超長行在要求完整結果（列印）時只解析開頭這麼多位元組，其餘以前景色顯示
const MAX_PARSE_LINE_BYTES: usize = 16 * 1024;

/// 互動突顯時，行位元組數超過此值就只解析「可見視窗」
///
/// syntect 的 `parse_line` 無法從行內中斷處接續，因此整行解析的成本是
/// 行長 × 語法規則數（實測約 3 µs/位元組）。行越長每按一鍵就越慢，唯一的
/// 出路是不要解析整行：畫面看得到的字元數是常數，成本也就變成常數。
const WINDOW_LINE_BYTES: usize = 2 * 1024;

/// 視窗大小（位元組）：涵蓋可見範圍再加上前後留白
const WINDOW_BYTES: usize = 1024;

/// 視窗起點的對齊格（位元組）：水平捲動在同一格內不重新解析
const WINDOW_ALIGN: usize = 256;

/// 視窗前緣留白：可見範圍之前先解析這麼多位元組
const WINDOW_BACKOFF: usize = 256;

/// 在視窗前緣尋找語句邊界（`;` `{` `}`）的最大回溯距離
const WINDOW_SNAP: usize = 64;

/// 單幀解析預算：超過就分時中斷，剩下的行先以前景色顯示、下幀續跑
const PARSE_BUDGET: Duration = Duration::from_millis(8);

/// 單一語法突顯區段
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightSpan {
    /// 本區段之前的字元數（超長行只解析可見視窗時 > 0，其餘為 0）；
    /// 這些字元不屬於任何區段，以前景色繪製
    pub skip_chars: usize,
    pub text: String,
    pub color_rgb: (u8, u8, u8),
    pub bold: bool,
    pub italic: bool,
}

impl HighlightSpan {
    pub fn plain(text: String, color_rgb: (u8, u8, u8)) -> Self {
        Self {
            skip_chars: 0,
            text,
            color_rgb,
            bold: false,
            italic: false,
        }
    }
}

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME_SET: OnceLock<ThemeSet> = OnceLock::new();

fn syntax_set() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(|| load_syntaxes(super::settings::Settings::path().as_deref().and_then(Path::parent)))
}

fn theme_set() -> &'static ThemeSet {
    THEME_SET.get_or_init(|| load_themes(super::settings::Settings::path().as_deref().and_then(Path::parent)))
}

/// Stable traversal makes duplicate-name overrides reproducible. Do not follow
/// symlinks: a settings-directory link must not cause a recursive cycle.
fn resource_files(root: &Path, extension: &str) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut folders = vec![root.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let entries = match std::fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(e) => {
                if e.kind() != std::io::ErrorKind::NotFound {
                    eprintln!("MadEdit: cannot read {}: {e}", folder.display());
                }
                continue;
            }
        };
        for entry in entries {
            match entry.and_then(|e| Ok((e.path(), e.file_type()?))) {
                Ok((path, kind)) if kind.is_dir() => folders.push(path),
                Ok((path, kind)) if kind.is_file() && path.extension().is_some_and(|e| e.eq_ignore_ascii_case(extension)) => files.push(path),
                Ok(_) => {}
                Err(e) => eprintln!("MadEdit: cannot read resource entry in {}: {e}", folder.display()),
            }
        }
    }
    files.sort();
    files
}

fn load_syntaxes(config_dir: Option<&Path>) -> SyntaxSet {
    let defaults = SyntaxSet::load_defaults_newlines();
    let Some(dir) = config_dir else { return defaults };
    let files = resource_files(&dir.join("syntaxes"), "sublime-syntax");
    if files.is_empty() {
        return defaults;
    }
    let mut builder = defaults.into_builder();
    for path in files {
        let loaded = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|text| SyntaxDefinition::load_from_str(&text, true, path.file_stem().and_then(|s| s.to_str())).map_err(|e| e.to_string()));
        match loaded {
            Ok(definition) => builder.add(definition),
            Err(e) => eprintln!("MadEdit: cannot load syntax {}: {e}", path.display()),
        }
    }
    builder.build()
}

fn load_themes(config_dir: Option<&Path>) -> ThemeSet {
    let mut themes = ThemeSet::load_defaults();
    if let Some(dir) = config_dir {
        for path in resource_files(&dir.join("themes"), "tmTheme") {
            match ThemeSet::get_theme(&path) {
                Ok(theme) => {
                    if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                        themes.themes.insert(name.to_owned(), theme);
                    }
                }
                Err(e) => eprintln!("MadEdit: cannot load theme {}: {e}", path.display()),
            }
        }
    }
    themes
}

/// 所有可選語法名稱（純文字在最前，其餘依字母排序）
pub fn syntax_names() -> Vec<String> {
    let mut names: Vec<String> = syntax_set()
        .syntaxes()
        .iter()
        .filter(|s| !s.hidden && s.name != PLAIN_TEXT)
        .map(|s| s.name.clone())
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup();
    names.insert(0, PLAIN_TEXT.to_string());
    names
}

/// 所有可選的配色主題名稱
pub fn theme_names() -> Vec<String> {
    theme_set().themes.keys().cloned().collect()
}

/// 依名稱取得主題；找不到時依深淺色回退到內建預設
fn theme_by_name(name: &str, is_dark: bool) -> &'static Theme {
    let themes = &theme_set().themes;
    themes
        .get(name)
        .or_else(|| themes.get(default_theme_name(is_dark)))
        .or_else(|| themes.values().next())
        .expect("syntect 內建主題集不可能為空")
}

/// 主題的編輯區配色
pub struct ThemeColors {
    pub foreground: (u8, u8, u8),
    pub background: (u8, u8, u8),
    pub selection: Option<(u8, u8, u8)>,
    pub line_highlight: Option<(u8, u8, u8)>,
}

/// 取得主題的前景、背景、選取與目前行顏色（主題未定義時依深淺色給預設值）
pub fn theme_colors(name: &str, is_dark: bool) -> ThemeColors {
    let s = &theme_by_name(name, is_dark).settings;
    let rgb = |c: Option<syntect::highlighting::Color>| c.map(|c| (c.r, c.g, c.b));
    ThemeColors {
        foreground: rgb(s.foreground).unwrap_or(if is_dark { (216, 222, 233) } else { (34, 34, 34) }),
        background: rgb(s.background).unwrap_or(if is_dark { (43, 48, 59) } else { (255, 255, 255) }),
        selection: rgb(s.selection),
        line_highlight: rgb(s.line_highlight),
    }
}

/// 深色與淺色的預設主題
pub fn default_theme_name(is_dark: bool) -> &'static str {
    if is_dark { "base16-ocean.dark" } else { "InspiredGitHub" }
}

/// 依名稱找語法，並處理 syntect 預設集沒有的語言（以相近語法代替）
fn find_syntax(name: &str) -> Option<&'static SyntaxReference> {
    let ps = syntax_set();
    if name == PLAIN_TEXT {
        return None;
    }
    ps.find_syntax_by_name(name).or_else(|| match name {
        "TypeScript" => ps.find_syntax_by_name("JavaScript"),
        "TOML" => ps.find_syntax_by_name("YAML"),
        "Shell" => ps.find_syntax_by_extension("sh"),
        _ => ps.find_syntax_by_token(name),
    })
}

/// 依檔案路徑偵測語法名稱（副檔名 → 檔名 → 第一行，例如 shebang）
pub fn detect_syntax(path: &Path, first_line: &str) -> String {
    detect_syntax_in(syntax_set(), path, first_line)
}

fn detect_syntax_in(ps: &SyntaxSet, path: &Path, first_line: &str) -> String {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if let Some(syntax) = ps.find_syntax_by_extension(file_name).or_else(|| ps.find_syntax_by_extension(&ext)) {
        return syntax.name.clone();
    }
    // syntect 預設集缺少或命名不同的副檔名
    let mapped = match ext.as_str() {
        "ts" | "mts" | "cts" | "tsx" => Some("JavaScript"),
        "toml" => Some("YAML"),
        "zsh" | "bash" => Some("Bourne Again Shell (bash)"),
        "kt" | "kts" => Some("Java"),
        "swift" => Some("Rust"),
        _ => None,
    };
    if let Some(name) = mapped.filter(|n| ps.find_syntax_by_name(n).is_some()) {
        return name.to_string();
    }
    ps.find_syntax_by_first_line(first_line)
        .map(|s| s.name.clone())
        .unwrap_or_else(|| PLAIN_TEXT.to_string())
}

/// 行註解與區塊註解符號（供「註解／取消註解」使用）
pub fn comment_tokens(syntax: &str) -> (Option<&'static str>, Option<(&'static str, &'static str)>) {
    let line = match syntax {
        "Python" | "Ruby" | "Perl" | "YAML" | "TOML" | "R" | "Makefile" | "Shell"
        | "Bourne Again Shell (bash)" | "Tcl" | "Ruby on Rails" | "Git Ignore" => Some("#"),
        "SQL" | "Lua" | "Haskell" | "Literate Haskell" => Some("--"),
        "Lisp" | "Clojure" => Some(";"),
        "LaTeX" | "TeX" | "Erlang" | "MATLAB" => Some("%"),
        "Batch File" => Some("REM "),
        "HTML" | "XML" | "Markdown" | "CSS" | "Plain Text" => None,
        _ => Some("//"),
    };
    let block = match syntax {
        "HTML" | "XML" | "Markdown" => Some(("<!--", "-->")),
        "CSS" | "C" | "C++" | "C#" | "Java" | "JavaScript" | "Rust" | "Go" | "Objective-C"
        | "Scala" | "PHP" | "D" | "Groovy" => Some(("/*", "*/")),
        _ => None,
    };
    (line, block)
}

/// 某一行要求的字元範圍 `(起, 迄)`；沒有視窗資訊（列印）時要求整行
fn view_range(windows: &[(usize, usize, usize)], line: usize) -> (usize, usize) {
    if windows.is_empty() {
        return (0, usize::MAX);
    }
    match windows.binary_search_by_key(&line, |w| w.0) {
        Ok(i) => (windows[i].1, windows[i].2),
        // 未列出的行：只知道行首
        Err(_) => (0, 0),
    }
}

/// 行內兩個字元索引 → 位元組位移（單次掃描；超出範圍時為行尾）
fn char_pair_to_byte(text: &str, a: usize, b: usize) -> (usize, usize) {
    let (a, b) = (a.min(b), a.max(b));
    let (mut ba, mut bb) = (text.len(), text.len());
    for (i, (off, _)) in text.char_indices().enumerate() {
        if i == a {
            ba = off;
        }
        if i >= b {
            bb = off;
            break;
        }
    }
    (ba, bb)
}

/// 對齊到前一個字元邊界（位元組位移可能落在多位元組字元中間）
fn snap_back(text: &str, mut i: usize) -> usize {
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// 對齊到後一個字元邊界
fn snap_forward(text: &str, mut i: usize) -> usize {
    while i < text.len() && !text.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// 可見視窗的位元組範圍
///
/// 起點對齊 [`WINDOW_ALIGN`] 格，水平捲動時同一格內不重新解析；再往前找最近的
/// 語句邊界（`;` `{` `}`）當起點，讓語法上下文盡量回到基底狀態——syntect 無法從
/// 行內任意位置接續，這是「只解析一段」唯一能降低誤差的作法。
///
/// 格線與位元組位移都不保證落在字元邊界上（`本` 之類的多位元組字元），
/// 因此兩端都要再對齊，否則切片會 panic。
fn window_range(text: &str, vis: (usize, usize)) -> (usize, usize) {
    let (vb_start, vb_end) = char_pair_to_byte(text, vis.0, vis.1);
    let base = vb_start.saturating_sub(WINDOW_BACKOFF) / WINDOW_ALIGN * WINDOW_ALIGN;
    let limit = vb_start.min(base + WINDOW_SNAP);
    let start = text.as_bytes()[base..limit]
        .iter()
        .rposition(|b| matches!(b, b';' | b'{' | b'}'))
        .map_or(base, |i| base + i + 1);
    // 起點只往前挪到邊界（不可超過可見範圍起點），終點取兩者的較大值
    let start = snap_forward(text, start).min(vb_start);
    let end = snap_back(text, start.saturating_add(WINDOW_BYTES).max(vb_end).min(text.len())).max(start);
    (start, end)
}

/// 由突顯器的輸出建立區段；整段同色時收斂成「整行以前景色顯示」的空區段
fn spans_of(
    hl: &mut HighlightState,
    ops: &[(usize, ScopeStackOp)],
    with_nl: &str,
    highlighter: &Highlighter,
    fg: (u8, u8, u8),
) -> Vec<HighlightSpan> {
    let mut spans = Vec::new();
    let mut uniform = true;
    for (style, piece) in HighlightIterator::new(hl, ops, with_nl, highlighter) {
        let piece = piece.trim_end_matches('\n');
        if piece.is_empty() {
            continue;
        }
        let c = style.foreground;
        let rgb = (c.r, c.g, c.b);
        let bold = style.font_style.contains(FontStyle::BOLD);
        let italic = style.font_style.contains(FontStyle::ITALIC);
        uniform &= rgb == fg && !bold && !italic;
        spans.push(HighlightSpan { skip_chars: 0, text: piece.to_string(), color_rgb: rgb, bold, italic });
    }
    if uniform {
        spans.clear();
        spans.push(HighlightSpan::plain(String::new(), fg));
    }
    spans
}

/// 只突顯超長行的一段視窗；視窗之前的字元記在首個區段的 `skip_chars`，以前景色繪製
fn highlight_window(
    text: &str,
    syntax: &SyntaxReference,
    highlighter: &Highlighter,
    vis: (usize, usize),
    fg: (u8, u8, u8),
) -> LineSpans {
    let (start, end) = window_range(text, vis);
    let skip = text[..start].chars().count();
    let slice = &text[start..end];
    let with_nl = format!("{slice}\n");
    let mut state = ParseState::new(syntax);
    let ops = state.parse_line(&with_nl, syntax_set()).unwrap_or_default();
    let mut hl = HighlightState::new(highlighter, ScopeStack::new());
    let mut spans = spans_of(&mut hl, &ops, &with_nl, highlighter, fg);
    let to = skip + slice.chars().count();
    let hash = crate::core::content_hash(text);
    if spans.len() == 1 && spans[0].text.is_empty() {
        // 整段同色：整行以前景色顯示，但仍只宣稱已解析到視窗結尾
        return LineSpans { spans: Rc::new(spans), from: 0, to, hash };
    }
    if skip > 0 {
        spans[0].skip_chars = skip;
    }
    LineSpans { spans: Rc::new(spans), from: skip, to, hash }
}

/// 單一檢查點：進入 `line` 行前的解析狀態（= 前一行結束後）
struct Checkpoint {
    line: usize,
    parse: ParseState,
    hl: HighlightState,
}

/// 一行的突顯結果，以及它涵蓋的字元範圍
struct LineSpans {
    spans: Rc<Vec<HighlightSpan>>,
    /// 涵蓋範圍（相對於行首的字元索引）；整行皆已解析時為 `0..usize::MAX`
    from: usize,
    to: usize,
    /// 行內容雜湊。視窗突顯的超長行靠它判斷內容變了沒：沒變就不重建視窗
    hash: u64,
}

impl LineSpans {
    /// 整行皆已解析
    fn whole(spans: Vec<HighlightSpan>) -> Self {
        Self { spans: Rc::new(spans), from: 0, to: usize::MAX, hash: 0 }
    }

    fn covers(&self, want: (usize, usize)) -> bool {
        self.from <= want.0 && self.to >= want.1
    }
}

/// 一份文件的語法突顯快取：保存解析狀態檢查點與已突顯的行
///
/// 編輯只標記 `dirty_from`，不丟棄後續行的快取；重解析時與編輯前留下的
/// 檢查點比對狀態，一致（收斂）即停止——典型編輯只需重解析被編輯的行。
pub struct SyntaxCache {
    syntax_name: String,
    theme_name: String,
    is_dark: bool,
    /// 依行號遞增；含編輯後仍未重驗證的「舊」檢查點（僅供收斂比對，不作為起算點）
    checkpoints: Vec<Checkpoint>,
    /// 自此行（含）起的快取不可信；None = 沒有失效
    dirty_from: Option<usize>,
    /// 分時解析中斷點：此行（含）之後尚未突顯；None = 沒有未完成的工作
    parse_frontier: Option<usize>,
    /// 單次 highlight 的解析預算（測試可調）；None = 不限時
    parse_budget: Option<Duration>,
    lines: HashMap<usize, LineSpans>,
}

impl Default for SyntaxCache {
    fn default() -> Self {
        Self {
            syntax_name: PLAIN_TEXT.to_string(),
            theme_name: String::new(),
            is_dark: false,
            checkpoints: Vec::new(),
            dirty_from: None,
            parse_frontier: None,
            parse_budget: Some(PARSE_BUDGET),
            lines: HashMap::new(),
        }
    }
}

impl SyntaxCache {
    /// 自第 `line` 行起（含）的快取失效
    ///
    /// 不立即重解析也不丟棄後續快取：保留舊檢查點供收斂比對，待下次
    /// [`Self::highlight`] 時才重解析受影響的範圍。
    pub fn invalidate_from(&mut self, line: usize) {
        self.dirty_from = Some(self.dirty_from.map_or(line, |d| d.min(line)));
    }

    /// Rebase cached line identities before invalidating parser state.
    pub fn lines_replaced(&mut self, start: usize, removed: usize, inserted: usize) {
        if removed != inserted {
            let old_end = start + removed;
            let shifted = |line: usize| line - removed + inserted;
            self.checkpoints.retain_mut(|c| {
                if c.line >= old_end {
                    c.line = shifted(c.line);
                    true
                } else {
                    c.line < start
                }
            });
            self.lines.retain(|&line, _| line < start || line >= old_end);
            // Remove all moving entries first, so shifted keys cannot overwrite
            // entries that have not moved yet. Span allocations remain Rc-shared.
            let moving: Vec<_> = self.lines.extract_if(|&line, _| line >= old_end).collect();
            self.lines.extend(moving.into_iter().map(|(line, spans)| (shifted(line), spans)));
        }
        self.invalidate_from(start);
        if let Some(frontier) = &mut self.parse_frontier {
            *frontier = (*frontier).min(start);
        }
    }

    /// 清除全部快取
    pub fn clear(&mut self) {
        self.checkpoints.clear();
        self.dirty_from = None;
        self.parse_frontier = None;
        self.lines.clear();
    }

    /// 檢查點數量上限；過多時只保留整段界線的檢查點
    fn trim_checkpoints(&mut self) {
        if self.checkpoints.len() > CHECKPOINT_CAP {
            self.checkpoints.retain(|c| c.line % CHECKPOINT_INTERVAL == 0);
        }
    }

    /// 文字預設前景色
    pub fn default_fg(&self) -> (u8, u8, u8) {
        theme_by_name(&self.theme_name, self.is_dark)
            .settings
            .foreground
            .map(|c| (c.r, c.g, c.b))
            .unwrap_or(if self.is_dark { (216, 222, 233) } else { (34, 34, 34) })
    }

    /// 取得 [first, first + count) 行的突顯結果；語法或主題改變時自動重建
    ///
    /// `windows` 是畫面可見行的字元範圍 `(行號, 起始字元, 結束字元)`（依行號排序）。
    /// 行長超過 [`WINDOW_LINE_BYTES`] 者只解析涵蓋該範圍的一段視窗，其餘字元以
    /// 前景色顯示；成本因此只與可見字元數有關，而與行長無關。
    ///
    /// 解析工作超過每幀預算時先回傳前景色暫替區段（[`Self::has_pending`] 為真，
    /// UI 應要求重繪續跑）；列印請改用 [`Self::highlight_full`]。
    #[allow(clippy::too_many_arguments)] // 參數與快取輸入一一對應，拆成結構只是多一層轉手
    pub fn highlight(
        &mut self,
        lines: &[String],
        first: usize,
        count: usize,
        syntax_name: &str,
        theme_name: &str,
        is_dark: bool,
        windows: &[(usize, usize, usize)],
    ) -> Vec<Rc<Vec<HighlightSpan>>> {
        self.highlight_inner(lines, first, count, syntax_name, theme_name, is_dark, self.parse_budget, windows)
    }

    /// 無預算上限的完整突顯（列印用）：會先跑完上次分時剩下的進度
    pub fn highlight_full(
        &mut self,
        lines: &[String],
        first: usize,
        count: usize,
        syntax_name: &str,
        theme_name: &str,
        is_dark: bool,
    ) -> Vec<Rc<Vec<HighlightSpan>>> {
        if self.parse_frontier.is_some() {
            self.highlight_inner(lines, 0, lines.len(), syntax_name, theme_name, is_dark, None, &[]);
        }
        self.highlight_inner(lines, first, count, syntax_name, theme_name, is_dark, None, &[])
    }

    /// 是否還有本輪未解析完的行（UI 應要求重繪以續跑）
    pub fn has_pending(&self) -> bool {
        self.parse_frontier.is_some()
    }

    #[allow(clippy::too_many_arguments)] // 同上；呼叫端只有 highlight 與 highlight_full
    fn highlight_inner(
        &mut self,
        lines: &[String],
        first: usize,
        count: usize,
        syntax_name: &str,
        theme_name: &str,
        is_dark: bool,
        budget: Option<Duration>,
        windows: &[(usize, usize, usize)],
    ) -> Vec<Rc<Vec<HighlightSpan>>> {
        if self.syntax_name != syntax_name || self.theme_name != theme_name || self.is_dark != is_dark {
            self.syntax_name = syntax_name.to_string();
            self.theme_name = theme_name.to_string();
            self.is_dark = is_dark;
            self.clear();
        }
        let end = (first + count).min(lines.len());
        if first >= end {
            return Vec::new();
        }
        let fg = self.default_fg();
        // 只有帶視窗資訊（畫面可見範圍）才做視窗突顯；列印與其他完整請求傳空切片
        let windowed = !windows.is_empty();
        let dirty = self.dirty_from;
        let pending = self.parse_frontier;
        let want = |l: usize| view_range(windows, l);
        // 快取已涵蓋要求的範圍（不判斷內容是否仍有效）
        let cached_ok = |cache: &HashMap<usize, LineSpans>, l: usize| {
            cache.get(&l).is_some_and(|c| c.covers(want(l)))
        };
        // 這一行是否仍需（重新）突顯
        let pending_line = |cache: &HashMap<usize, LineSpans>, l: usize| -> bool {
            let Some(c) = cache.get(&l) else {
                return true;
            };
            if windowed && lines[l].len() > WINDOW_LINE_BYTES {
                // 超長行只解析自己的視窗，與跨行狀態無關：內容或視窗範圍變了才重建。
                // 內容比對用雜湊，編輯別的行不會連帶重建（這才是分時預算的真正用途）。
                !c.covers(want(l)) || c.hash != crate::core::content_hash(&lines[l])
            } else {
                dirty.is_some_and(|d| l >= d) || !c.covers(want(l))
            }
        };
        let Some(syntax) = find_syntax(syntax_name) else {
            // 純文字：以空白文字區段代表整行前景色，並快取起來避免每幀複製整行
            self.parse_frontier = None;
            let mut out = Vec::with_capacity(end - first);
            for l in first..end {
                let cached = self.lines.entry(l).or_insert_with(|| LineSpans::whole(vec![HighlightSpan::plain(String::new(), fg)]));
                out.push(Rc::clone(&cached.spans));
            }
            return out;
        };
        if dirty.is_none()
            && pending.is_none_or(|f| f >= end)
            && (first..end).all(|l| cached_ok(&self.lines, l))
        {
            return (first..end).map(|l| Rc::clone(&self.lines[&l].spans)).collect();
        }

        // 逐行結果快取只需涵蓋最近看過的範圍，過大時清空避免記憶體無限成長
        if self.lines.len() > 5000 {
            self.lines.clear();
        }
        let theme = theme_by_name(theme_name, is_dark);
        let highlighter = Highlighter::new(theme);
        let ps = syntax_set();

        // 需要重建/建立突顯的起點：
        // - 有中斷點（frontier）時，其前的行都已完成，可視範圍在其下時直接沿用快取
        // - 有編輯失效（dirty）時，自 dirty（或可視範圍起點，取較小）起狀態都須重掃
        let work_from = match (dirty, self.parse_frontier) {
            (Some(d), Some(f)) => first.max(d.min(f)),
            (Some(d), None) => first.min(d),
            (None, Some(f)) => first.max(f),
            (None, None) => first,
        };
        // 起算點：進入 work_from 前最近的「有效」檢查點。行號 > dirty 的檢查點
        // 是編輯前留下的舊狀態，從它起算會與後面的舊檢查點形成假收斂，不可作為起點；
        // 沒有有效檢查點時由第 0 行或目標附近重新起算
        let cp_limit = work_from.min(dirty.unwrap_or(usize::MAX));
        let at = self.checkpoints.partition_point(|c| c.line <= cp_limit);
        let save_checkpoints = at > 0 || work_from <= MAX_CATCH_UP_LINES;
        let (mut parse, mut hl, mut line_no) = if at > 0 {
            let cp = &self.checkpoints[at - 1];
            (cp.parse.clone(), cp.hl.clone(), cp.line)
        } else {
            let start = if work_from <= MAX_CATCH_UP_LINES { 0 } else { work_from - work_from % CHECKPOINT_INTERVAL };
            (
                ParseState::new(syntax),
                HighlightState::new(&highlighter, ScopeStack::new()),
                start,
            )
        };
        if self.checkpoints.is_empty() && save_checkpoints {
            self.checkpoints.push(Checkpoint { line: 0, parse: parse.clone(), hl: hl.clone() });
        }

        let deadline = budget.map(|b| Instant::now() + b);
        let mut bytes_since = 0usize;
        let mut converged = None;
        let mut cut_at = None;
        while line_no < end {
            let text = &lines[line_no];
            // 超長行：只解析可見視窗。視窗解析有自己的起始狀態，與逐行狀態無關，
            // 因此這種行對「跨行解析狀態」是透明的：不消耗也不產生狀態。好處是
            // 在超長行內編輯不會連帶重解析它後面的行，且狀態仍然確定（可收斂）。
            if windowed && text.len() > WINDOW_LINE_BYTES {
                if line_no >= work_from && pending_line(&self.lines, line_no) {
                    let entry = highlight_window(text, syntax, &highlighter, want(line_no), fg);
                    self.lines.insert(line_no, entry);
                }
            } else {
                // 超長行（列印）只解析開頭（在 UTF-8 字元邊界截斷），其餘字元沿用前景色
                let (with_nl, prefix_only) = if text.len() <= MAX_PARSE_LINE_BYTES {
                    (format!("{}\n", text), false)
                } else {
                    let mut cut = MAX_PARSE_LINE_BYTES;
                    while !text.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    (format!("{}\n", &text[..cut]), true)
                };
                let ops = parse.parse_line(&with_nl, ps).unwrap_or_default();
                bytes_since += with_nl.len();
                // 需要重建的行（≥ work_from 且「已失效或沒有快取」）：產生並覆蓋快取；
                // 其餘只推進突顯狀態
                let stale = dirty.is_some_and(|d| line_no >= d);
                if line_no >= work_from && (stale || !self.lines.contains_key(&line_no)) {
                    // 超長行解析到的開頭就是「已解析範圍」上限（位元組數不會小於字元數，安全）
                    let to = if prefix_only { MAX_PARSE_LINE_BYTES } else { usize::MAX };
                    self.lines.insert(line_no, LineSpans { spans: Rc::new(spans_of(&mut hl, &ops, &with_nl, &highlighter, fg)), from: 0, to, hash: 0 });
                } else {
                    // 只推進突顯狀態，不產生區段
                    for _ in HighlightIterator::new(&mut hl, &ops, &with_nl, &highlighter) {}
                }
            }
            let next = line_no + 1;
            // 收斂判定：解析完 dirty 之後的行，若進入下一行的狀態與編輯前的
            // 檢查點相同，代表更後面的舊快取仍然正確，直接沿用並停止
            if dirty.is_none_or(|d| next > d) {
                let at = self.checkpoints.partition_point(|c| c.line < next);
                if let Some(c) = self.checkpoints.get(at).filter(|c| c.line == next) {
                    if c.parse == parse && c.hl == hl {
                        if (next..end).all(|l| cached_ok(&self.lines, l)) {
                            converged = Some(next);
                            break;
                        }
                    } else {
                        // 狀態已分歧：以此行後的新狀態取代舊檢查點
                        self.checkpoints[at] = Checkpoint { line: next, parse: parse.clone(), hl: hl.clone() };
                    }
                }
            }
            // 分時預算：超過就中斷。存下目前狀態當檢查點，下幀自 next 續跑；
            // 沒有剩餘工作（其餘行都有快取且未失效）就直接收工
            if let Some(deadline) = deadline
                && Instant::now() >= deadline
            {
                let rest_work = (next..end).find(|&l| pending_line(&self.lines, l));
                if rest_work.is_some() {
                    let at = self.checkpoints.partition_point(|c| c.line < next);
                    let exists = self.checkpoints.get(at).is_some_and(|c| c.line == next);
                    if !exists {
                        self.checkpoints.insert(at, Checkpoint { line: next, parse: parse.clone(), hl: hl.clone() });
                    }
                    cut_at = Some(next);
                }
                break;
            }
            if save_checkpoints {
                let at = self.checkpoints.partition_point(|c| c.line < next);
                let exists = self.checkpoints.get(at).is_some_and(|c| c.line == next);
                if !exists && (next % CHECKPOINT_INTERVAL == 0 || bytes_since >= CHECKPOINT_MIN_BYTES) {
                    self.checkpoints.insert(at, Checkpoint { line: next, parse: parse.clone(), hl: hl.clone() });
                    bytes_since = 0;
                    self.trim_checkpoints();
                }
            }
            line_no = next;
        }
        match converged {
            Some(_) => {
                // 收斂：next 起的舊快取仍然正確
                self.dirty_from = None;
                self.parse_frontier = None;
            }
            None => match cut_at {
                Some(next) => {
                    // 預算中斷：[work_from..next) 已重建；dirty 尚未驗證到收斂，
                    // 未重建的失效區仍不可信
                    self.dirty_from = dirty.map(|d| d.max(next));
                    self.parse_frontier = Some(next);
                }
                None => {
                    // 已處理到 end
                    self.parse_frontier = None;
                    self.dirty_from = (dirty.is_some_and(|d| end < d) && end < lines.len()).then_some(end);
                }
            },
        }
        // out 依可視範圍補齊：快取優先，沒有的行以前景色暫替
        let missing = Rc::new(vec![HighlightSpan::plain(String::new(), fg)]);
        let mut out = Vec::with_capacity(end - first);
        for l in first..end {
            out.push(self.lines.get(&l).map_or_else(|| Rc::clone(&missing), |c| Rc::clone(&c.spans)));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 反覆呼叫直到分時解析完成，回傳最後一次的完整結果
    fn drain(cache: &mut SyntaxCache, lines: &[String], first: usize, count: usize) -> Vec<Rc<Vec<HighlightSpan>>> {
        loop {
            let out = cache.highlight(lines, first, count, "Rust", "InspiredGitHub", false, &[]);
            if !cache.has_pending() {
                return out;
            }
        }
    }

    #[test]
    fn test_custom_syntax_and_theme_override_fallbacks_despite_bad_files() {
        let dir = crate::test_support::TempDir::new("custom-syntax-theme");
        let syntaxes = dir.path().join("syntaxes/nested");
        let themes = dir.path().join("themes");
        std::fs::create_dir_all(&syntaxes).unwrap();
        std::fs::create_dir_all(&themes).unwrap();
        std::fs::write(syntaxes.join("00-broken.sublime-syntax"), "contexts: [invalid").unwrap();
        std::fs::write(syntaxes.join("custom.sublime-syntax"), r#"%YAML 1.2
---
name: Custom Language
file_extensions: [ts, toml, CustomFile]
scope: source.custom
contexts:
  main:
    - match: '\bmagic\b'
      scope: keyword.control.custom
"#).unwrap();
        std::fs::write(themes.join("00-broken.tmTheme"), "not a plist").unwrap();
        std::fs::write(themes.join("Custom.tmTheme"), r##"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>name</key><string>Custom</string>
<key>settings</key><array>
<dict><key>settings</key><dict><key>foreground</key><string>#112233</string></dict></dict>
<dict><key>scope</key><string>keyword.control.custom</string><key>settings</key><dict>
<key>foreground</key><string>#dd2244</string><key>fontStyle</key><string>bold italic</string>
</dict></dict></array></dict></plist>"##).unwrap();
        let defaults = load_syntaxes(None);
        assert_eq!(detect_syntax_in(&defaults, Path::new("foo.ts"), ""), "JavaScript");
        let ps = load_syntaxes(Some(dir.path()));
        for file in ["foo.ts", "foo.toml", "CustomFile"] {
            assert_eq!(detect_syntax_in(&ps, Path::new(file), ""), "Custom Language");
        }
        let themes = load_themes(Some(dir.path()));
        let theme = &themes.themes["Custom"];
        let syntax = ps.find_syntax_by_name("Custom Language").unwrap();
        let mut hl = syntect::easy::HighlightLines::new(syntax, theme);
        let spans = hl.highlight_line("magic\n", &ps).unwrap();
        let style = spans.iter().find(|(_, s)| s.contains("magic")).unwrap().0;
        assert_eq!((style.foreground.r, style.foreground.g, style.foreground.b), (0xdd, 0x22, 0x44));
        assert!(style.font_style.contains(FontStyle::BOLD | FontStyle::ITALIC));
        assert!(themes.themes.contains_key("InspiredGitHub"));
    }

    #[test]
    fn test_missing_custom_resource_folders_keep_builtins() {
        let dir = crate::test_support::TempDir::new("no-custom-syntax");
        let ps = load_syntaxes(Some(dir.path()));
        assert_eq!(detect_syntax_in(&ps, Path::new("foo.ts"), ""), "JavaScript");
        assert_eq!(detect_syntax_in(&ps, Path::new("foo.swift"), ""), "Rust");
        assert_eq!(detect_syntax_in(&ps, Path::new("foo.rs"), ""), "Rust");
        assert!(load_themes(Some(dir.path())).themes.contains_key("InspiredGitHub"));
    }

    #[test]
    fn test_markdown_preserves_bold_and_italic_spans() {
        let lines = vec!["# Heading".into(), "**bold** and *italic*".into()];
        let spans = SyntaxCache::default().highlight_full(&lines, 0, lines.len(), "Markdown", "InspiredGitHub", false);
        assert!(spans[0].iter().any(|s| s.bold && s.text.contains("Heading")));
        assert!(spans[1].iter().any(|s| s.bold && s.text.contains("bold")));
        assert!(spans[1].iter().any(|s| s.italic && s.text.contains("italic")));
    }

    #[test]
    fn test_foreground_colored_styles_are_not_collapsed() {
        let mut theme = Theme::default();
        let color = syntect::highlighting::Color { r: 30, g: 40, b: 50, a: 255 };
        theme.settings.foreground = Some(color);
        theme.scopes.push(syntect::highlighting::ThemeItem {
            scope: "markup.heading".parse().unwrap(),
            style: syntect::highlighting::StyleModifier {
                foreground: Some(color),
                font_style: Some(FontStyle::BOLD | FontStyle::ITALIC),
                ..Default::default()
            },
        });
        let highlighter = Highlighter::new(&theme);
        let mut parse = ParseState::new(syntax_set().find_syntax_by_name("Markdown").unwrap());
        let mut hl = HighlightState::new(&highlighter, ScopeStack::new());
        let ops = parse.parse_line("# Heading\n", syntax_set()).unwrap();
        let spans = spans_of(&mut hl, &ops, "# Heading\n", &highlighter, (30, 40, 50));
        assert!(spans.iter().any(|s| s.bold && s.italic && s.text.contains("Heading")));
    }

    #[test]
    fn test_detect_syntax() {
        assert_eq!(detect_syntax(Path::new("main.rs"), ""), "Rust");
        assert_eq!(detect_syntax(Path::new("a.py"), ""), "Python");
        assert_eq!(detect_syntax(Path::new("script"), "#!/bin/bash"), "Bourne Again Shell (bash)");
        assert_eq!(detect_syntax(Path::new("notes.unknownext"), "hello"), PLAIN_TEXT);
    }

    #[test]
    fn test_long_line_only_parses_prefix() {
        // 沒有視窗資訊（例如列印）時，超長行只解析開頭，不應 panic，且區段文字總長不超過行長
        let long = format!("let s = \"{}\";", "x".repeat(MAX_PARSE_LINE_BYTES * 2));
        let lines = vec!["fn main() {".to_string(), long, "}".to_string()];
        let mut cache = SyntaxCache::default();
        let out = cache.highlight(&lines, 0, 3, "Rust", "InspiredGitHub", false, &[]);
        assert_eq!(out.len(), 3);
        let text_chars: usize = out[1].iter().map(|s| s.text.chars().count()).sum();
        assert!(text_chars <= MAX_PARSE_LINE_BYTES, "超長行只突顯開頭");
        assert!(text_chars <= lines[1].chars().count());
    }

    #[test]
    fn test_long_line_window_parses_only_visible_region() {
        // 160 KB 單行：只解析涵蓋可見範圍的 1 KB 視窗，成本與行長無關
        let lines = vec!["let a=1;".repeat(20_000)];
        let mut cache = SyntaxCache::default();
        let vis = (100_000, 100_100);
        let out = cache.highlight(&lines, 0, 1, "Rust", "InspiredGitHub", false, &[(0, vis.0, vis.1)]);
        assert!(!cache.has_pending());
        let spans = &out[0];
        let parsed: usize = spans.iter().map(|s| s.text.chars().count()).sum();
        assert!(parsed <= WINDOW_BYTES, "只解析視窗（{parsed} 字元），不得解析整行");
        assert!(spans[0].skip_chars > 0 && spans[0].skip_chars <= vis.0, "視窗之前的字元必須跳過");
        assert!(spans[0].skip_chars + parsed >= vis.1, "視窗必須涵蓋可見範圍");
        assert!(spans.iter().any(|s| s.color_rgb != cache.default_fg()), "可見範圍必須有語法著色");
        // 同一視窗再要一次：直接沿用快取
        let again = cache.highlight(&lines, 0, 1, "Rust", "InspiredGitHub", false, &[(0, vis.0, vis.1)]);
        assert!(Rc::ptr_eq(&out[0], &again[0]), "同一視窗不應重解析");
    }

    #[test]
    fn test_long_line_window_respects_char_boundaries() {
        // 含全形字的超長行：視窗起點的 256 位元組格線會落在多位元組字元中間
        // 一長串全形字（字串或註解）：64 位元組內找不到 `;` `{` `}`，
        // 視窗起點會退回 256 位元組格線，正好落在字元中間
        let line = format!("let s = \"{}\";", "中文內容測試".repeat(4000));
        let chars = line.chars().count();
        let lines = vec![line];
        let mut cache = SyntaxCache { parse_budget: None, ..SyntaxCache::default() };
        for start in (0..chars).step_by(61) {
            let vis = (start, (start + 40).min(chars));
            let out = cache.highlight(&lines, 0, 1, "Rust", "InspiredGitHub", false, &[(0, vis.0, vis.1)]);
            let spans = &out[0];
            // 單一空區段代表「整行同色」：可見範圍一定有顏色
            let (from, to) = if spans.len() == 1 && spans[0].text.is_empty() {
                (0, usize::MAX)
            } else {
                let from = spans[0].skip_chars;
                (from, from + spans.iter().map(|s| s.text.chars().count()).sum::<usize>())
            };
            assert!(from <= vis.0, "視窗必須涵蓋可見起點（start={start} vis={vis:?} from={from} to={to}）");
            assert!(to >= vis.1, "視窗必須涵蓋可見終點（start={start} vis={vis:?} from={from} to={to}）");
        }
    }

    #[test]
    fn test_long_line_window_follows_scroll() {
        // 視窗隨可見範圍移動：行首與行尾各自只解析附近，編輯後也只重建視窗
        let mut lines = vec!["let a=1;".repeat(20_000)];
        let mut cache = SyntaxCache::default();
        let head = cache.highlight(&lines, 0, 1, "Rust", "InspiredGitHub", false, &[(0, 0, 200)]);
        assert_eq!(head[0][0].skip_chars, 0, "行首視窗從第 0 字元起算");
        let tail = cache.highlight(&lines, 0, 1, "Rust", "InspiredGitHub", false, &[(0, 150_000, 150_200)]);
        assert!(tail[0][0].skip_chars > 149_000, "行尾視窗不重解析行首");
        assert!(tail[0][0].skip_chars <= 150_000);
        lines[0].insert(150_050, 'x');
        cache.invalidate_from(0);
        let after = cache.highlight(&lines, 0, 1, "Rust", "InspiredGitHub", false, &[(0, 150_000, 150_200)]);
        let parsed: usize = after[0].iter().map(|s| s.text.chars().count()).sum();
        assert!(parsed <= WINDOW_BYTES, "編輯後仍只解析視窗");
        assert!(after[0][0].skip_chars > 149_000);
        assert!(!cache.has_pending());
    }

    #[test]
    fn test_plain_text_spans_are_cached_and_empty() {
        let lines = vec!["just text".to_string(), "更多文字".to_string()];
        let mut cache = SyntaxCache::default();
        let first = cache.highlight(&lines, 0, 2, PLAIN_TEXT, "InspiredGitHub", false, &[]);
        assert_eq!(first.len(), 2);
        for line in &first {
            assert_eq!(line.len(), 1);
            assert!(line[0].text.is_empty(), "純文字以空白區段代表整行前景色");
        }
        // 第二次呼叫走快取，結果一致
        let again = cache.highlight(&lines, 0, 2, PLAIN_TEXT, "InspiredGitHub", false, &[]);
        assert_eq!(first, again);
    }

    #[test]
    fn test_multiline_comment_state_carries_across_lines() {
        let lines: Vec<String> = ["/* start", "still comment", "*/ fn main() {}"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut cache = SyntaxCache::default();
        // 只請求第 2 行，也必須知道它位於註解內（與從頭突顯的結果一致）
        let only_second = drain(&mut cache, &lines, 1, 1);
        let mut fresh = SyntaxCache::default();
        let all = drain(&mut fresh, &lines, 0, 3);
        assert_eq!(only_second[0], all[1]);
        // 註解顏色應不同於一般程式碼 fn 關鍵字
        assert_ne!(all[1][0].color_rgb, all[2].last().unwrap().color_rgb);
    }

    #[test]
    fn test_line_insert_and_delete_keeps_colors_aligned() {
        let mut lines: Vec<String> = (0..80)
            .map(|i| if i % 2 == 0 { format!("// c{i}") } else { format!("let v{i} = {i};") })
            .collect();
        let mut cache = SyntaxCache::default();
        drain(&mut cache, &lines, 0, lines.len());
        lines.insert(5, String::new());
        cache.lines_replaced(5, 0, 1);
        assert_eq!(
            drain(&mut cache, &lines, 0, lines.len()),
            drain(&mut SyntaxCache::default(), &lines, 0, lines.len())
        );
        lines.drain(10..12);
        cache.lines_replaced(10, 2, 0);
        assert_eq!(
            drain(&mut cache, &lines, 0, lines.len()),
            drain(&mut SyntaxCache::default(), &lines, 0, lines.len())
        );
    }

    #[test]
    fn test_line_insert_before_pending_frontier() {
        let mut lines: Vec<String> = (0..80).map(|i| format!("let v{i} = {i};")).collect();
        let mut cache = SyntaxCache { parse_budget: Some(Duration::ZERO), ..Default::default() };
        for _ in 0..8 {
            cache.highlight(&lines, 0, lines.len(), "Rust", "InspiredGitHub", false, &[]);
        }
        assert!(cache.has_pending());
        lines.insert(2, "/*".into());
        cache.lines_replaced(2, 0, 1);
        lines.insert(40, "*/".into());
        cache.lines_replaced(40, 0, 1);
        assert_eq!(
            drain(&mut cache, &lines, 12, lines.len() - 12),
            drain(&mut SyntaxCache::default(), &lines, 12, lines.len() - 12)
        );
    }

    #[test]
    fn test_invalidate_after_edit() {
        let mut lines: Vec<String> = vec!["let a = 1;".into(), "let b = 2;".into()];
        let mut cache = SyntaxCache::default();
        cache.highlight(&lines, 0, 2, "Rust", "InspiredGitHub", false, &[]);
        lines[1] = "// comment".into();
        cache.invalidate_from(1);
        let out = cache.highlight(&lines, 1, 1, "Rust", "InspiredGitHub", false, &[]);
        let text: String = out[0].iter().map(|s| s.text.as_str()).collect();
        assert_eq!(text, "// comment");
        assert_eq!(out[0][0].color_rgb, out[0].last().unwrap().color_rgb);
    }

    #[test]
    fn test_edit_converges_and_reuses_following_cache() {
        // 編輯不影響後續行狀態時，重解析到下一個檢查點即收斂，後續行沿用原快取（Rc 同源）
        // 行長 ≥ CHECKPOINT_MIN_BYTES：每行後都有檢查點，收斂即時發生
        let pad = "x".repeat(4096);
        let mut lines: Vec<String> = (0..6).map(|i| format!("let v{i} = {i}; // {pad}")).collect();
        let mut cache = SyntaxCache::default();
        let before = drain(&mut cache, &lines, 0, 6);
        lines[1].push('x');
        cache.invalidate_from(1);
        let after = drain(&mut cache, &lines, 1, 5);
        assert_eq!(after.len(), 5);
        let text: String = after[0].iter().map(|s| s.text.as_str()).collect();
        assert_eq!(text, lines[1], "編輯的行必須重建");
        assert!(Rc::ptr_eq(&before[2], &after[1]), "收斂後第 2 行必須沿用原快取");
        assert!(Rc::ptr_eq(&before[5], &after[4]), "收斂後第 5 行必須沿用原快取");
        assert_eq!(cache.dirty_from, None);
    }

    #[test]
    fn test_diverging_edit_rehighlights_following_lines() {
        // 在第 1 行打開區塊註解：後續行狀態分歧，必須重解析且顏色改變
        let pad = "x".repeat(4096);
        let mut lines: Vec<String> = (0..6).map(|i| format!("let v{i} = {i}; // {pad}")).collect();
        let mut cache = SyntaxCache::default();
        let before = drain(&mut cache, &lines, 0, 6);
        lines[1] = format!("/* let v1 // {pad}");
        cache.invalidate_from(1);
        let after = drain(&mut cache, &lines, 1, 5);
        assert!(!Rc::ptr_eq(&before[2], &after[1]), "註解狀態分歧，第 2 行必須重解析");
        assert_ne!(before[2][0].color_rgb, after[1].last().unwrap().color_rgb, "第 2 行應變為註解色");
        assert!(!Rc::ptr_eq(&before[3], &after[2]), "註解持續到結尾前都應重解析");
        // 收斂後與從頭重算的結果一致
        let mut fresh = SyntaxCache::default();
        let all = drain(&mut fresh, &lines, 0, 6);
        for i in 0..5 {
            assert_eq!(after[i], all[i + 1]);
        }
    }

    #[test]
    fn test_time_sliced_highlight_fills_progressively() {
        // 預算歸零：每幀只解析一行，未完成的行先以前景色暫替，最終結果與一次算完一致
        let pad = "x".repeat(4096);
        let lines: Vec<String> = (0..6).map(|i| format!("let v{i} = {i}; // {pad}")).collect();
        let mut cache = SyntaxCache { parse_budget: Some(Duration::ZERO), ..Default::default() };
        let mut frames = 0;
        let last = loop {
            assert!(frames < 20, "分時解析必須在有限幀內完成");
            let out = cache.highlight(&lines, 0, 6, "Rust", "InspiredGitHub", false, &[]);
            frames += 1;
            assert_eq!(out.len(), 6);
            if !cache.has_pending() {
                break out;
            }
        };
        let mut fresh = SyntaxCache::default();
        let all = drain(&mut fresh, &lines, 0, 6);
        for i in 0..6 {
            assert_eq!(last[i], all[i], "分時完成後應與一次算完一致");
        }
        assert!(frames > 1, "零預算必須分多幀");
    }

    #[test]
    fn test_diverging_edit_below_viewport_rebuilds_stale_lines() {
        // 編輯行在可視範圍之前且狀態分歧（未關閉的區塊註解）：可視範圍的行必須全部重解析
        let pad = "x".repeat(4096);
        let mut lines: Vec<String> = (0..8).map(|i| format!("let v{i} = {i}; // {pad}")).collect();
        let mut cache = SyntaxCache::default();
        let before = drain(&mut cache, &lines, 0, 8);
        lines[1] = format!("/* let v1 // {pad}");
        cache.invalidate_from(1);
        let after = drain(&mut cache, &lines, 4, 4);
        assert_eq!(after.len(), 4);
        for i in 0..4 {
            assert!(!Rc::ptr_eq(&before[4 + i], &after[i]), "註解分歧，第 {} 行必須重解析", 4 + i);
        }
        // 與從頭重算的結果一致
        let mut fresh = SyntaxCache::default();
        let all = drain(&mut fresh, &lines, 0, 8);
        for i in 0..4 {
            assert_eq!(after[i], all[4 + i]);
        }
    }

    #[test]
    fn test_syntax_names_contains_common_languages() {
        let names = syntax_names();
        assert_eq!(names[0], PLAIN_TEXT);
        for n in ["Rust", "Python", "C++", "JavaScript", "HTML"] {
            assert!(names.iter().any(|x| x == n), "{n}");
        }
    }
}
