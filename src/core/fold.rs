//! Code folding model: which line ranges the user has collapsed.
//!
//! Pure logic (no egui, no view state): brace/bracket-based fold detection for
//! C-like languages, the active fold list, and patience when the text changes. The
//! view layer maps document rows to display rows in `app/view/foldmap.rs`.
//!
//! The module is self-contained on purpose: deleting it plus its two call sites
//! removes code folding without touching the editor.

use std::fmt;

/// A collapsed range: `header` stays visible, `header + 1 ..= end` is hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FoldRange {
    pub header: usize,
    pub end: usize,
}

impl FoldRange {
    pub fn new(header: usize, end: usize) -> Self {
        Self { header, end }
    }

    /// Number of hidden lines (always >= 1 for a valid range).
    pub fn hidden_lines(&self) -> usize {
        self.end.saturating_sub(self.header)
    }
}

impl fmt::Display for FoldRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..={}", self.header, self.end)
    }
}

/// How a language marks comments and strings for the brace scanner.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CommentStyle {
    /// `//`, `/* … */`, backslash escapes, C++ raw strings.
    CLike,
    /// `#`, `<# … #>`, backtick escapes and `@"…"@` heredocs.
    PowerShell,
}

/// What kind of folding a syntax supports.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FoldKind {
    Braces(CommentStyle),
    /// Python-style blocks: a line ending in `:` folds the more-indented lines.
    Indent,
}

/// Folding rule for a syntax name, or `None` when the syntax has no folding.
pub fn fold_kind(name: &str) -> Option<FoldKind> {
    match name {
        "Python" => return Some(FoldKind::Indent),
        "PowerShell" => return Some(FoldKind::Braces(CommentStyle::PowerShell)),
        "C" | "C++" | "C#" | "Java" | "JavaScript" | "TypeScript" | "Go" | "Rust" | "PHP"
        | "Squirrel" | "D" | "Groovy" | "Scala" | "Objective-C" | "JSON" | "CSS" => {
            return Some(FoldKind::Braces(CommentStyle::CLike));
        }
        _ => {}
    }
    None
}

/// Syntaxes whose braces or indentation can be folded.
#[cfg(test)]
pub fn is_foldable_syntax(name: &str) -> bool {
    fold_kind(name).is_some()
}

/// What a detected fold looks like from the gutter's point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FoldMarker {
    /// The fold starts on this line.
    pub folded: bool,
    /// Hidden line count when folded.
    pub hidden: usize,
    /// It can be expanded but is currently not (or not folded at all).
    pub foldable: bool,
}

/// Key for the detection cache: the same document version, syntax and line count
/// always produce the same candidate list.
#[derive(Clone, PartialEq, Eq)]
struct DetectKey {
    version: u64,
    syntax: String,
    lines: usize,
}

/// Active folds plus the lazily detected candidate ranges.
#[derive(Default)]
pub struct FoldModel {
    /// Currently collapsed ranges, sorted by header, never overlapping.
    active: Vec<FoldRange>,
    /// Every foldable brace range in the document, sorted by header.
    candidates: Vec<FoldRange>,
    detected: Option<DetectKey>,
}

impl FoldModel {
    // ── Queries ─────────────────────────────────────────────────────────────

    pub fn active(&self) -> &[FoldRange] {
        &self.active
    }

    #[cfg(test)]
    pub fn candidates(&self) -> &[FoldRange] {
        &self.candidates
    }

    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }

    /// The collapsed range that hides `line`, if any.
    pub fn header_of(&self, line: usize) -> Option<FoldRange> {
        self.active.iter().copied().find(|r| r.header < line && line <= r.end)
    }

    /// Active fold that starts exactly at `header`.
    pub fn range_at(&self, header: usize) -> Option<FoldRange> {
        self.active.iter().copied().find(|r| r.header == header)
    }

    /// The innermost foldable range containing `line` (its header is above the
    /// line). Used when the caret sits inside a block instead of on its `{` line.
    pub fn enclosing_header(&self, line: usize) -> Option<usize> {
        self.candidates
            .iter()
            .filter(|r| r.header < line && line <= r.end)
            .map(|r| r.header)
            .max()
    }

    /// Marker state for the gutter of `line`.
    pub fn marker(&self, line: usize) -> Option<FoldMarker> {
        if let Some(r) = self.range_at(line) {
            return Some(FoldMarker { folded: true, hidden: r.hidden_lines(), foldable: true });
        }
        self.candidates
            .iter()
            .find(|r| r.header == line)
            .map(|r| FoldMarker { folded: false, hidden: r.hidden_lines(), foldable: true })
    }

    // ── Detection ───────────────────────────────────────────────────────────

    /// Recompute the candidate ranges when the document changed.
    ///
    /// Folds whose range no longer exists are dropped; existing folds keep their
    /// state, so editing inside a folded file does not pop everything open.
    pub fn refresh(&mut self, lines: &[String], syntax: &str, version: u64) {
        let key = DetectKey { version, syntax: syntax.to_string(), lines: lines.len() };
        if self.detected.as_ref() == Some(&key) {
            return;
        }
        let candidates = match fold_kind(syntax) {
            Some(FoldKind::Braces(style)) => detect_folds_with(lines, style),
            Some(FoldKind::Indent) => detect_indent_folds(lines),
            None => Vec::new(),
        };
        // Keep active folds that still correspond to a detected range.
        self.active.retain(|a| candidates.iter().any(|c| *c == *a));
        self.candidates = candidates;
        self.detected = Some(key);
        self.clamp(lines.len());
    }

    /// Forget the detection cache (settings toggled, syntax changed by hand).
    pub fn clear(&mut self) {
        self.active.clear();
        self.candidates.clear();
        self.detected = None;
    }

    // ── Toggling ────────────────────────────────────────────────────────────

    /// Toggle the fold starting at `header`.
    ///
    /// Returns `Some(true)` when it became folded, `Some(false)` when expanded,
    /// and `None` when the line has no foldable range at all.
    pub fn toggle(&mut self, header: usize) -> Option<bool> {
        if let Some(pos) = self.active.iter().position(|r| r.header == header) {
            self.active.remove(pos);
            return Some(false);
        }
        let candidate = *self.candidates.iter().find(|r| r.header == header)?;
        // Collapsing a range absorbs folds nested inside it.
        self.active.retain(|r| !(r.header >= candidate.header && r.end <= candidate.end));
        self.active.push(candidate);
        self.active.sort_by_key(|r| r.header);
        Some(true)
    }

    /// Expand the fold starting at `header`, if any.
    pub fn unfold_at(&mut self, header: usize) -> bool {
        let before = self.active.len();
        self.active.retain(|r| r.header != header);
        self.active.len() != before
    }

    /// Collapse every outermost detected range.
    pub fn fold_all(&mut self) {
        let mut active: Vec<FoldRange> = Vec::new();
        for c in &self.candidates {
            let nested = active.last().is_some_and(|outer| c.header <= outer.end);
            if !nested {
                active.push(*c);
            }
        }
        self.active = active;
    }

    pub fn unfold_all(&mut self) {
        self.active.clear();
    }

    // ── Keeping folds aligned with edits ────────────────────────────────────

    /// Shift or drop folds after a line-level edit.
    ///
    /// `at` is the first affected line, `delta` the net line-count change
    /// (positive when lines were inserted). Ranges intersecting the edit are
    /// dropped: their boundaries no longer describe the text.
    pub fn on_line_delta(&mut self, at: usize, delta: isize) {
        if delta == 0 {
            return;
        }
        let shift = |r: &mut FoldRange| -> bool {
            if r.end < at {
                return true;
            }
            if r.header >= at {
                let h = r.header as isize + delta;
                let e = r.end as isize + delta;
                if h < 0 || e < h {
                    return false;
                }
                r.header = h as usize;
                r.end = e as usize;
                return true;
            }
            // The edit starts inside the range: its header or body changed.
            false
        };
        self.active.retain_mut(shift);
        self.candidates.retain_mut(shift);
        self.detected = None;
    }

    /// Drop ranges that no longer fit the document.
    pub fn clamp(&mut self, line_count: usize) {
        self.active.retain(|r| r.header + 1 < line_count && r.end < line_count && r.end > r.header);
        self.candidates.retain(|r| r.header + 1 < line_count && r.end < line_count && r.end > r.header);
    }
}

/// Find brace-based fold ranges (`{` line .. `}` line) for C-like files,
/// ignoring braces inside strings, character literals and comments.
#[cfg(test)]
pub fn detect_braces(lines: &[String]) -> Vec<FoldRange> {
    let mut braces = scan(lines, CommentStyle::CLike).0;
    braces.sort_by_key(|r| r.header);
    braces
}

/// Bracket (`[` … `]`) fold ranges for C-like files.
#[cfg(test)]
pub fn detect_brackets(lines: &[String]) -> Vec<FoldRange> {
    let mut brackets = scan(lines, CommentStyle::CLike).1;
    brackets.sort_by_key(|r| r.header);
    brackets
}

/// Every foldable range for C-like files: braces plus multi-line comments
/// (`/* ... */` and runs of consecutive `//` lines).
#[cfg(test)]
pub fn detect_folds(lines: &[String]) -> Vec<FoldRange> {
    detect_folds_with(lines, CommentStyle::CLike)
}

/// Brace and comment folds for the given comment/string style.
pub fn detect_folds_with(lines: &[String], style: CommentStyle) -> Vec<FoldRange> {
    let (mut ranges, mut brackets, mut comments) = scan(lines, style);
    ranges.append(&mut brackets);
    ranges.append(&mut comments);
    ranges.sort_by_key(|r| r.header);
    ranges
}

/// Python-style folding: a line whose code ends with `:` folds the lines below
/// it that are indented further, plus runs of `#` comment lines, plus multi-line
/// `[ … ]` array literals (a line that leaves a bracket open folds until it closes).
pub fn detect_indent_folds(lines: &[String]) -> Vec<FoldRange> {
    let mut out = Vec::new();
    let indents: Vec<Option<usize>> = lines.iter().map(|line| indent_width(line)).collect();
    for (i, line) in lines.iter().enumerate() {
        let Some(base) = indents[i] else { continue };
        if !python_opens_block(line) {
            continue;
        }
        let mut end = None;
        for (j, indent) in indents.iter().enumerate().skip(i + 1) {
            match indent {
                Some(ind) if *ind > base => end = Some(j),
                Some(_) => break,
                // Blank lines never end a block (they may carry its indentation).
                None => {}
            }
        }
        if let Some(end) = end {
            out.push(FoldRange::new(i, end));
        }
    }
    // Runs of `#` comment lines (two or more) fold like `//` runs.
    let mut run: Option<(usize, usize)> = None;
    for (i, line) in lines.iter().enumerate() {
        let is_comment = line.trim_start().starts_with('#');
        if is_comment {
            let start = run.map_or(i, |(start, _)| start);
            run = Some((start, i));
        } else if let Some((start, last)) = run.take()
            && last > start
        {
            out.push(FoldRange::new(start, last));
        }
    }
    if let Some((start, last)) = run
        && last > start
    {
        out.push(FoldRange::new(start, last));
    }
    // 多行 `[ … ]` 陣列字面值：一行結束時仍有未閉合的 `[`，折到閉合的那一行。
    out.extend(detect_bracket_blocks(lines));
    out.sort_by_key(|r| r.header);
    out
}

/// Multi-line `[ … ]` blocks: a line that ends with an unclosed `[` folds until
/// the line whose last `]` brings the depth back to zero. Brackets inside
/// strings and `#` comments are ignored. Only spans of 2+ lines produce a fold.
pub fn detect_bracket_blocks(lines: &[String]) -> Vec<FoldRange> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut header: Option<usize> = None;
    for (line_no, line) in lines.iter().enumerate() {
        let code = strip_python_noise(line);
        let mut quote: Option<char> = None;
        let mut escaped = false;
        for c in code.chars() {
            if escaped {
                escaped = false;
                continue;
            }
            match quote {
                Some(q) => {
                    if c == '\\' {
                        escaped = true;
                    } else if c == q {
                        quote = None;
                    }
                }
                None => match c {
                    '"' | '\'' => quote = Some(c),
                    '[' => depth += 1,
                    ']' if depth > 0 => depth -= 1,
                    _ => {}
                },
            }
        }
        if depth > 0 {
            if header.is_none() {
                header = Some(line_no);
            }
        } else if let Some(start) = header.take()
            && line_no > start
        {
            out.push(FoldRange::new(start, line_no));
        }
    }
    out
}

/// Strip a `#` comment from a Python line (string-aware enough for folding:
/// a `#` inside a quoted string is kept, one outside is treated as a comment).
fn strip_python_noise(line: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (idx, c) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match quote {
            Some(q) => {
                if c == '\\' {
                    escaped = true;
                } else if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                '#' => return &line[..idx],
                _ => {}
            },
        }
    }
    line
}

/// Leading indentation width; `None` for blank lines. Tabs count as four columns.
fn indent_width(line: &str) -> Option<usize> {
    let mut width = 0usize;
    for c in line.chars() {
        match c {
            ' ' => width += 1,
            '\t' => width += 4,
            _ => break,
        }
    }
    (!line.trim().is_empty()).then_some(width)
}

/// Does this Python line open a block? Code ending in `:` (a trailing `#` comment
/// is ignored, string contents are not parsed).
fn python_opens_block(line: &str) -> bool {
    let code = line.split('#').next().unwrap_or("");
    code.trim_end().ends_with(':')
}

/// One pass over the text: brace pairs, block comments and line-comment runs.
///
/// This is a deliberately small scanner: it understands what C-like and
/// PowerShell files use for folding, not the full grammar. Unbalanced braces
/// and unterminated comments simply produce no fold.
fn scan(lines: &[String], style: CommentStyle) -> (Vec<FoldRange>, Vec<FoldRange>, Vec<FoldRange>) {
    let mut braces = Vec::new();
    let mut brackets = Vec::new();
    let mut comments = Vec::new();
    let mut open: Vec<usize> = Vec::new();
    let mut open_brk: Vec<usize> = Vec::new();
    let mut in_block_comment = false;
    let mut block_open: Option<usize> = None;
    // C++ raw string: `R"delim( ... )delim"` may span lines and contains raw braces.
    let mut raw: Option<String> = None;
    let mut raw_matched = 0usize;
    // PowerShell here-string terminator quote (`@"`/`@'`); `None` when not inside one.
    let mut here: Option<char> = None;
    // Run of consecutive line comments (a blank line or code line ends it).
    let mut run_start: Option<usize> = None;
    let mut run_last: Option<usize> = None;
    for (line_no, line) in lines.iter().enumerate() {
        if let Some(quote) = here {
            if line.trim_start().starts_with(&format!("{quote}@")) {
                here = None;
            }
            if let (Some(start), Some(last)) = (run_start.take(), run_last.take())
                && last > start
            {
                comments.push(FoldRange::new(start, last));
            }
            continue;
        }
        let mut chars = line.chars().peekable();
        let mut quote: Option<char> = None;
        let mut first_nonspace = true;
        let mut starts_line_comment = false;
        while let Some(c) = chars.next() {
            if let Some(pattern) = &raw {
                first_nonspace = false;
                // Match `)delim"` one character at a time; braces inside are ignored.
                let expected = pattern.chars().nth(raw_matched);
                if expected == Some(c) {
                    raw_matched += 1;
                    if raw_matched == pattern.chars().count() {
                        raw = None;
                        raw_matched = 0;
                    }
                } else {
                    raw_matched = usize::from(c == ')');
                }
                continue;
            }
            if in_block_comment {
                first_nonspace = false;
                let closes = match style {
                    CommentStyle::CLike => c == '*' && chars.peek() == Some(&'/'),
                    CommentStyle::PowerShell => c == '#' && chars.peek() == Some(&'>'),
                };
                if closes {
                    chars.next();
                    in_block_comment = false;
                    if let Some(header) = block_open.take()
                        && line_no > header
                    {
                        comments.push(FoldRange::new(header, line_no));
                    }
                }
                continue;
            }
            if let Some(q) = quote {
                first_nonspace = false;
                match style {
                    CommentStyle::CLike => {
                        if c == '\\' {
                            chars.next();
                        } else if c == q {
                            quote = None;
                        }
                    }
                    CommentStyle::PowerShell => {
                        if q == '"' && c == '`' {
                            chars.next(); // `"` escapes the next character
                        } else if q == '\'' && c == '\'' && chars.peek() == Some(&'\'') {
                            chars.next(); // '' is an escaped quote
                        } else if c == q {
                            quote = None;
                        }
                    }
                }
                continue;
            }
            // Line comment marker: `//` for C-like, `#` for PowerShell.
            let line_marker = match style {
                CommentStyle::CLike => c == '/' && chars.peek() == Some(&'/'),
                CommentStyle::PowerShell => c == '#',
            };
            if line_marker {
                if first_nonspace {
                    starts_line_comment = true;
                }
                break;
            }
            // Block comment opener: `/*` or `<#`.
            let block_marker = match style {
                CommentStyle::CLike => c == '/' && chars.peek() == Some(&'*'),
                CommentStyle::PowerShell => c == '<' && chars.peek() == Some(&'#'),
            };
            if block_marker {
                first_nonspace = false;
                chars.next();
                in_block_comment = true;
                block_open = Some(line_no);
                continue;
            }
            // C++11 raw string literal: R"delim( ... )delim"
            if style == CommentStyle::CLike && c == 'R' && chars.peek() == Some(&'"') {
                first_nonspace = false;
                chars.next();
                let mut delim = String::new();
                let mut ok = false;
                while let Some(&next) = chars.peek() {
                    if next == '(' {
                        chars.next();
                        ok = true;
                        break;
                    }
                    if delim.len() >= 16 || next.is_whitespace() || next == '\\' || next == ')' {
                        break;
                    }
                    delim.push(next);
                    chars.next();
                }
                if ok {
                    raw = Some(format!("){delim}\""));
                    raw_matched = 0;
                }
                continue;
            }
            // PowerShell here-string opener: `@"` or `@'` must end the line.
            if style == CommentStyle::PowerShell && c == '@' {
                let Some(&q) = chars.peek() else { first_nonspace = false; continue };
                if q == '"' || q == '\'' {
                    chars.next();
                    if chars.clone().all(|ch| ch.is_whitespace()) {
                        here = Some(q);
                        break;
                    }
                }
                first_nonspace = false;
                continue;
            }
            match c {
                '"' | '\'' => {
                    first_nonspace = false;
                    quote = Some(c);
                }
                '{' => {
                    first_nonspace = false;
                    open.push(line_no);
                }
                '}' => {
                    first_nonspace = false;
                    if let Some(header) = open.pop()
                        && line_no > header
                    {
                        braces.push(FoldRange::new(header, line_no));
                    }
                }
                '[' => {
                    first_nonspace = false;
                    open_brk.push(line_no);
                }
                ']' => {
                    first_nonspace = false;
                    if let Some(header) = open_brk.pop()
                        && line_no > header
                    {
                        brackets.push(FoldRange::new(header, line_no));
                    }
                }
                c if c.is_whitespace() => {}
                _ => first_nonspace = false,
            }
        }
        // Close/extend the line-comment run. Blank lines break it, as do code
        // lines with a trailing comment (the comment is not the first token there).
        if starts_line_comment {
            run_start.get_or_insert(line_no);
            run_last = Some(line_no);
        } else if let (Some(start), Some(last)) = (run_start.take(), run_last.take())
            && last > start
        {
            comments.push(FoldRange::new(start, last));
        }
    }
    if let (Some(start), Some(last)) = (run_start, run_last)
        && last > start
    {
        comments.push(FoldRange::new(start, last));
    }
    (braces, brackets, comments)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(|l| l.to_string()).collect()
    }

    #[test]
    fn test_detects_brace_block() {
        let src = lines("void f() {\n    int x = 1;\n}\n\nafter();");
        assert_eq!(detect_braces(&src), vec![FoldRange::new(0, 2)]);
    }

    #[test]
    fn test_ignores_braces_in_strings_and_comments() {
        let src = lines(
            "local s = \"{\";\n// {\n/* {\n still comment }\n*/\nfunction f() {\n    print(\"}\");\n}",
        );
        let ranges = detect_braces(&src);
        assert_eq!(ranges, vec![FoldRange::new(5, 7)], "only the function body folds");
    }

    #[test]
    fn test_ignores_braces_in_char_literals() {
        let src = lines("char c = '{';\nchar d = '}';\nvoid g() {\n}");
        assert_eq!(detect_braces(&src), vec![FoldRange::new(2, 3)]);
    }

    #[test]
    fn test_nested_candidates_and_fold_all_keeps_outermost() {
        let src = lines("a {\n    b {\n        c;\n    }\n}");
        let mut model = FoldModel::default();
        model.refresh(&src, "Squirrel", 1);
        assert_eq!(model.candidates().len(), 2, "nested braces both foldable");
        model.fold_all();
        assert_eq!(model.active(), &[FoldRange::new(0, 4)], "fold all keeps the outer range only");
        assert!(model.header_of(2).is_some());
    }

    #[test]
    fn test_toggle_and_nested_absorption() {
        let src = lines("a {\n    b {\n    }\n}");
        let mut model = FoldModel::default();
        model.refresh(&src, "Squirrel", 1);
        assert_eq!(model.toggle(1), Some(true), "inner block folds");
        assert_eq!(model.active(), &[FoldRange::new(1, 2)]);
        assert_eq!(model.toggle(0), Some(true), "outer block folds and absorbs the inner one");
        assert_eq!(model.active(), &[FoldRange::new(0, 3)]);
        assert_eq!(model.toggle(0), Some(false), "outer block expands");
        assert!(model.active().is_empty());
        assert_eq!(model.toggle(9), None, "no fold starts there");
    }

    #[test]
    fn test_non_foldable_syntax_has_no_candidates() {
        let src = lines("# heading\nsome text");
        let mut model = FoldModel::default();
        model.refresh(&src, "Markdown", 1);
        assert!(model.candidates().is_empty());
        assert_eq!(model.marker(0), None);
    }

    #[test]
    fn test_marker_reports_folded_state() {
        let src = lines("a {\n}\nb {\n}");
        let mut model = FoldModel::default();
        model.refresh(&src, "C", 1);
        assert_eq!(model.marker(0), Some(FoldMarker { folded: false, hidden: 1, foldable: true }));
        model.toggle(0);
        assert_eq!(model.marker(0), Some(FoldMarker { folded: true, hidden: 1, foldable: true }));
        assert_eq!(model.marker(1), None, "the hidden body line has no marker");
    }

    #[test]
    fn test_edit_shifts_folds_after_the_change() {
        let src = lines("a {\n}\nb {\n}");
        let mut model = FoldModel::default();
        model.refresh(&src, "C", 1);
        model.toggle(2);
        model.on_line_delta(2, 3); // three lines inserted before the second block
        model.refresh(&lines("a {\n}\nx\ny\nz\nb {\n}"), "C", 2);
        assert_eq!(model.active(), &[FoldRange::new(5, 6)], "fold moved with the text");
    }

    #[test]
    fn test_edit_inside_a_fold_drops_it() {
        let src = lines("a {\n    body;\n}");
        let mut model = FoldModel::default();
        model.refresh(&src, "C", 1);
        model.toggle(0);
        model.on_line_delta(1, 1); // new line inserted inside the body
        model.refresh(&lines("a {\n    body;\n    more;\n}"), "C", 2);
        assert!(model.active().is_empty(), "the range no longer describes the block");
        assert!(!model.candidates().is_empty());
    }

    #[test]
    fn test_clamp_drops_ranges_past_the_end() {
        let src = lines("a {\n}");
        let mut model = FoldModel::default();
        model.refresh(&src, "C", 1);
        model.toggle(0);
        model.clamp(1);
        assert!(model.active().is_empty());
    }

    #[test]
    fn test_ignores_braces_in_raw_strings() {
        // C++ raw strings may span lines and contain unbalanced braces.
        let src = lines("auto s = R\"(\n{ not code\n})\";\nvoid f() {\n}");
        assert_eq!(detect_braces(&src), vec![FoldRange::new(3, 4)]);
        // Custom delimiters end the literal only at `)tag"`.
        let src = lines("auto s = R\"xx( }) )x )xx\";\nvoid g() {\n}");
        assert_eq!(detect_braces(&src), vec![FoldRange::new(1, 2)]);
    }

    #[test]
    fn test_lone_brace_line_is_foldable() {
        // Regression shape: blank line, signature, `{` on its own line (a.cpp).
        let mut src = vec![String::new(), "\t\tvoid simplify_polygon(Polygon_2& p)".into(), "\t\t{".into()];
        for i in 0..16 {
            src.push(format!("\t\t\tauto x{i} = {i};"));
        }
        src.push("\t\t}".into());
        let ranges = detect_braces(&src);
        assert_eq!(ranges, vec![FoldRange::new(2, 19)], "the lone brace on line 3 folds");
    }

    #[test]
    fn test_block_comments_fold_across_lines() {
        let src = lines("a;\n/*\n comment\n*/\nb;\n/** doc\n * more\n */");
        assert_eq!(
            detect_folds(&src),
            vec![FoldRange::new(1, 3), FoldRange::new(5, 7)],
            "multi-line block comments fold, single-line ones do not"
        );
        // A one-line block comment has nothing to hide.
        let src = lines("a; /* x */ b;");
        assert!(detect_folds(&src).is_empty());
    }

    #[test]
    fn test_line_comment_runs_fold() {
        let src = lines("// one\n// two\n// three\ncode();\n// tail");
        assert_eq!(detect_folds(&src), vec![FoldRange::new(0, 2)], "a run of three // lines folds");
        // Blank lines split the run; a single comment line on its own does not fold.
        let src = lines("// a\n// b\n\n// c\nx;\n// d");
        assert_eq!(detect_folds(&src), vec![FoldRange::new(0, 1)]);
    }

    #[test]
    fn test_trailing_comments_and_strings_do_not_start_runs() {
        let src = lines("x = 1; // trailing\ny = 2; // trailing\nz = \"http://example\";\n// alone");
        assert!(detect_folds(&src).is_empty(), "trailing comments and URL strings are not runs");
        // A code line between two comment lines splits the run.
        let src = lines("// a\ncode();\n// b\n// c");
        assert_eq!(detect_folds(&src), vec![FoldRange::new(2, 3)]);
    }

    #[test]
    fn test_line_comments_inside_block_comments_are_not_runs() {
        let src = lines("/*\n// not a run\n// still inside\n*/\n// a\n// b");
        let ranges = detect_folds(&src);
        assert!(ranges.contains(&FoldRange::new(0, 3)), "{ranges:?}");
        assert!(ranges.contains(&FoldRange::new(4, 5)), "{ranges:?}");
        assert_eq!(ranges.len(), 2, "no extra runs from inside the block comment");
    }

    #[test]
    fn test_commented_out_braces_do_not_fold_as_code() {
        let src = lines("// function {\n/* {\n} */\nvoid f() {\n}");
        assert_eq!(detect_braces(&src), vec![FoldRange::new(3, 4)]);
    }

    #[test]
    fn test_python_indent_folds() {
        let src = lines(
            "import os\n\ndef outer(a):\n    if a:\n        step()\n    return a\n\nclass C:\n    def m(self):\n        pass\n",
        );
        let ranges = detect_indent_folds(&src);
        assert_eq!(ranges, vec![FoldRange::new(2, 5), FoldRange::new(3, 4), FoldRange::new(7, 9), FoldRange::new(8, 9)]);
        // A colon inside a dict (not at the end of the code) does not open a block.
        let src = lines("d = {'a': 1}\nif x:  # trailing comment\n    y()\n");
        assert_eq!(detect_indent_folds(&src), vec![FoldRange::new(1, 2)]);
        // Blank lines inside a block do not end it; a dedent does.
        let src = lines("def f():\n    a()\n\n    b()\nc()\n");
        assert_eq!(detect_indent_folds(&src), vec![FoldRange::new(0, 3)]);
    }

    #[test]
    fn test_python_comment_runs_fold() {
        let src = lines("# a\n# b\n\n# single\nx = 1\n# c\n# d\n# e");
        assert_eq!(detect_indent_folds(&src), vec![FoldRange::new(0, 1), FoldRange::new(5, 7)]);
    }

    #[test]
    fn test_json_arrays_fold_across_lines() {
        // 頂層陣列可折疊；同一行內閉合的 [] 不折疊
        let src = lines("[\n  1,\n  [2, 3],\n  4\n]\nx");
        assert_eq!(detect_folds(&src), vec![FoldRange::new(0, 4)]);
        let src = lines("let v = [\n    1,\n    2,\n];\nuse_it(v);");
        assert_eq!(detect_brackets(&src), vec![FoldRange::new(0, 3)], "only multi-line brackets fold");
        // 字串與註解裡的 ] 不參與配對
        let src = lines("let s = \"]\";\n// ]\nlet v = [\n    1,\n];");
        assert_eq!(detect_brackets(&src), vec![FoldRange::new(2, 4)]);
    }

    #[test]
    fn test_python_bracket_blocks_fold() {
        // 多行陣列字面值折疊；字串與 # 註解裡的 [] 被忽略
        let src = lines(
            "a = [\n    1,\n    \"[not folded]\",\n    2,  # ] inside comment\n]\nb = [1, 2]\nrun()",
        );
        let ranges = detect_indent_folds(&src);
        assert!(ranges.contains(&FoldRange::new(0, 4)), "{ranges:?}");
    }

    #[test]
    fn test_python_bracket_nested_and_unterminated() {
        let src = lines("m = [\n    [1,\n    2],\n]\nend()");
        assert_eq!(detect_bracket_blocks(&src), vec![FoldRange::new(0, 3)]);
        // 未閉合的 [ 不產生摺疊
        let src = lines("a = [1,\n2\n");
        assert!(detect_bracket_blocks(&src).is_empty());
    }

    #[test]
    fn test_powershell_braces_comments_and_here_strings() {
        let src = lines(
            "function Test {\n    # { comment\n    Write-Host \"{\"\n}\n$s = @\"\n{ not a brace\n\"@\nfunction B {\n}\n",
        );
        assert_eq!(
            detect_folds_with(&src, CommentStyle::PowerShell),
            vec![FoldRange::new(0, 3), FoldRange::new(7, 8)],
            "braces inside comments, strings and here-strings are ignored"
        );
        let src = lines("<#\n{ ignored\n#>\nfunction C {\n}\n");
        assert_eq!(
            detect_folds_with(&src, CommentStyle::PowerShell),
            vec![FoldRange::new(0, 2), FoldRange::new(3, 4)]
        );
        let src = lines("# one\n# two\nGet-Date\n");
        assert_eq!(detect_folds_with(&src, CommentStyle::PowerShell), vec![FoldRange::new(0, 1)]);
    }

    #[test]
    fn test_foldable_syntax_list() {
        for name in ["C", "C++", "C#", "Java", "JavaScript", "Go", "Rust", "PHP", "Squirrel", "JSON", "Python", "PowerShell"] {
            assert!(is_foldable_syntax(name), "{name}");
        }
        for name in ["Markdown", "Plain Text", "YAML", "INI"] {
            assert!(!is_foldable_syntax(name), "{name}");
        }
    }
}
