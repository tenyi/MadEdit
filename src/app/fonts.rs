//! 系統字型清單與 egui 字型設定（編輯區字型可由使用者任選，CJK 與 emoji 由後備字型補字）

use std::sync::Arc;

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};

/// 編輯區使用的字型家族名稱（egui 內部名稱）
pub const EDITOR_FAMILY: &str = "editor";

/// 未指定字型時，依序挑選第一個已安裝的等寬字型
const DEFAULT_EDITOR_FONTS: &[&str] = &[
    "Sarasa Term TC",
    "Sarasa Mono TC",
    "Sarasa Term SC",
    "Noto Sans Mono CJK TC",
    "Menlo",
    "SF Mono",
    "Consolas",
    "Cascadia Mono",
    "DejaVu Sans Mono",
    "Liberation Mono",
    "Courier New",
];

/// CJK 後備字型（介面與編輯區共用）
const CJK_FALLBACKS: &[&str] = &[
    "PingFang TC",
    "Heiti TC",
    "Hiragino Sans",
    "Microsoft JhengHei",
    "Microsoft YaHei",
    "Noto Sans CJK TC",
    "Noto Sans TC",
    "Source Han Sans TC",
    "WenQuanYi Micro Hei",
    "Droid Sans Fallback",
];

/// Hangul fallbacks, used only when the CJK fallback has no Hangul (PingFang TC, Microsoft
/// JhengHei, ...); otherwise 한국어 in the language menu draws as empty boxes. AppleGothic comes
/// before Apple SD Gothic Neo because it is about 15 MB instead of 55 MB.
const HANGUL_FALLBACKS: &[&str] = &[
    "AppleGothic",
    "Apple SD Gothic Neo",
    "Malgun Gothic",
    "Noto Sans CJK KR",
    "Noto Sans KR",
    "NanumGothic",
    "UnDotum",
];

/// Probe character for Hangul coverage.
const HANGUL_PROBE: char = '한';

/// 沒有 CJK 字型時，Mermaid 圖表改用的無襯線字型
const DIAGRAM_LATIN_FONTS: &[&str] = &["Inter", "Helvetica", "Arial", "Segoe UI", "DejaVu Sans", "Liberation Sans", "Noto Sans"];

pub struct FontManager {
    collection: fontique::Collection,
    /// 系統字型家族名稱（已排序）
    pub families: Vec<String>,
    /// 目前實際套用的編輯區字型
    pub editor_family: String,
    /// 目前實際套用的介面字型（空字串表示沿用 egui 內建字型）
    pub ui_family: String,
    cjk_family: Option<String>,
}

impl FontManager {
    pub fn new() -> Self {
        let mut collection = fontique::Collection::new(fontique::CollectionOptions {
            shared: false,
            system_fonts: true,
        });
        let mut families: Vec<String> = collection
            .family_names()
            .filter(|n| !n.starts_with('.'))
            .map(str::to_string)
            .collect();
        families.sort_by_key(|n| n.to_lowercase());
        families.dedup();
        let cjk_family = CJK_FALLBACKS
            .iter()
            .find(|n| collection.family_by_name(n).is_some())
            .map(|n| n.to_string());
        Self {
            collection,
            families,
            editor_family: String::new(),
            ui_family: String::new(),
            cjk_family,
        }
    }

    /// 預設的編輯區字型（第一個已安裝的偏好字型）
    pub fn default_editor_family(&mut self) -> String {
        DEFAULT_EDITOR_FONTS
            .iter()
            .find(|n| self.collection.family_by_name(n).is_some())
            .map(|n| n.to_string())
            .unwrap_or_default()
    }

    /// Mermaid 圖表用的單一字型：回傳（家族名稱, 字型檔內容）。
    /// 優先用 CJK 後備字型（同時含英文字母），沒有才找常見無襯線字型；只載入這一個，不掃描全部系統字型
    pub fn diagram_font(&mut self) -> Option<(String, fontique::Blob<u8>)> {
        let candidates: Vec<String> = self.cjk_family.iter().cloned().chain(DIAGRAM_LATIN_FONTS.iter().map(|s| s.to_string())).collect();
        for name in candidates {
            if let Some(font) = self.collection.family_by_name(&name).and_then(|f| f.default_font().cloned())
                && let Some(blob) = font.load(None)
            {
                return Some((name, blob));
            }
        }
        None
    }

    /// Loads the family's regular face and reports whether it has a Hangul glyph, read from
    /// the already loaded bytes so no font is loaded twice.
    fn load_family_probe(&mut self, name: &str) -> Option<(FontData, bool)> {
        let family = self.collection.family_by_name(name)?;
        let font = family.default_font()?;
        let blob = font.load(None)?;
        let has_hangul = font.charmap_index().charmap(blob.data()).and_then(|m| m.map(HANGUL_PROBE)).is_some_and(|gid| gid != 0);
        let mut data = FontData::from_owned(blob.data().to_vec());
        data.index = font.index();
        Some((data, has_hangul))
    }

    /// First installed Hangul fallback that really has Hangul glyphs.
    fn load_hangul_fallback(&mut self) -> Option<FontData> {
        HANGUL_FALLBACKS
            .iter()
            .filter_map(|name| self.load_family_probe(name))
            .find_map(|(data, has_hangul)| has_hangul.then_some(data))
    }

    /// 依設定的編輯區與介面字型重建 egui 字型定義；參數為空字串時自動（編輯區挑第一個
    /// 已安裝的等寬字型，介面沿用 egui 內建字型）。
    ///
    /// 介面字型只影響 [`FontFamily::Proportional`]（選單、對話框、分頁、狀態列等），
    /// 編輯區與「執行輸出」面板使用的是 [`EDITOR_FAMILY`]，不會被介面字型影響。
    pub fn apply(&mut self, ctx: &egui::Context, requested: &str, ui_requested: &str) {
        let mut defs = FontDefinitions::default();
        let mut editor_chain: Vec<String> = Vec::new();

        let wanted = if requested.is_empty() { self.default_editor_family() } else { requested.to_string() };
        self.editor_family = String::new();
        let mut editor_has_hangul = false;
        if !wanted.is_empty()
            && let Some((data, has_hangul)) = self.load_family_probe(&wanted)
        {
            defs.font_data.insert("editor-main".into(), Arc::new(data));
            editor_chain.push("editor-main".into());
            self.editor_family = wanted;
            editor_has_hangul = has_hangul;
        }

        // 編輯區字型與 CJK 後備字型相同時直接共用已載入的資料，介面仍須補上 CJK
        let mut cjk_has_hangul = false;
        let cjk_key = match self.cjk_family.clone() {
            Some(cjk) if cjk == self.editor_family => {
                cjk_has_hangul = editor_has_hangul;
                Some("editor-main")
            }
            Some(cjk) => self.load_family_probe(&cjk).map(|(data, has_hangul)| {
                defs.font_data.insert("cjk".into(), Arc::new(data));
                editor_chain.push("cjk".into());
                cjk_has_hangul = has_hangul;
                "cjk"
            }),
            None => None,
        };
        let mut ui_fallbacks: Vec<&str> = cjk_key.into_iter().collect();
        if !cjk_has_hangul && let Some(data) = self.load_hangul_fallback() {
            defs.font_data.insert("hangul".into(), Arc::new(data));
            editor_chain.push("hangul".into());
            ui_fallbacks.push("hangul");
        }
        // 介面字型：預設字型之後立即以 CJK 補字（emoji 仍排在最後）
        for fam in [FontFamily::Proportional, FontFamily::Monospace] {
            if let Some(list) = defs.families.get_mut(&fam) {
                for (i, key) in ui_fallbacks.iter().enumerate() {
                    list.insert((1 + i).min(list.len()), key.to_string());
                }
            }
        }

        // UI symbols such as ▾ ▼ ● exist only in egui's bundled monospace font (Hack), so it is the
        // last fallback for proportional UI text too; otherwise they draw as empty boxes.
        if let Some(list) = defs.families.get_mut(&FontFamily::Proportional)
            && !list.iter().any(|f| f == "Hack")
        {
            list.push("Hack".into());
        }

        // 介面字型：放在比例字型鏈最前面，CJK／emoji 後備字型仍接在後面補字。
        // 與編輯區或 CJK 後備字型相同時直接共用已載入的資料，不重複載入整個字型檔。
        self.ui_family = String::new();
        if !ui_requested.is_empty() {
            let shared = if self.editor_family == ui_requested {
                Some("editor-main")
            } else if self.cjk_family.as_deref() == Some(ui_requested) {
                cjk_key
            } else {
                None
            };
            let key = match shared {
                Some(key) => Some(key.to_string()),
                None => self.load_family_probe(ui_requested).map(|(data, _)| {
                    defs.font_data.insert("ui-main".into(), Arc::new(data));
                    "ui-main".to_string()
                }),
            };
            if let Some(key) = key {
                if let Some(list) = defs.families.get_mut(&FontFamily::Proportional) {
                    list.insert(0, key);
                }
                self.ui_family = ui_requested.to_string();
            }
        }

        // 編輯區字型鏈：使用者字型 → CJK → egui 內建等寬字型與 emoji
        if let Some(mono) = defs.families.get(&FontFamily::Monospace) {
            for f in mono {
                if !editor_chain.contains(f) {
                    editor_chain.push(f.clone());
                }
            }
        }
        defs.families.insert(FontFamily::Name(EDITOR_FAMILY.into()), editor_chain);
        ctx.set_fonts(defs);
    }
}

/// Whether the active editor font is proportional, judged by its own Latin glyphs: in a
/// monospaced font "i" and "M" have the same advance. Measured through egui, so it reflects
/// the font egui really draws with (fonts set by `FontManager::apply` count from the next frame).
pub fn editor_is_proportional(ctx: &egui::Context) -> bool {
    let font = egui::FontId::new(14.0, FontFamily::Name(EDITOR_FAMILY.into()));
    let (narrow, wide) = ctx.fonts_mut(|f| (f.glyph_width(&font, 'i'), f.glyph_width(&font, 'M')));
    (wide - narrow).abs() > 0.5
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 編輯區選用 CJK 後備字型本身（如 PingFang TC）時，介面字型仍須含 CJK 補字
    #[test]
    fn test_ui_keeps_cjk_when_editor_font_is_cjk_fallback() {
        let mut fm = FontManager::new();
        let Some(cjk) = fm.cjk_family.clone() else { return };
        let ctx = egui::Context::default();
        fm.apply(&ctx, &cjk, "");
        ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
        let ok = ctx.fonts(|f| f.definitions().families[&FontFamily::Proportional].contains(&"editor-main".to_string()));
        assert!(ok, "介面字型鏈缺少 CJK 字型");
    }

    /// The language menu always lists 한국어, so the UI font chain must cover Hangul whenever
    /// any Korean-capable font is installed, even if the CJK fallback (PingFang TC) lacks it.
    #[test]
    fn test_ui_draws_hangul_when_a_korean_font_is_installed() {
        let mut fm = FontManager::new();
        let installed = HANGUL_FALLBACKS.iter().any(|n| fm.collection.family_by_name(n).is_some());
        if !installed {
            return;
        }
        let ctx = egui::Context::default();
        fm.apply(&ctx, "", "");
        ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
        let ok = ctx.fonts_mut(|f| f.has_glyph(&egui::FontId::proportional(14.0), HANGUL_PROBE));
        assert!(ok, "UI font chain has no Hangul glyph");
    }

    /// 指定介面字型時，比例字型鏈最前面換成該字型；編輯區字型家族不受影響，
    /// 所以「執行輸出」與編輯區仍用編輯區字型。
    #[test]
    fn test_ui_font_goes_first_in_proportional_chain_only() {
        let mut fm = FontManager::new();
        // 找一個真的載入得到的系統字型
        let Some(name) = fm.families.clone().into_iter().find(|n| fm.load_family_probe(n).is_some()) else { return };
        let ctx = egui::Context::default();
        fm.apply(&ctx, "", &name);
        ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
        assert_eq!(fm.ui_family, name, "the UI font is reported as applied");
        let proportional = ctx.fonts(|f| f.definitions().families[&FontFamily::Proportional].clone());
        assert_eq!(proportional.first().map(String::as_str), Some("ui-main"), "UI font leads the proportional chain: {proportional:?}");
        let editor = ctx.fonts(|f| f.definitions().families[&FontFamily::Name(EDITOR_FAMILY.into())].clone());
        assert!(!editor.contains(&"ui-main".to_string()), "the editor family must ignore the UI font: {editor:?}");
    }

    /// 介面字型與編輯區字型相同時共用同一份字型資料，不重複載入字型檔
    #[test]
    fn test_ui_font_reuses_editor_font_data() {
        let mut fm = FontManager::new();
        let Some(name) = fm.families.clone().into_iter().find(|n| fm.load_family_probe(n).is_some()) else { return };
        let ctx = egui::Context::default();
        fm.apply(&ctx, &name, &name);
        ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
        assert_eq!(fm.ui_family, name);
        let (proportional, loaded_twice) = ctx.fonts(|f| {
            (f.definitions().families[&FontFamily::Proportional].clone(), f.definitions().font_data.contains_key("ui-main"))
        });
        assert_eq!(proportional.first().map(String::as_str), Some("editor-main"), "shared font data: {proportional:?}");
        assert!(!loaded_twice, "the same font must not be loaded twice");
    }
}
