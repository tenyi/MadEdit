//! Text search: pattern compilation, line-oriented and whole-document matching, replacement.
//!
//! Everything here is independent of the UI. Two matching engines share one navigation
//! contract ([`find_in_lines`] and [`find_in_spans`]): a pattern that can match a line break
//! is matched on the whole document joined into one text ([`JoinedText`]), every other
//! pattern line by line, which never copies the document. The app owns the caches
//! (`app::find::LiveSearch`) and translates [`SearchError`] when it is shown.

use std::fmt;
use std::ops::Range;

use regex::{Captures, Regex, RegexBuilder};

use super::editor::{char_to_byte, EditorState, TextPosition};

/// 搜尋匹配選項
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SearchOptions {
    pub match_case: bool,
    pub whole_word: bool,
    pub use_regex: bool,
}

impl SearchOptions {
    pub fn new(match_case: bool, whole_word: bool, use_regex: bool) -> Self {
        Self { match_case, whole_word, use_regex }
    }
}

/// Why a search could not run. The core reports only the kind of failure; the app
/// translates it when it is shown, so no user-facing text lives here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchError {
    /// The query is empty.
    EmptyQuery,
    /// The regular expression did not compile. The `regex` crate's message explains why.
    InvalidRegex(String),
    /// The pattern is valid but too large for the regex engine's size limit.
    PatternTooLarge,
    /// A hex query is empty.
    EmptyHexQuery,
    /// A hex query contains a character that is not an ASCII hex digit.
    HexDigit(char),
    /// A hex query has an odd number of digits, so its last one is not a whole byte.
    OddHexLength,
}

impl fmt::Display for SearchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyQuery => write!(f, "the search text is empty"),
            Self::InvalidRegex(msg) => write!(f, "invalid regular expression: {msg}"),
            Self::PatternTooLarge => write!(f, "the search text is too long to build a pattern from"),
            Self::EmptyHexQuery => write!(f, "the hex pattern is empty"),
            Self::HexDigit(c) => write!(f, "invalid hex character: {c}"),
            Self::OddHexLength => write!(f, "the hex pattern must have an even number of digits"),
        }
    }
}

impl std::error::Error for SearchError {}

/// 解析十六進位搜尋字串（支援如 "48 65 6C 6C 6F" 或 "48656C6C6F" 或 "0x48 0x65"）
pub fn parse_hex_query(query: &str) -> Result<Vec<u8>, SearchError> {
    let clean = query.replace("0x", "").replace("0X", "").replace([' ', '\t', '\r', '\n', ',', '-'], "");

    if clean.is_empty() {
        return Err(SearchError::EmptyHexQuery);
    }
    // 先確認全為 ASCII 十六進位數字：之後以位元組切片取兩字一組才安全（中文等多位元組字元會切在字元中間而 panic）
    if let Some(c) = clean.chars().find(|c| !c.is_ascii_hexdigit()) {
        return Err(SearchError::HexDigit(c));
    }
    if !clean.len().is_multiple_of(2) {
        return Err(SearchError::OddHexLength);
    }

    let mut bytes = Vec::with_capacity(clean.len() / 2);
    for i in (0..clean.len()).step_by(2) {
        bytes.push(u8::from_str_radix(&clean[i..i + 2], 16).expect("every pair was checked to be ASCII hex digits"));
    }
    Ok(bytes)
}

/// 建構比對用之正規表達式物件。非正則查詢以字面比對（特殊字元全部逸出）。
pub fn build_regex(query: &str, options: &SearchOptions) -> Result<Regex, SearchError> {
    if query.is_empty() {
        return Err(SearchError::EmptyQuery);
    }
    let pattern = if options.use_regex { query.to_string() } else { regex::escape(query) };
    let pattern_str = if options.whole_word { format!(r"\b(?:{pattern})\b") } else { pattern };

    RegexBuilder::new(&pattern_str)
        .case_insensitive(!options.match_case)
        .multi_line(true)
        .build()
        .map_err(|e| match e {
            regex::Error::CompiledTooBig(_) => SearchError::PatternTooLarge,
            other => SearchError::InvalidRegex(other.to_string()),
        })
}

/// A pattern that can only match by crossing a line break: it contains a literal newline, or
/// (as a regular expression) the `\n` escape, but not the literal text `\\n`. Line by line
/// such a pattern finds nothing, so it is matched on [`JoinedText`] instead.
///
/// Other expressions that also accept a line break (`\s`, `[^x]`, `\W`) deliberately stay
/// within a line: routing them through the joined copy would copy the whole document for
/// everyday patterns such as `a\s*=\s*b`.
pub fn pattern_may_span_lines(query: &str, use_regex: bool) -> bool {
    let bytes = query.as_bytes();
    if bytes.contains(&b'\n') {
        return true;
    }
    // `n` right after a backslash that is not itself escaped
    use_regex
        && bytes.iter().enumerate().any(|(i, &c)| {
            c == b'n' && i > 0 && bytes[i - 1] == b'\\' && bytes[..i - 1].iter().rev().take_while(|&&b| b == b'\\').count() % 2 == 0
        })
}

/// One line's non-empty matches as `(start column, end column)`, counted in characters.
fn line_spans<'a>(line: &'a str, re: &'a Regex) -> impl Iterator<Item = (usize, usize)> + 'a {
    re.find_iter(line)
        .filter(|m| m.start() != m.end())
        .map(move |m| (line[..m.start()].chars().count(), line[..m.end()].chars().count()))
}

/// 在行陣列中尋找下一個（或上一個）符合的位置（逐行比對，不跨行）。
///
/// Forward: the first match that starts at or after `from`. Backward: the last match that
/// ends at or before `from`. With `wrap`, a search that runs off its end continues from the
/// other end. Empty matches are skipped. [`find_in_spans`] keeps the same contract for
/// patterns that cross line breaks.
pub fn find_in_lines(ed: &EditorState, re: &Regex, from: TextPosition, backward: bool, wrap: bool) -> Option<(TextPosition, TextPosition)> {
    let n = ed.lines.len();
    if n == 0 {
        return None;
    }
    let last = from.line.min(n - 1);
    let found = |li: usize, (s, e): (usize, usize)| Some((TextPosition::new(li, s), TextPosition::new(li, e)));
    if !backward {
        for li in from.line..n {
            let min_col = if li == from.line { from.col } else { 0 };
            if let Some(span) = line_spans(&ed.lines[li], re).find(|&(s, _)| s >= min_col) {
                return found(li, span);
            }
        }
        if wrap {
            for li in 0..=last {
                if let Some(span) = line_spans(&ed.lines[li], re).next() {
                    return found(li, span);
                }
            }
        }
    } else {
        for li in (0..=last).rev() {
            let max_col = if li == from.line { from.col } else { usize::MAX };
            // The regex engine cannot walk its matches backwards: keep the last one before the caret.
            if let Some(span) = line_spans(&ed.lines[li], re).filter(|&(_, e)| e <= max_col).last() {
                return found(li, span);
            }
        }
        if wrap {
            for li in (last..n).rev() {
                if let Some(span) = line_spans(&ed.lines[li], re).last() {
                    return found(li, span);
                }
            }
        }
    }
    None
}

/// Matches per line for the live count; summing gives the document total.
pub fn count_matches_per_line(lines: &[String], re: &Regex) -> Vec<usize> {
    lines.iter().map(|line| re.find_iter(line).filter(|m| m.start() != m.end()).count()).collect()
}

/// Which match (1-based) the selection covers exactly, if any. `counts` holds one entry per
/// line, so only the selection's own line is scanned; the lines before it contribute their
/// cached counts.
pub fn selected_match_index(ed: &EditorState, counts: &[usize], re: Option<&Regex>, s: TextPosition, e: TextPosition) -> Option<usize> {
    let re = re?;
    if s.line != e.line || s == e || s.line >= counts.len() || s.line >= ed.lines.len() {
        return None;
    }
    let line = &ed.lines[s.line];
    let before: usize = counts[..s.line].iter().sum();
    let (sb, eb) = (char_to_byte(line, s.col), char_to_byte(line, e.col));
    for (rank, m) in re.find_iter(line).filter(|m| m.start() != m.end()).enumerate() {
        if m.start() == sb && m.end() == eb {
            return Some(before + rank + 1);
        }
    }
    None
}

/// The document as one text, its lines joined with `\n`.
///
/// Building it copies the whole document, so callers cache it by document version and
/// rebuild it only after an edit, instead of joining again for every key press or Find Next.
pub struct JoinedText {
    pub text: String,
    /// Byte offset where each line starts in `text`.
    pub line_starts: Vec<usize>,
}

impl JoinedText {
    pub fn new(lines: &[String]) -> Self {
        let mut text = String::with_capacity(lines.iter().map(String::len).sum::<usize>() + lines.len());
        let mut line_starts = Vec::with_capacity(lines.len());
        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                text.push('\n');
            }
            line_starts.push(text.len());
            text.push_str(line);
        }
        Self { text, line_starts }
    }

    /// The byte offset of a position; `lines` supplies the character widths of its line.
    /// A column past the end of the line is the end of the line.
    pub fn pos_to_byte(&self, lines: &[String], p: TextPosition) -> usize {
        let Some(last) = lines.len().checked_sub(1) else { return 0 };
        let li = p.line.min(last);
        self.line_starts[li] + char_to_byte(&lines[li], p.col)
    }

    /// The position of a byte offset, clamped to the text. The line break itself belongs to
    /// the line it ends. `b` must be a character boundary, which every match edge is.
    pub fn byte_to_pos(&self, b: usize) -> TextPosition {
        let b = b.min(self.text.len());
        let li = self.line_starts.partition_point(|&s| s <= b).saturating_sub(1);
        TextPosition::new(li, self.text[self.line_starts[li]..b].chars().count())
    }
}

/// Non-empty matches of a cross-line pattern in the joined text, as byte ranges in
/// document order.
pub fn text_match_spans(re: &Regex, text: &str) -> Vec<(usize, usize)> {
    re.find_iter(text).filter(|m| m.start() != m.end()).map(|m| (m.start(), m.end())).collect()
}

/// Per-line counts of `spans`, each counted on the line where it starts. The result has
/// the same shape as [`count_matches_per_line`].
pub fn spans_per_line(joined: &JoinedText, spans: &[(usize, usize)]) -> Vec<usize> {
    let mut counts = vec![0usize; joined.line_starts.len()];
    for &(s, _) in spans {
        let li = joined.line_starts.partition_point(|&x| x <= s).saturating_sub(1);
        counts[li] += 1;
    }
    counts
}

/// The same navigation contract as [`find_in_lines`], over sorted non-overlapping byte
/// ranges of a joined text. `from` is a byte offset in that text.
pub fn find_in_spans(spans: &[(usize, usize)], from: usize, backward: bool, wrap: bool) -> Option<(usize, usize)> {
    if backward {
        spans.partition_point(|(_, e)| *e <= from).checked_sub(1).map(|i| spans[i]).or_else(|| wrap.then(|| spans.last().copied()).flatten())
    } else {
        let first = spans.partition_point(|(s, _)| *s < from);
        spans.get(first).copied().or_else(|| wrap.then(|| spans.first().copied()).flatten())
    }
}

/// The 0-based rank of the span that is exactly `(start, end)`.
pub fn span_index(spans: &[(usize, usize)], start: usize, end: usize) -> Option<usize> {
    spans.binary_search(&(start, end)).ok()
}

/// The text one match is replaced with: capture references (`$1`) for regular-expression
/// searches, the literal text otherwise.
pub fn expand_match(caps: &Captures<'_>, rep: &str, use_regex: bool) -> String {
    if !use_regex {
        return rep.to_string();
    }
    let mut out = String::new();
    caps.expand(rep, &mut out);
    out
}

/// Replaces every match in `text`, empty matches included, and returns the new text with
/// the number of replacements.
pub fn replace_in_text(text: &str, re: &Regex, rep: &str, use_regex: bool) -> (String, usize) {
    let mut count = 0;
    let out = re.replace_all(text, |caps: &Captures<'_>| {
        count += 1;
        expand_match(caps, rep, use_regex)
    });
    (out.into_owned(), count)
}

/// Line breaks in `s` become separate lines: `\r\n` and a lone `\r` count as `\n`.
fn push_split_lines(out: &mut Vec<String>, s: &str) {
    if s.contains('\r') {
        out.extend(s.replace("\r\n", "\n").replace('\r', "\n").split('\n').map(str::to_string));
    } else {
        out.extend(s.split('\n').map(str::to_string));
    }
}

/// 全部取代（以行為單位），並維護書籤（取代文字含換行時，書籤隨行號位移）。
/// 只有 `lines` 範圍內的行會被取代；其餘行原樣保留（不複製）。
pub fn replace_all_lines(ed: &mut EditorState, re: &Regex, rep: &str, use_regex: bool, lines: Range<usize>) -> usize {
    let mut count = 0;
    let mut line_map = Vec::with_capacity(ed.lines.len());
    // Lines without a match are moved into the new vector, not copied.
    let old = std::mem::take(&mut ed.lines);
    let mut new_lines = Vec::with_capacity(old.len());
    for (li, line) in old.into_iter().enumerate() {
        line_map.push(new_lines.len());
        if !lines.contains(&li) || !re.is_match(&line) {
            new_lines.push(line);
            continue;
        }
        let (replaced, n) = replace_in_text(&line, re, rep, use_regex);
        count += n;
        push_split_lines(&mut new_lines, &replaced);
    }
    if count > 0 {
        // P1-11：書籤依新的行號重新對應
        ed.bookmarks = ed.bookmarks.iter().filter_map(|&b| line_map.get(b).copied()).collect();
        ed.clamp_caret();
    }
    ed.lines = new_lines;
    count
}

/// 跨行全部取代：整份文字一次取代後重建行列表；書籤依每個取代點的淨行數位移。
/// 空匹配一律略過（與查找下一個一致）。
pub fn replace_all_multiline(ed: &mut EditorState, re: &Regex, rep: &str, use_regex: bool) -> usize {
    let joined = JoinedText::new(&ed.lines);
    let text = joined.text.as_str();
    let starts = &joined.line_starts;
    let mut out = String::with_capacity(text.len());
    let mut last = 0usize;
    let mut count = 0usize;
    // 每個取代點：(起始行, 淨行數變化)
    let mut deltas: Vec<(usize, i64)> = Vec::new();
    for cap in re.captures_iter(text) {
        let Some(m) = cap.get(0) else { continue };
        if m.start() == m.end() {
            continue;
        }
        let mut seg = expand_match(&cap, rep, use_regex);
        if seg.contains('\r') {
            seg = seg.replace("\r\n", "\n").replace('\r', "\n");
        }
        let old_lines = 1 + text[m.start()..m.end()].matches('\n').count();
        let new_lines = 1 + seg.matches('\n').count();
        let at_line = starts.partition_point(|&s| s <= m.start()).saturating_sub(1);
        deltas.push((at_line, new_lines as i64 - old_lines as i64));
        out.push_str(&text[last..m.start()]);
        out.push_str(&seg);
        last = m.end();
        count += 1;
    }
    out.push_str(&text[last..]);
    let mut new_lines = Vec::new();
    push_split_lines(&mut new_lines, &out);
    // 書籤位移：位在某取代點之後的書籤累加該點的淨行數；越界的捨棄
    ed.bookmarks = ed
        .bookmarks
        .iter()
        .filter_map(|&b| {
            let shift: i64 = deltas.iter().filter(|(at, _)| *at < b).map(|(_, d)| d).sum();
            let nb = b as i64 + shift;
            (nb >= 0 && (nb as usize) < new_lines.len()).then_some(nb as usize)
        })
        .collect();
    ed.lines = new_lines;
    ed.clamp_caret();
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile(query: &str, use_regex: bool) -> Regex {
        build_regex(query, &SearchOptions::new(true, false, use_regex)).unwrap()
    }

    #[test]
    fn test_options_case_whole_word_and_regex() {
        let lines = vec!["Apple apple Application".to_string(), "an apple a day".to_string()];
        let counts = |query: &str, opts: &SearchOptions| count_matches_per_line(&lines, &build_regex(query, opts).unwrap());
        // Case-insensitive, "Apple" and "apple" on the first line, "apple" on the second.
        assert_eq!(counts("apple", &SearchOptions::new(false, false, false)), vec![2, 1]);
        assert_eq!(counts("apple", &SearchOptions::new(true, false, false)), vec![1, 1]);
        // "Application" starts with a capital A, so it only counts when case is ignored.
        assert_eq!(counts("app", &SearchOptions::new(true, false, false)), vec![1, 1]);
        assert_eq!(counts("app", &SearchOptions::new(false, false, false)), vec![3, 1]);
        // Whole words only.
        assert_eq!(counts("app", &SearchOptions::new(false, true, false)), vec![0, 0]);
        assert_eq!(counts("apple", &SearchOptions::new(true, true, false)), vec![1, 1]);
        // A literal query: the dot is plain text, so "axc" is left alone.
        let (text, n) = replace_in_text("a.c axc", &compile("a.c", false), "-", false);
        assert_eq!((text.as_str(), n), ("- axc", 1));
        // Regular expressions, and the whole-word rule around an alternation.
        let code = vec!["Error: code 404 not found".to_string(), "Warning: code 500 error".to_string()];
        assert_eq!(count_matches_per_line(&code, &compile(r"code \d+", true)), vec![1, 1]);
        let word_alt = build_regex("a|bc", &SearchOptions::new(false, true, true)).unwrap();
        assert_eq!(count_matches_per_line(&["a bc abc x".to_string()], &word_alt), vec![2]);
    }

    #[test]
    fn test_errors_report_their_kind() {
        assert!(matches!(build_regex("", &SearchOptions::default()), Err(SearchError::EmptyQuery)));
        assert!(matches!(build_regex("(", &SearchOptions::new(false, false, true)), Err(SearchError::InvalidRegex(_))));
        // A literal query is never an invalid regular expression.
        assert!(build_regex("(", &SearchOptions::new(false, false, false)).is_ok());
    }

    #[test]
    fn test_hex_query_parse() {
        assert_eq!(parse_hex_query("48 65 6C 6C 6F").unwrap(), b"Hello");
        assert_eq!(parse_hex_query("48656C6C6F").unwrap(), b"Hello");
        assert_eq!(parse_hex_query("0x48 0x65").unwrap(), b"He");
        assert_eq!(parse_hex_query("").unwrap_err(), SearchError::EmptyHexQuery);
        // 位元組長度為偶數的多位元組字元曾因位元組切片落在字元中間而 panic
        assert_eq!(parse_hex_query("中文").unwrap_err(), SearchError::HexDigit('中'));
        assert_eq!(parse_hex_query("A中").unwrap_err(), SearchError::HexDigit('中'));
        assert_eq!(parse_hex_query("ABC").unwrap_err(), SearchError::OddHexLength);
        // from_str_radix 會接受正號，必須明確拒絕
        assert_eq!(parse_hex_query("+1").unwrap_err(), SearchError::HexDigit('+'));
    }

    #[test]
    fn test_pattern_may_span_lines() {
        assert!(pattern_may_span_lines("a\nb", false), "a literal newline, searched literally");
        assert!(pattern_may_span_lines(r"a\nb", true));
        // The two characters `\n` searched literally, and a regex for a literal backslash and n.
        assert!(!pattern_may_span_lines(r"\n", false));
        assert!(!pattern_may_span_lines(r"a\\nb", true));
        assert!(pattern_may_span_lines(r"a\\\nb", true), "an escaped backslash, then the newline escape");
        assert!(!pattern_may_span_lines(r"a\s*b", true), "whitespace classes stay within a line");
    }

    #[test]
    fn test_find_in_lines_wraps_and_respects_direction() {
        let ed = EditorState::from_text("foo bar\nbar foo\nfoo");
        let re = compile("foo", false);
        let m = find_in_lines(&ed, &re, TextPosition::new(0, 1), false, true).unwrap();
        assert_eq!(m, (TextPosition::new(1, 4), TextPosition::new(1, 7)));
        assert_eq!(find_in_lines(&ed, &re, TextPosition::new(2, 3), false, true).unwrap().0, TextPosition::new(0, 0));
        assert!(find_in_lines(&ed, &re, TextPosition::new(2, 3), false, false).is_none());
        assert_eq!(find_in_lines(&ed, &re, TextPosition::new(1, 4), true, true).unwrap().0, TextPosition::new(0, 0));
        // Without wrap: from the end of a match, and from inside one.
        assert_eq!(find_in_lines(&ed, &re, TextPosition::new(1, 4), true, false).unwrap().0, TextPosition::new(0, 0));
        assert_eq!(find_in_lines(&ed, &re, TextPosition::new(1, 7), true, false).unwrap().0, TextPosition::new(1, 4));
        assert_eq!(find_in_lines(&ed, &re, TextPosition::new(1, 5), true, false).unwrap().0, TextPosition::new(0, 0), "a match the caret is inside is not a previous match");
        assert_eq!(find_in_lines(&ed, &re, TextPosition::new(0, 0), true, true).unwrap().0, TextPosition::new(2, 0));
        // Columns are characters, not bytes: "中foo foo" matches at columns 1-4 and 5-8.
        let ed = EditorState::from_text("中foo foo");
        assert_eq!(find_in_lines(&ed, &re, TextPosition::new(0, 1), false, false).unwrap(), (TextPosition::new(0, 1), TextPosition::new(0, 4)));
        assert_eq!(find_in_lines(&ed, &re, TextPosition::new(0, 2), false, false).unwrap(), (TextPosition::new(0, 5), TextPosition::new(0, 8)));
        assert_eq!(find_in_lines(&ed, &re, TextPosition::new(0, 5), true, false).unwrap(), (TextPosition::new(0, 1), TextPosition::new(0, 4)));
        // Empty matches are skipped.
        assert!(find_in_lines(&ed, &compile("x*", true), TextPosition::new(0, 0), false, true).is_none());
    }

    #[test]
    fn test_selected_match_index_counts_characters() {
        let mut ed = EditorState::from_text("ab ab\n中ab");
        let re = compile("ab", false);
        let counts = count_matches_per_line(&ed.lines, &re);
        assert_eq!(counts, vec![2, 1]);
        let current = |ed: &EditorState| selected_match_index(ed, &counts, Some(&re), ed.selection.start(), ed.selection.end());
        assert_eq!(current(&ed), None);
        // Columns are characters: the match after "中" starts at byte 3 but column 1.
        ed.select_range(TextPosition::new(1, 1), TextPosition::new(1, 3));
        assert_eq!(current(&ed), Some(3));
        ed.select_range(TextPosition::new(0, 5), TextPosition::new(0, 3));
        assert_eq!(current(&ed), Some(2));
        ed.select_range(TextPosition::new(0, 0), TextPosition::new(0, 1));
        assert_eq!(current(&ed), None);
        assert_eq!(selected_match_index(&ed, &counts, None, TextPosition::new(0, 0), TextPosition::new(0, 2)), None);
    }

    #[test]
    fn test_replace_in_text_expands_captures_only_for_regex() {
        let (text, n) = replace_in_text("k1=v1 k2=v2", &compile(r"(k\d)=(v\d)", true), "$2:$1", true);
        assert_eq!((text.as_str(), n), ("v1:k1 v2:k2", 2));
        // Searched literally, the replacement is literal too: `$1` is just text.
        let (text, n) = replace_in_text("k=v k=v", &compile("k=v", false), "$1", false);
        assert_eq!((text.as_str(), n), ("$1 $1", 2));
        let (text, n) = replace_in_text("abc", &compile("zz", false), "x", false);
        assert_eq!((text.as_str(), n), ("abc", 0));
    }

    #[test]
    fn test_replace_all_lines_keeps_bookmarks_and_untouched_lines() {
        let mut ed = EditorState::from_text("a\nb\na\nc");
        ed.bookmarks.insert(2);
        ed.bookmarks.insert(3);
        // 取代文字含換行：第 0 行變兩行，之後的書籤往後移一行
        assert_eq!(replace_all_lines(&mut ed, &compile("a", false), "x\ny", false, 0..1), 1);
        assert_eq!(ed.lines, vec!["x", "y", "b", "a", "c"]);
        assert_eq!(ed.bookmarks.iter().copied().collect::<Vec<_>>(), vec![3, 4]);
        // CR and CRLF in the replacement split like a newline.
        let mut ed = EditorState::from_text("k v");
        assert_eq!(replace_all_lines(&mut ed, &compile(" ", false), "a\r\nb\rc", false, 0..1), 1);
        assert_eq!(ed.lines, vec!["ka", "b", "cv"]);
        // No match: the document and the caret stay as they are.
        let mut ed = EditorState::from_text("a\nb");
        ed.set_caret(1, 1);
        assert_eq!(replace_all_lines(&mut ed, &compile("zz", false), "x", false, 0..2), 0);
        assert_eq!(ed.lines, vec!["a", "b"]);
        assert_eq!((ed.selection.head.line, ed.selection.head.col), (1, 1));
        // Zero-width matches are replaced: `^` prefixes every line.
        let mut ed = EditorState::from_text("a\nb");
        assert_eq!(replace_all_lines(&mut ed, &compile("^", true), "> ", true, 0..2), 2);
        assert_eq!(ed.lines, vec!["> a", "> b"]);
    }

    #[test]
    fn test_joined_text_converts_cjk_columns() {
        let lines = vec!["中文ab".to_string(), "x".to_string()];
        let joined = JoinedText::new(&lines);
        assert_eq!(joined.text, "中文ab\nx");
        assert_eq!(joined.line_starts, vec![0, 9]);
        // 中 and 文 take three bytes each but one column.
        assert_eq!(joined.pos_to_byte(&lines, TextPosition::new(0, 2)), 6);
        assert_eq!(joined.byte_to_pos(6), TextPosition::new(0, 2));
        assert_eq!(joined.byte_to_pos(3), TextPosition::new(0, 1));
        // The line break belongs to the line it ends; positions past the end clamp.
        assert_eq!(joined.byte_to_pos(8), TextPosition::new(0, 4));
        assert_eq!(joined.pos_to_byte(&lines, TextPosition::new(1, 9)), 10);
        assert_eq!(joined.pos_to_byte(&[], TextPosition::new(0, 0)), 0, "no lines: no panic");
    }

    #[test]
    fn test_cross_line_matches_navigate_like_line_matches() {
        let lines = vec!["foo bar".to_string(), "bar foo".to_string(), "end".to_string()];
        let joined = JoinedText::new(&lines);
        let re = compile(r"bar\nbar", true);
        let spans = text_match_spans(&re, &joined.text);
        // "bar\nbar" is 7 bytes: byte 4 to byte 11.
        assert_eq!(spans, vec![(4, 11)]);
        // Each match counts on the line where it starts.
        assert_eq!(spans_per_line(&joined, &spans), vec![1, 0, 0]);
        assert_eq!(joined.byte_to_pos(4), TextPosition::new(0, 4));
        assert_eq!(joined.byte_to_pos(11), TextPosition::new(1, 3));
        assert_eq!(joined.pos_to_byte(&lines, TextPosition::new(1, 3)), 11);
        // Forward: a match starting at the caret counts as the next one.
        assert_eq!(find_in_spans(&spans, 0, false, false), Some((4, 11)));
        assert_eq!(find_in_spans(&spans, 4, false, false), Some((4, 11)));
        assert_eq!(find_in_spans(&spans, 5, false, false), None);
        assert_eq!(find_in_spans(&spans, 5, false, true), Some((4, 11)), "wraps to the first");
        // Backward: a match ending at the caret counts as the previous one.
        assert_eq!(find_in_spans(&spans, 11, true, false), Some((4, 11)));
        assert_eq!(find_in_spans(&spans, 10, true, false), None);
        assert_eq!(find_in_spans(&spans, 10, true, true), Some((4, 11)), "wraps to the last");
        assert_eq!(find_in_spans(&[], 0, false, true), None);
        // The rank of the match a selection covers exactly.
        assert_eq!(span_index(&spans, 4, 11), Some(0));
        assert_eq!(span_index(&spans, 4, 10), None);
    }

    #[test]
    fn test_alternation_with_a_line_break_finds_every_branch() {
        // Line by line, `a\nb|b` still has results: the lone `b`s. The first line engine
        // result used to end the search there, so Find Next landed on line 1 and skipped the
        // cross-line match that starts earlier, at the top of the document.
        let ed = EditorState::from_text("a\nb\nb");
        let re = compile(r"a\nb|b", true);
        assert_eq!(count_matches_per_line(&ed.lines, &re), vec![0, 1, 1]);
        let joined = JoinedText::new(&ed.lines);
        let spans = text_match_spans(&re, &joined.text);
        assert_eq!(spans, vec![(0, 3), (4, 5)]);
        assert_eq!(joined.byte_to_pos(spans[0].0), TextPosition::new(0, 0));
        assert_eq!(joined.byte_to_pos(spans[0].1), TextPosition::new(1, 1));
        // Each match counts on the line where it starts.
        assert_eq!(spans_per_line(&joined, &spans), vec![1, 0, 1]);
        assert_eq!(span_index(&spans, 4, 5), Some(1));
    }

    #[test]
    fn test_replace_all_multiline_and_bookmarks() {
        let mut ed = EditorState::from_text("a\nXX\nXX\nb\nXX");
        ed.bookmarks.insert(1);
        ed.bookmarks.insert(4);
        // 兩行合併為一行：後方書籤上移一行，前面的書籤不動；剩餘單獨 XX 不符合
        assert_eq!(replace_all_multiline(&mut ed, &compile(r"XX\nXX", true), "M", true), 1);
        assert_eq!(ed.lines, vec!["a", "M", "b", "XX"]);
        assert_eq!(ed.bookmarks.iter().copied().collect::<Vec<_>>(), vec![1, 3]);
        // Capture references across the break; CR in the replacement splits like a newline.
        let mut ed = EditorState::from_text("a k1\nv1 b\nend");
        ed.bookmarks.insert(2);
        assert_eq!(replace_all_multiline(&mut ed, &compile(r"(k\d)\n(v\d)", true), "$2\r\n$1", true), 1);
        assert_eq!(ed.lines, vec!["a v1", "k1 b", "end"]);
        assert_eq!(ed.bookmarks.iter().copied().collect::<Vec<_>>(), vec![2]);
        // Nothing to replace leaves the text as it is.
        assert_eq!(replace_all_multiline(&mut ed, &compile("zz", false), "x", false), 0);
        assert_eq!(ed.lines, vec!["a v1", "k1 b", "end"]);
    }
}
