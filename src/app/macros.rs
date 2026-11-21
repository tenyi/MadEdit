//! 巨集：錄製命令、編輯動作與搜尋，播放與以 JSON 檔案儲存／載入
//!
//! 原版以內嵌 Python 執行巨集腳本；重製版改為錄製「步驟清單」，不嵌入腳本引擎。

use std::path::PathBuf;

use eframe::egui;
use serde::{Deserialize, Serialize};

use super::commands::C;
use super::view::{self, EdAction, Layout};
use super::App;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MacroStep {
    /// 執行選單命令（以命令識別碼記錄）
    Command(String),
    /// 編輯區的輸入與游標動作
    Edit(EdAction),
    Find { query: String, hex: bool, match_case: bool, whole_word: bool, regex: bool, backward: bool },
    ReplaceAll { query: String, replacement: String, match_case: bool, whole_word: bool, regex: bool },
}

#[derive(Default)]
pub struct MacroState {
    pub recording: bool,
    pub steps: Vec<MacroStep>,
    /// 待播放的步驟（下一幀執行）
    pub queued: Option<Vec<MacroStep>>,
}

impl MacroState {
    pub fn record(&mut self, step: MacroStep) {
        if self.recording {
            // 連續輸入的文字合併成一步，讓巨集檔較易閱讀
            if let (MacroStep::Edit(EdAction::Insert(new)), Some(MacroStep::Edit(EdAction::Insert(last)))) = (&step, self.steps.last_mut()) {
                last.push_str(new);
                return;
            }
            self.steps.push(step);
        }
    }

    pub fn record_action(&mut self, a: EdAction) {
        self.record(MacroStep::Edit(a));
    }

    pub fn record_command(&mut self, c: C) {
        // 巨集相關命令本身不錄製
        if !matches!(c, C::StartRecMacro | C::StopRecMacro | C::PlayRecMacro | C::SaveRecMacro | C::RunTempMacro | C::LoadMacro | C::EditMacroFile) {
            self.record(MacroStep::Command(c.id().to_string()));
        }
    }
}

/// 巨集檔資料夾
pub fn macro_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("MadEdit").join("macros"))
}

/// 列出已儲存的巨集檔
pub fn list_macros() -> Vec<PathBuf> {
    let Some(dir) = macro_dir() else { return Vec::new() };
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json")).collect())
        .unwrap_or_default();
    v.sort();
    v
}

pub fn to_json(steps: &[MacroStep]) -> String {
    serde_json::to_string_pretty(steps).unwrap_or_default()
}

pub fn from_json(s: &str) -> Result<Vec<MacroStep>, String> {
    serde_json::from_str(s).map_err(|e| format!("巨集格式錯誤：{e}"))
}

impl App {
    /// 播放佇列中的巨集
    pub fn poll_macro(&mut self, ctx: &egui::Context) {
        // 沒有開啟的文件時不播放巨集：每個編輯步驟都需要目前分頁。
        if self.tabs.is_empty() {
            self.macros.queued = None;
            return;
        }
        let Some(steps) = self.macros.queued.take() else { return };
        let was_recording = std::mem::replace(&mut self.macros.recording, false);
        for step in steps {
            match step {
                MacroStep::Command(id) => match C::from_id(&id) {
                    Some(c) => self.run(ctx, c),
                    None => self.set_status(self.tf("msg.unknown_cmd", "巨集中有未知命令：{q}", None, Some(&id))),
                },
                MacroStep::Edit(a) => {
                    let lay = Layout::new(&self.settings, self.doc().mode);
                    let settings = self.settings.clone();
                    let tab = &mut self.tabs[self.active];
                    view::apply_action(&mut tab.doc, &mut tab.view, &a, &settings, &self.widths, &lay);
                }
                MacroStep::Find { query, hex, match_case, whole_word, regex, backward } => {
                    self.find.query = query;
                    self.find.hex = hex;
                    self.settings.search_match_case = match_case;
                    self.settings.search_whole_word = whole_word;
                    self.settings.search_regex = regex;
                    self.find_next(backward);
                }
                MacroStep::ReplaceAll { query, replacement, match_case, whole_word, regex } => {
                    self.find.query = query;
                    self.find.replacement = replacement;
                    self.find.hex = false;
                    self.settings.search_match_case = match_case;
                    self.settings.search_whole_word = whole_word;
                    self.settings.search_regex = regex;
                    self.replace_all();
                }
            }
        }
        self.macros.recording = was_recording;
    }

    pub fn play_macro(&mut self, steps: Vec<MacroStep>) {
        if steps.is_empty() {
            self.set_status(self.t("msg.no_macro", "沒有可播放的巨集"));
        } else {
            self.macros.queued = Some(steps);
        }
    }

    pub fn load_macro_file(&mut self, path: &std::path::Path) {
        match std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|s| from_json(&s)) {
            Ok(steps) => self.play_macro(steps),
            Err(e) => self.message("無法載入巨集", e),
        }
    }

    pub fn save_recorded_macro(&mut self) {
        if self.macros.steps.is_empty() {
            self.set_status(self.t("msg.no_recorded", "尚未錄製任何巨集"));
            return;
        }
        let title = self.t("msg.save_macro", "儲存巨集").to_string();
        let filter = self.t("msg.macro_filter", "MadEdit 巨集").to_string();
        let mut dlg = rfd::FileDialog::new().set_title(title).add_filter(filter, &["json"]).set_file_name("macro.json");
        if let Some(dir) = macro_dir() {
            let _ = std::fs::create_dir_all(&dir);
            dlg = dlg.set_directory(dir);
        }
        if let Some(p) = dlg.save_file() {
            if let Err(e) = std::fs::write(&p, to_json(&self.macros.steps)) {
                self.message("儲存巨集失敗", e.to_string());
            } else {
                let shown = p.display().to_string();
                self.set_status(self.tf("msg.macro_saved", "巨集已儲存：{q}", None, Some(&shown)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_macro_json_roundtrip_and_merge() {
        let mut m = MacroState { recording: true, ..Default::default() };
        m.record_action(EdAction::Insert("a".into()));
        m.record_action(EdAction::Insert("b".into()));
        m.record_command(C::ToUpperCase);
        m.record_command(C::StopRecMacro);
        assert_eq!(m.steps.len(), 2);
        let json = to_json(&m.steps);
        assert_eq!(from_json(&json).unwrap(), m.steps);
        assert_eq!(m.steps[0], MacroStep::Edit(EdAction::Insert("ab".into())));
    }
}
