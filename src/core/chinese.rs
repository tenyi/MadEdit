//! 簡繁轉換與日文漢字轉換（對照表取自原版 MadEdit）

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use super::chinese_phrases::*;
use super::chinese_tables::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChineseConversion {
    SimpToTrad,
    TradToSimp,
    KanjiToTrad,
    KanjiToSimp,
    ChineseToKanji,
}

/// 依轉換方向與安全模式建立對照表（安全模式不轉換易誤轉的字，如「于」→「於」）
fn table(conv: ChineseConversion, safe: bool) -> &'static HashMap<char, char> {
    static CACHE: OnceLock<HashMap<(ChineseConversion, bool), HashMap<char, char>>> = OnceLock::new();
    let all = CACHE.get_or_init(|| {
        let build = |base: &[(u32, u32)], unsafe_extra: Option<&[(u32, u32)]>| {
            let mut m: HashMap<char, char> = base
                .iter()
                .filter_map(|&(a, b)| Some((char::from_u32(a)?, char::from_u32(b)?)))
                .collect();
            if let Some(extra) = unsafe_extra {
                for &(a, b) in extra {
                    if let (Some(a), Some(b)) = (char::from_u32(a), char::from_u32(b)) {
                        m.insert(a, b);
                    }
                }
            }
            m
        };
        let phrase_only = |mut m: HashMap<char, char>| {
            S2T_PHRASE_ONLY.chars().for_each(|c| {
                m.remove(&c);
            });
            m
        };
        let mut all = HashMap::new();
        for safe in [true, false] {
            let pick = |t: &'static [(u32, u32)]| (!safe).then_some(t);
            all.insert((ChineseConversion::SimpToTrad, safe), phrase_only(build(SIMP2TRAD, pick(SIMP2TRAD_UNSAFE))));
            all.insert((ChineseConversion::TradToSimp, safe), build(TRAD2SIMP, pick(TRAD2SIMP_UNSAFE)));
            all.insert((ChineseConversion::KanjiToTrad, safe), build(KANJI2TRAD, None));
            all.insert((ChineseConversion::KanjiToSimp, safe), build(KANJI2SIMP, None));
            all.insert((ChineseConversion::ChineseToKanji, safe), build(CHINESE2KANJI, None));
        }
        all
    });
    &all[&(conv, safe)]
}

struct PhraseSet {
    map: HashMap<&'static str, &'static str>,
    first: HashSet<char>,
    max_chars: usize,
}

impl PhraseSet {
    fn new(entries: &'static [(&'static str, &'static str)]) -> Self {
        Self {
            map: entries.iter().copied().collect(),
            first: entries.iter().filter_map(|(k, _)| k.chars().next()).collect(),
            max_chars: entries.iter().map(|(k, _)| k.chars().count()).max().unwrap_or(0),
        }
    }

    /// 回傳 `text` 開頭最長的詞組與其輸出
    fn longest(&self, text: &str) -> Option<(&'static str, &'static str)> {
        let mut ends = Vec::with_capacity(self.max_chars);
        for (i, c) in text.char_indices().take(self.max_chars) {
            ends.push(i + c.len_utf8());
        }
        ends.iter().skip(1).rev().find_map(|&e| self.map.get_key_value(&text[..e]).map(|(k, v)| (*k, *v)))
    }
}

/// 詞組表：只有簡轉繁與繁轉簡有歧義字需要處理
fn phrases(conv: ChineseConversion) -> Option<&'static PhraseSet> {
    static S2T: OnceLock<PhraseSet> = OnceLock::new();
    static T2S: OnceLock<PhraseSet> = OnceLock::new();
    match conv {
        ChineseConversion::SimpToTrad => Some(S2T.get_or_init(|| PhraseSet::new(S2T_PHRASES))),
        ChineseConversion::TradToSimp => Some(T2S.get_or_init(|| PhraseSet::new(T2S_PHRASES))),
        _ => None,
    }
}

/// 轉換文字，回傳（結果, 轉換字數）。詞組優先（最長匹配），其餘逐字轉換
pub fn convert(text: &str, conv: ChineseConversion, safe: bool) -> (String, usize) {
    let t = table(conv, safe);
    let ph = phrases(conv);
    let mut count = 0;
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if let Some((key, val)) = ph.filter(|p| p.first.contains(&c)).and_then(|p| p.longest(rest)) {
            for (k, v) in key.chars().zip(val.chars()) {
                let r = if v == '=' { t.get(&k).copied().unwrap_or(k) } else { v };
                count += usize::from(r != k);
                out.push(r);
            }
            rest = &rest[key.len()..];
            continue;
        }
        match t.get(&c) {
            Some(&r) => {
                count += 1;
                out.push(r);
            }
            None => out.push(c),
        }
        rest = &rest[c.len_utf8()..];
    }
    (out, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simp_trad_roundtrip() {
        assert_eq!(convert("简体中文", ChineseConversion::SimpToTrad, true).0, "簡體中文");
        assert_eq!(convert("繁體中文", ChineseConversion::TradToSimp, true).0, "繁体中文");
        // 安全模式不轉換「于」
        assert_eq!(convert("于", ChineseConversion::SimpToTrad, true).0, "于");
        assert_eq!(convert("于", ChineseConversion::SimpToTrad, false).0, "於");
    }

    #[test]
    fn test_phrase_table_is_well_formed() {
        for set in [S2T_PHRASES, T2S_PHRASES] {
            let mut seen = HashSet::new();
            for (k, v) in set {
                assert_eq!(k.chars().count(), v.chars().count(), "length mismatch: {k}");
                assert!(seen.insert(*k), "duplicate phrase: {k}");
            }
        }
    }

    #[test]
    fn test_phrases_resolve_ambiguous_chars() {
        let s2t = |s: &str, safe| convert(s, ChineseConversion::SimpToTrad, safe).0;
        for safe in [true, false] {
            assert_eq!(s2t("开发者预览软件，用于生产环境", safe), "開發者預覽軟件，用於生產環境");
            assert_eq!(s2t("头发和皇后", safe), "頭髮和皇后");
            assert_eq!(s2t("面条、方便面、方面", safe), "麵條、方便麵、方面");
            assert_eq!(s2t("干净、干部、干涉、饼干", safe), "乾淨、幹部、干涉、餅乾");
            assert_eq!(s2t("关系、联系、系统", safe), "關係、聯繫、系統");
            assert_eq!(s2t("一只猫，只有它", safe), "一隻貓，只有它");
            assert_eq!(s2t("复制、制度、控制", safe), "複製、制度、控制");
            assert_eq!(s2t("这里、公里", safe), "這裡、公里");
            assert_eq!(s2t("占领、占卜", safe), "佔領、占卜");
        }
        // 非安全模式的逐字預設仍適用於詞組以外
        assert_eq!(s2t("后", false), "後");
        assert_eq!(s2t("后", true), "后");
        assert_eq!(s2t("面", false), "面");
        let t2s = |s: &str| convert(s, ChineseConversion::TradToSimp, false).0;
        assert_eq!(t2s("乾坤乾淨"), "乾坤干净");
    }

    #[test]
    fn test_phrase_count_and_non_ascii_boundaries() {
        // 詞組在字串結尾、前後夾雜 ASCII / emoji 時不應出錯，計數只算實際改變的字
        let (out, n) = convert("a😀头发", ChineseConversion::SimpToTrad, true);
        assert_eq!(out, "a😀頭髮");
        assert_eq!(n, 2);
        assert_eq!(convert("头", ChineseConversion::SimpToTrad, true).0, "頭");
    }
}
