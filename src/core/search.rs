#![allow(dead_code)]

use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use super::editor::TextPosition;

/// 正規表示式方言：Perl（Rust `regex` crate 語法）、POSIX ERE（Unix）與 UltraEdit 傳統語法。
/// 後兩者在 `build_regex` 內翻譯成 Perl 語法再編譯。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RegexFlavor {
    #[default]
    Perl,
    Unix,
    UltraEdit,
}

impl RegexFlavor {
    pub const ALL: [RegexFlavor; 3] = [RegexFlavor::Perl, RegexFlavor::Unix, RegexFlavor::UltraEdit];

    /// 顯示名稱（正規表達式工具慣例以英文原文呈現）
    pub fn label(self) -> &'static str {
        match self {
            RegexFlavor::Perl => "Perl",
            RegexFlavor::Unix => "Unix",
            RegexFlavor::UltraEdit => "UltraEdit",
        }
    }
}

/// UltraEdit 查找模式含反向引用：RE2 引擎不支援
const ERR_UE_BACKREF: &str = "UltraEdit 語法不支援在查找模式中使用反向引用（^1..^9），請改用 Perl 語法";
/// UltraEdit 取代文字含剪貼板引用：暫不支援
const ERR_UE_CLIPBOARD: &str = "UltraEdit 語法的 ^c／^b（剪貼板取代）暫不支援";

/// 搜尋匹配選項
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SearchOptions {
    pub match_case: bool,
    pub whole_word: bool,
    pub use_regex: bool,
    /// 僅在 `use_regex` 時生效；預設 Perl
    pub flavor: RegexFlavor,
}

impl SearchOptions {
    pub fn new(match_case: bool, whole_word: bool, use_regex: bool) -> Self {
        Self {
            match_case,
            whole_word,
            use_regex,
            flavor: RegexFlavor::Perl,
        }
    }

    /// 指定方言（Builder 風格）
    pub fn with_flavor(mut self, flavor: RegexFlavor) -> Self {
        self.flavor = flavor;
        self
    }
}

/// 單一搜尋匹配結果
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchMatch {
    pub start: TextPosition,
    pub end: TextPosition,
    pub byte_offset: usize,
    pub byte_len: usize,
    pub matched_text: String,
}

impl SearchMatch {
    #[allow(dead_code)]
    pub fn new(
        start: TextPosition,
        end: TextPosition,
        byte_offset: usize,
        byte_len: usize,
        matched_text: String,
    ) -> Self {
        Self {
            start,
            end,
            byte_offset,
            byte_len,
            matched_text,
        }
    }
}

/// 解析十六進位搜尋字串（支援如 "48 65 6C 6C 6F" 或 "48656C6C6F" 或 "0x48 0x65"）
pub fn parse_hex_query(query: &str) -> Result<Vec<u8>, String> {
    let clean = query
        .replace("0x", "")
        .replace("0X", "")
        .replace([' ', '\t', '\r', '\n', ',', '-'], "");

    if clean.is_empty() {
        return Err("搜尋內容不可為空".to_string());
    }
    // 先確認全為 ASCII 十六進位數字：之後以位元組切片取兩字一組才安全（中文等多位元組字元會切在字元中間而 panic）
    if let Some(c) = clean.chars().find(|c| !c.is_ascii_hexdigit()) {
        return Err(format!("無效的十六進位字元: {}", c));
    }
    if !clean.len().is_multiple_of(2) {
        return Err("十六進位序列長度必須為偶數".to_string());
    }

    let mut bytes = Vec::with_capacity(clean.len() / 2);
    for i in (0..clean.len()).step_by(2) {
        let byte_str = &clean[i..i + 2];
        let byte = u8::from_str_radix(byte_str, 16)
            .map_err(|_| format!("無效的十六進位字元: {}", byte_str))?;
        bytes.push(byte);
    }
    Ok(bytes)
}

/// 在位元組串流中搜尋 Hex Pattern，回傳所有匹配的位元組位移（Offsets）
pub fn find_hex_matches(data: &[u8], pattern: &[u8]) -> Vec<usize> {
    if pattern.is_empty() || data.len() < pattern.len() {
        return Vec::new();
    }
    let mut matches = Vec::new();
    for i in 0..=(data.len() - pattern.len()) {
        if &data[i..i + pattern.len()] == pattern {
            matches.push(i);
        }
    }
    matches
}

/// 將 Unix（POSIX ERE）搜尋模式轉成 Rust `regex` 語法。
/// POSIX 字元類（`[:digit:]` 等）與 `\w`/`\d` 皆為 Rust 正則原生支援，僅需處理單詞邊界寫法。
fn translate_unix_pattern(query: &str) -> String {
    query.replace(r"[[:<:]]", r"\b").replace(r"[[:>:]]", r"\b")
}

/// UltraEdit 傳統語法的搜尋模式翻譯。
/// 語法符號：`?` 任意單字元、`*` 任意串、`^t`/`^n`/`^r` 控制字元、`^(…)` 捕獲群組（以 `)` 結束）、
/// `[…]` 字元集合（`[~…]` 為否定）、`^^` 字面 `^`、`\x` 字面 x；`.` 是字面句點，`( )` 是字面括號。
/// 行首 `^` 與行尾 `$` 是錨點。
fn translate_ue_pattern(query: &str) -> Result<String, String> {
    let mut out = String::with_capacity(query.len() * 2);
    let chars: Vec<char> = query.chars().collect();
    let mut i = 0;
    let mut in_class = false;
    // UE 群組為 ^( … )：普通 ) 在群組內是結束符，在群組外是字面括號
    let mut group_depth = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if in_class {
            // 字元集合內除了開頭的否定記號 `~` 外全按字面處理
            if c == ']' {
                in_class = false;
                out.push(']');
            } else if c == '~' && out.ends_with('[') {
                out.push('^');
            } else {
                out.push(c);
            }
            i += 1;
            continue;
        }
        match c {
            // UE 以 ^^ 表示字面的 ^
            '^' => {
                if chars.get(i + 1) == Some(&'^') {
                    out.push_str(r"\^");
                    i += 2;
                } else if let Some(&n) = chars.get(i + 1) {
                    match n {
                        't' => {
                            out.push_str(r"\t");
                            i += 2;
                        }
                        'n' => {
                            out.push('\n');
                            i += 2;
                        }
                        'r' => {
                            out.push('\r');
                            i += 2;
                        }
                        '(' => {
                            // ^( … ) 為捕獲群組
                            out.push('(');
                            group_depth += 1;
                            i += 2;
                        }
                        '0'..='9' => {
                            // 查找模式中的反向引用：RE2 引擎不支援
                            return Err(ERR_UE_BACKREF.to_string());
                        }
                        _ => {
                            // 行首的孤立 ^ 是行錨點；其餘視為字面 ^
                            if i == 0 {
                                out.push('^');
                            } else {
                                out.push_str(r"\^");
                            }
                            i += 1;
                        }
                    }
                } else {
                    out.push('^');
                    i += 1;
                }
            }
            '*' => {
                out.push_str(".*");
                i += 1;
            }
            '?' => {
                out.push('.');
                i += 1;
            }
            // UE 的 . 是字面句點，不做萬用字元
            '.' => {
                out.push_str(r"\.");
                i += 1;
            }
            '[' => {
                in_class = true;
                out.push('[');
                i += 1;
            }
            // UE 的群組以 ) 結束；群組外的 ) 與 ( 皆為字面括號
            ')' if group_depth > 0 => {
                out.push(')');
                group_depth -= 1;
                i += 1;
            }
            '(' | ')' => {
                out.push('\\');
                out.push(c);
                i += 1;
            }
            // UE 不支援 alternation
            '|' => {
                out.push_str(r"\|");
                i += 1;
            }
            // UE 行尾錨點
            '$' if i + 1 == chars.len() => {
                out.push('$');
                i += 1;
            }
            '$' => {
                out.push_str(r"\$");
                i += 1;
            }
            // UE 以 \ 逸出下一個字元（如 \* 表示字面星號）
            '\\' if i + 1 < chars.len() => {
                let n = chars[i + 1];
                if "\\.[]{}()*+?|^$".contains(n) {
                    out.push('\\');
                }
                out.push(n);
                i += 2;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    Ok(out)
}

/// 將取代文字依方言轉成 Rust `Captures::expand` 可接受的展開語法。
/// Perl/Unix 原樣（已是 `$1` 語法）；UltraEdit 把 `^1`..`^9`、`^0` 轉成 `$n`、
/// `^t`/`^n`/`^r` 轉成實際控制字元，字面 `$` 以 `$$` 逸出。
/// UE 的 `^c`（剪貼板）與 `^b` 不支援，回傳錯誤。
pub fn translate_replacement(rep: &str, flavor: RegexFlavor) -> Result<String, String> {
    if flavor != RegexFlavor::UltraEdit {
        return Ok(rep.to_string());
    }
    let mut out = String::with_capacity(rep.len());
    let chars: Vec<char> = rep.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c != '^' {
            // Rust expand 以 $ 引用群組，字面 $ 需加倍
            if c == '$' {
                out.push_str("$$");
            } else {
                out.push(c);
            }
            i += 1;
            continue;
        }
        match chars.get(i + 1) {
            Some('^') => {
                out.push('^');
                i += 2;
            }
            Some('t') => {
                out.push('\t');
                i += 2;
            }
            Some('n') => {
                out.push('\n');
                i += 2;
            }
            Some('r') => {
                out.push('\r');
                i += 2;
            }
            Some(n @ ('0'..='9')) => {
                out.push('$');
                out.push(*n);
                i += 2;
            }
            Some('c' | 'b') => {
                return Err(ERR_UE_CLIPBOARD.to_string());
            }
            _ => {
                out.push('^');
                i += 1;
            }
        }
    }
    Ok(out)
}

/// 建構比對用之正規表達式物件
pub fn build_regex(query: &str, options: &SearchOptions) -> Result<Regex, String> {
    if query.is_empty() {
        return Err("搜尋關鍵字不可為空".to_string());
    }

    let pattern_str = if options.use_regex {
        let translated = match options.flavor {
            RegexFlavor::Perl => query.to_string(),
            RegexFlavor::Unix => translate_unix_pattern(query),
            RegexFlavor::UltraEdit => translate_ue_pattern(query)?,
        };
        if options.whole_word {
            format!(r"\b(?:{})\b", translated)
        } else {
            translated
        }
    } else {
        let escaped = regex::escape(query);
        if options.whole_word {
            format!(r"\b{}\b", escaped)
        } else {
            escaped
        }
    };

    RegexBuilder::new(&pattern_str)
        .case_insensitive(!options.match_case)
        .multi_line(true)
        .build()
        .map_err(|e| format!("無效的正規表達式: {}", e))
}

/// 在文字行列表中搜尋所有匹配項目
pub fn find_all_matches(
    lines: &[String],
    query: &str,
    options: &SearchOptions,
) -> Result<Vec<SearchMatch>, String> {
    let re = build_regex(query, options)?;
    let mut matches = Vec::new();
    let mut line_start_byte_offset = 0;

    for (line_idx, line) in lines.iter().enumerate() {
        for m in re.find_iter(line) {
            let start_byte = m.start();
            let end_byte = m.end();
            let start_col = line[..start_byte].chars().count();
            let end_col = start_col + line[start_byte..end_byte].chars().count();

            matches.push(SearchMatch {
                start: TextPosition::new(line_idx, start_col),
                end: TextPosition::new(line_idx, end_col),
                byte_offset: line_start_byte_offset + start_byte,
                byte_len: end_byte - start_byte,
                matched_text: m.as_str().to_string(),
            });
        }
        line_start_byte_offset += line.len() + 1; // 預估包含換行字元
    }

    Ok(matches)
}

/// 計算匹配總次數
#[allow(dead_code)]
pub fn count_matches(
    lines: &[String],
    query: &str,
    options: &SearchOptions,
) -> Result<usize, String> {
    let matches = find_all_matches(lines, query, options)?;
    Ok(matches.len())
}

/// 尋找游標之後的下一個匹配項目（環狀循環）
pub fn find_next_match(
    matches: &[SearchMatch],
    current_pos: TextPosition,
) -> Option<(usize, SearchMatch)> {
    if matches.is_empty() {
        return None;
    }
    // 尋找第一個起點在 current_pos 之後的項目
    for (idx, m) in matches.iter().enumerate() {
        if m.start >= current_pos {
            return Some((idx, m.clone()));
        }
    }
    // 環狀回起點
    Some((0, matches[0].clone()))
}

/// 尋找游標之前的上一個匹配項目（環狀循環）
pub fn find_prev_match(
    matches: &[SearchMatch],
    current_pos: TextPosition,
) -> Option<(usize, SearchMatch)> {
    if matches.is_empty() {
        return None;
    }
    // 從後往前找第一個起點在 current_pos 之前的項目
    for (idx, m) in matches.iter().enumerate().rev() {
        if m.start < current_pos {
            return Some((idx, m.clone()));
        }
    }
    // 環狀回末尾
    let last_idx = matches.len() - 1;
    Some((last_idx, matches[last_idx].clone()))
}

/// 全部取代：將所有符合條件之字串替換為目標字串，回傳更新後之行列表與取代總次數
pub fn replace_all_matches(
    lines: &[String],
    query: &str,
    replace_with: &str,
    options: &SearchOptions,
) -> Result<(Vec<String>, usize), String> {
    let re = build_regex(query, options)?;
    let rep = if options.use_regex { translate_replacement(replace_with, options.flavor)? } else { replace_with.to_string() };
    let mut new_lines = Vec::with_capacity(lines.len());
    let mut total_replacements = 0;

    for line in lines {
        let mut count = 0;
        let replaced = re.replace_all(line, |caps: &regex::Captures| {
            count += 1;
            if options.use_regex {
                let mut dst = String::new();
                caps.expand(&rep, &mut dst);
                dst
            } else {
                replace_with.to_string()
            }
        });
        total_replacements += count;
        new_lines.push(replaced.into_owned());
    }

    Ok((new_lines, total_replacements))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_case_sensitive_and_whole_word() {
        let lines = vec![
            "Apple apple Application".to_string(),
            "an apple a day".to_string(),
        ];

        // 1. 忽略大小寫
        let opt_ci = SearchOptions::new(false, false, false);
        let m1 = find_all_matches(&lines, "apple", &opt_ci).unwrap();
        assert_eq!(m1.len(), 3); // "Apple", "apple", "apple"

        // 2. 區分大小寫
        let opt_cs = SearchOptions::new(true, false, false);
        let m2 = find_all_matches(&lines, "apple", &opt_cs).unwrap();
        assert_eq!(m2.len(), 2); // 只有小寫 "apple"

        // 3. 全字匹配 + 區分大小寫
        let opt_ww = SearchOptions::new(true, true, false);
        let m3 = find_all_matches(&lines, "apple", &opt_ww).unwrap();
        assert_eq!(m3.len(), 2);
        assert_eq!(m3[0].start, TextPosition::new(0, 6)); // 第 0 行的 "apple"
        assert_eq!(m3[1].start, TextPosition::new(1, 3)); // 第 1 行的 "apple"
    }

    #[test]
    fn test_search_regex() {
        let lines = vec![
            "Error: code 404 not found".to_string(),
            "Warning: code 500 error".to_string(),
        ];

        let opt_regex = SearchOptions::new(false, false, true);
        let m = find_all_matches(&lines, r"code \d+", &opt_regex).unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].matched_text, "code 404");
        assert_eq!(m[1].matched_text, "code 500");
    }

    #[test]
    fn test_hex_query_parse_and_find() {
        let query = "48 65 6C 6C 6F";
        let parsed = parse_hex_query(query).unwrap();
        assert_eq!(parsed, b"Hello");

        let data = b"Header...Hello World...Hello!";
        let matches = find_hex_matches(data, &parsed);
        assert_eq!(matches, vec![9, 23]);
    }

    #[test]
    fn test_hex_query_rejects_non_hex_without_panic() {
        // 位元組長度為偶數的多位元組字元曾因位元組切片落在字元中間而 panic
        assert!(parse_hex_query("中文").is_err());
        assert!(parse_hex_query("A中").is_err());
        // from_str_radix 會接受正號，必須明確拒絕
        assert!(parse_hex_query("+1").is_err());
    }

    #[test]
    fn test_find_next_and_prev_navigation() {
        let lines = vec!["A B A C A".to_string()];
        let opt = SearchOptions::default();
        let matches = find_all_matches(&lines, "A", &opt).unwrap();
        assert_eq!(matches.len(), 3);

        // 游標在 (0, 0)，找下一個就是第 0 個
        let (idx, _) = find_next_match(&matches, TextPosition::new(0, 0)).unwrap();
        assert_eq!(idx, 0);

        // 游標在 (0, 1)，找下一個是第 1 個 (offset 4)
        let (idx, m) = find_next_match(&matches, TextPosition::new(0, 1)).unwrap();
        assert_eq!(idx, 1);
        assert_eq!(m.start, TextPosition::new(0, 4));

        // 游標在 (0, 9)，找下一個環狀回第 0 個
        let (idx, _) = find_next_match(&matches, TextPosition::new(0, 9)).unwrap();
        assert_eq!(idx, 0);

        // 游標在 (0, 9)，找上一個是第 2 個 (offset 8)
        let (idx, m) = find_prev_match(&matches, TextPosition::new(0, 9)).unwrap();
        assert_eq!(idx, 2);
        assert_eq!(m.start, TextPosition::new(0, 8));

        // 游標在 (0, 0)，找上一個環狀回最後一個 (idx 2)
        let (idx, _) = find_prev_match(&matches, TextPosition::new(0, 0)).unwrap();
        assert_eq!(idx, 2);
    }

    #[test]
    fn test_replace_all_matches() {
        let lines = vec![
            "foo bar foo".to_string(),
            "foo baz".to_string(),
        ];
        let opt = SearchOptions::default();
        let (replaced, count) = replace_all_matches(&lines, "foo", "qux", &opt).unwrap();
        assert_eq!(count, 3);
        assert_eq!(replaced, vec!["qux bar qux", "qux baz"]);
    }

    #[test]
    fn test_ue_flavor_pattern_translation() {
        let opt = || SearchOptions::new(false, false, true).with_flavor(RegexFlavor::UltraEdit);
        // ^t 是 tab、? 是單字元、* 是任意串
        let re = build_regex("co^tde", &opt()).unwrap();
        assert!(re.is_match("co\tde"));
        let re = build_regex("c?de", &opt()).unwrap();
        assert!(re.is_match("cxde") && !re.is_match("cde"));
        let re = build_regex("a*b", &opt()).unwrap();
        assert!(re.is_match("axxxb"));
        // UE 的 . 是字面句點
        let re = build_regex("a.b", &opt()).unwrap();
        assert!(re.is_match("a.b") && !re.is_match("axb"));
        // 字面括號；^( ) 才是群組
        let re = build_regex("(a)", &opt()).unwrap();
        assert!(re.is_match("(a)"));
        let re = build_regex("^(ab)", &opt()).unwrap();
        assert!(re.is_match("xabx"));
        // 行首 ^ 與行尾 $ 錨點
        let re = build_regex("^ab", &opt()).unwrap();
        assert!(re.is_match("ab"));
        let re = build_regex("ab$", &opt()).unwrap();
        assert!(re.is_match("ab"));
        // 字元集合與否定集合 [~…]
        let re = build_regex("[0-9]", &opt()).unwrap();
        assert!(re.is_match("x5"));
        let re = build_regex("[~0-9]", &opt()).unwrap();
        assert!(re.is_match("x") && !re.is_match("5"));
        // 逸出：\* 與 ^^
        let re = build_regex(r"a\*b", &opt()).unwrap();
        assert!(re.is_match("a*b"));
        let re = build_regex("a^^b", &opt()).unwrap();
        assert!(re.is_match("a^b"));
        // 反向引用查找：明確報錯而非靜默錯配
        assert!(build_regex("^(a)^1", &opt()).is_err());
    }

    #[test]
    fn test_ue_flavor_replacement_translation() {
        // ^1..^9、^0 群組引用；^t/^n 控制字元；字面 $ 逸出
        assert_eq!(translate_replacement("^1=^0", RegexFlavor::UltraEdit).unwrap(), "$1=$0");
        assert_eq!(translate_replacement("a^tb", RegexFlavor::UltraEdit).unwrap(), "a\tb");
        assert_eq!(translate_replacement("100$", RegexFlavor::UltraEdit).unwrap(), "100$$");
        assert_eq!(translate_replacement("$1", RegexFlavor::Perl).unwrap(), "$1");
        // ^c 剪貼板不支援
        assert!(translate_replacement("^c", RegexFlavor::UltraEdit).is_err());
        // 端到端：UE 查找 + UE 取代
        let lines = vec!["k1=v1 k2=v2".to_string()];
        let opt = SearchOptions::new(false, false, true).with_flavor(RegexFlavor::UltraEdit);
        let (replaced, count) = replace_all_matches(&lines, "^(k[0-9])=^(v[0-9])", "^2:^1", &opt).unwrap();
        assert_eq!(count, 2);
        assert_eq!(replaced, vec!["v1:k1 v2:k2"]);
    }

    #[test]
    fn test_unix_flavor_pattern_translation() {
        // POSIX 單詞邊界寫法轉成 \b；其餘 ERE 原生支援
        let opt = SearchOptions::new(false, false, true).with_flavor(RegexFlavor::Unix);
        let re = build_regex(r"[[:<:]]word[[:>:]]", &opt).unwrap();
        assert!(re.is_match("a word here") && !re.is_match("sword"));
        let re = build_regex(r"code \d+", &opt).unwrap();
        assert!(re.is_match("code 123"));
    }

    #[test]
    fn test_flavor_defaults_and_serde() {
        assert_eq!(RegexFlavor::default(), RegexFlavor::Perl);
        assert_eq!(SearchOptions::new(false, false, true).flavor, RegexFlavor::Perl);
        // 舊設定檔（無 flavor 欄位）與巨集皆可還原
        assert_eq!(serde_json::from_str::<RegexFlavor>("\"perl\"").unwrap(), RegexFlavor::Perl);
        assert_eq!(serde_json::from_str::<RegexFlavor>("\"ultraedit\"").unwrap(), RegexFlavor::UltraEdit);
        assert!(serde_json::from_str::<RegexFlavor>("\"bogus\"").is_err());
    }
}
