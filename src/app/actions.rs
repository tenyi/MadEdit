//! 命令的實作（選單、工具列、快捷鍵與巨集都經由 `App::run` 執行）

use eframe::egui;

use super::commands::C;
use super::dialogs::Dialog;
use super::view::{self, selected_line_range};
use super::{i18n, print, App, Confirm};
use crate::core::chinese::{self, ChineseConversion};
use crate::core::document::EditKind;
use crate::core::dprint;
use crate::core::editor::{extract_visual_slice, EditorState, Selection, TextPosition};
use crate::core::encoding;
use crate::core::model::{EditMode, NewLineFormat};
use crate::core::settings::{SortOptions, WrapMode};
use crate::core::syntax;
use crate::core::textops;

/// 文字轉換的套用範圍（無選取時）
#[derive(Clone, Copy, PartialEq)]
enum Scope {
    /// 游標所在的字詞
    Word,
    /// 整份文件
    Whole,
}

/// View commands that flip a saved preference. They are written to the settings file at once,
/// like the theme, so the choice survives even when the app does not exit normally.
fn saves_view_preference(c: C) -> bool {
    matches!(
        c,
        C::AlwaysOnTop
            | C::ZoomIn
            | C::ZoomOut
            | C::FixedWidthMode
            | C::NoWrap
            | C::WrapByWindow
            | C::WrapByColumn
            | C::DisplayLineNumber
            | C::DisplayBookmark
            | C::Display80ColHint
            | C::ShowRuler
            | C::ShowEndOfLine
            | C::ShowTabChar
            | C::ShowSpaceChar
            | C::ShowAllChar
            | C::RightToLeft
            | C::MarkActiveLine
            | C::MarkBracePair
            | C::SpellCheck
            | C::TypewriterMode
            | C::ToggleToolbar
            | C::ToggleMenubar
            | C::ToggleStatusbar
    )
}

/// 以新內容取代列塊選取範圍：自 `first_line` 起逐行，將 [min_vcol, max_vcol) 的片段換成 `texts` 的對應項
fn insert_block(ed: &mut EditorState, first_line: usize, min_vcol: usize, texts: Vec<String>) {
    let b = ed.selection.visual_bounds(ed);
    for (k, text) in texts.into_iter().enumerate() {
        let l = first_line + k;
        if l >= ed.lines.len() {
            break;
        }
        let (_, s, e) = extract_visual_slice(&ed.lines[l], min_vcol, b.max_vcol.max(min_vcol));
        let chars: Vec<char> = ed.lines[l].chars().collect();
        let mut out: String = chars[..s].iter().collect();
        out.push_str(&text);
        out.extend(&chars[e..]);
        ed.lines[l] = out;
    }
    ed.clamp_caret();
}

impl App {
    /// 將第 `idx` 個分頁的文件轉為 HTML，交給系統瀏覽器並自動開啟列印對話框
    fn print_doc(&mut self, idx: usize) {
        let Some(tab) = self.tabs.get(idx) else { return };
        let doc = &tab.doc;
        let html = print::document_html(doc, &self.settings.print, &self.fonts.editor_family, self.settings.font_size, true);
        if let Err(e) = print::open_html(&doc.display_name(), &html) {
            self.message("列印", e);
        }
    }

    /// 顯示預覽列印視窗（獨立原生視窗；後端不支援多視窗時改為內嵌視窗）
    pub fn show_print_preview(&mut self, ctx: &egui::Context) {
        let Some(mut pv) = self.preview.take() else { return };
        // 預覽中的文件已被關閉時，一併關閉預覽
        let Some(idx) = self.tabs.iter().position(|t| t.doc.id == pv.doc_id) else { return };
        let lang = self.ui_lang();
        let title = i18n::fill(i18n::translate(lang, "pv.title", "預覽列印 - {q}"), None, Some(&pv.title));
        let builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size([880.0, 960.0])
            .with_min_inner_size([480.0, 360.0]);
        let action = ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("madedit-print-preview"), builder, |ui, _class| pv.ui(ui, lang));
        match action {
            super::preview::PreviewAction::None => self.preview = Some(pv),
            super::preview::PreviewAction::Print => self.print_doc(idx),
            super::preview::PreviewAction::Close => {}
        }
    }

    /// 顯示 Markdown 預覽分頁或並排面板，內容隨文件版本與深淺色主題自動更新；
    /// 並排面板必須在 CentralPanel 之前呼叫
    pub fn show_markdown_preview(&mut self, ui: &mut egui::Ui) {
        use super::mdpreview::Placement;
        let ctx = ui.ctx().clone();
        let lang = self.ui_lang();
        let tab = self.tab_mut();
        let Some(mut pv) = tab.md_preview.take() else { return };
        // 相對路徑圖片以文件所在資料夾為基準
        let base_dir = tab.doc.path.as_ref().and_then(|p| p.parent()).map(|d| d.to_path_buf());
        pv.refresh(&ctx, &tab.doc.text(), tab.doc.version, Self::is_dark(&ctx), base_dir);
        let (top_line, fraction) = tab.view.top_line(&tab.doc.editor);
        // 編輯區（扣除選單、工具列、狀態列後）剩下的範圍，並排時各佔一半
        let area = ui.available_rect_before_wrap();
        let action = match pv.placement {
            Placement::Tab => {
                pv.ui(ui, top_line, fraction, lang)
            }
            placement => {
                // 左右與上下用不同的面板識別，各自記住使用者拖曳後的大小
                let (panel, half) = if placement == Placement::Right {
                    (egui::Panel::right("md-preview-right"), area.width() / 2.0)
                } else {
                    (egui::Panel::bottom("md-preview-bottom"), area.height() / 2.0)
                };
                let mut panel = panel.resizable(true).default_size(half).min_size(160.0);
                // 剛切換成並排時強制設為一半，不沿用先前的大小；之後使用者可拖曳邊界調整
                let available = half * 2.0;
                let keep_half = pv.dock_geometry.is_some_and(|(old_available, old_size)| {
                    (available - old_available).abs() > 0.5 && (old_size - old_available / 2.0).abs() < 1.0
                });
                if std::mem::take(&mut pv.reset_size) || keep_half {
                    panel = panel.exact_size(half);
                }
                if let Some(size) = pv.resize_to.take() {
                    panel = panel.exact_size(size);
                }
                let shown = panel.show(ui, |ui| {
                    // 預覽只使用面板分配的範圍；寬表格、程式碼或圖表不能把內容需求
                    // 回報成面板的新尺寸，否則下一幀會覆蓋各半或手動拖曳後的大小。
                    let rect = ui.available_rect_before_wrap();
                    let mut content = ui.new_child(egui::UiBuilder::new().id_salt("md-preview-content").max_rect(rect));
                    content.shrink_clip_rect(rect);
                    let action = pv.ui(&mut content, top_line, fraction, lang);
                    ui.advance_cursor_after_rect(rect);
                    action
                });
                if pv.placement == placement {
                    let size = if placement == Placement::Right { shown.response.rect.width() } else { shown.response.rect.height() };
                    pv.dock_geometry = Some((available, size));
                    let pane = shown.response.rect;
                    let edge = if placement == Placement::Right {
                        egui::Rect::from_min_max(pane.min, egui::pos2(pane.left(), pane.bottom()))
                    } else { egui::Rect::from_min_max(pane.min, egui::pos2(pane.right(), pane.top())) };
                    let edge = edge.expand(ui.style().interaction.resize_grab_radius_side);
                    if ctx.input(|i| i.pointer.primary_pressed()) && ui.rect_contains_pointer(edge) {
                        pv.resize_started = true;
                    }
                    // 快速移動與放開可能落在同一幀，egui 此時未必回報 drag_stopped；
                    // 只要先前確實按在分隔線上，就依放開位置保存尺寸，再交由面板正常記憶。
                    if ctx.input(|i| i.pointer.primary_released())
                        && std::mem::take(&mut pv.resize_started)
                        && let Some(pos) = ctx.input(|i| i.pointer.interact_pos())
                    {
                        let size = if placement == Placement::Right { area.right() - pos.x } else { area.bottom() - pos.y };
                        pv.resize_to = Some(size.clamp(160.0_f32.min(available), available));
                        ctx.request_repaint();
                    }
                }
                shown.inner
            }
        };
        if let super::mdpreview::MdAction::ScrollTo { line, fraction } = action {
            let tab = self.tab_mut();
            tab.view.scroll_to_source(&tab.doc.editor, line, fraction);
            let (line, fraction) = tab.view.top_line(&tab.doc.editor);
            pv.acknowledge_scroll(line, fraction);
            ctx.request_repaint();
        }
        if action != super::mdpreview::MdAction::Close {
            self.preview_tab_active = pv.placement == Placement::Tab;
            self.tab_mut().md_preview = Some(pv);
        } else {
            self.preview_tab_active = false;
            self.focus_editor = true;
        }
    }

    /// 執行命令
    pub fn run(&mut self, ctx: &egui::Context, c: C) {
        // The Options dialog holds unapplied changes, so a command that opens another dialog
        // (F1, F9, a menu item) must not replace it and silently drop them. Options stays
        // open instead; F1 shows its Keys tab, where shortcuts are listed and changed.
        let options = match self.dialog.take() {
            Some(Dialog::Options(st)) => Some(st),
            other => {
                self.dialog = other;
                None
            }
        };
        self.dispatch(ctx, c);
        if saves_view_preference(c) {
            self.save_settings();
        }
        if let Some(mut st) = options {
            if c == C::KeyboardShortcuts {
                st.tab = super::dialogs::OPTIONS_KEYS_TAB;
            } else if !matches!(self.dialog, None | Some(Dialog::Options(_))) {
                // Say why the dialog the command asked for did not appear.
                self.set_status(self.t("status.options_open", "請先關閉選項對話框"));
            }
            self.dialog = Some(Dialog::Options(st));
        }
    }

    fn dispatch(&mut self, ctx: &egui::Context, c: C) {
        if self.macros.recording {
            self.macros.record_command(c);
        }
        let editable = !self.doc().read_only && !self.is_preview_tab();
        let hex = self.doc().mode == EditMode::Hex;
        match c {
            // ── 檔案 ──
            C::New => self.new_document(),
            C::Open => self.open_dialog(),
            C::Save => {
                let i = self.active;
                self.save_tab(i, false);
            }
            C::SaveAs => {
                let i = self.active;
                self.save_tab(i, true);
            }
            C::SaveACopy => {
                let title = self.t("msg.save_copy", "另存副本").to_string();
                let fail = self.t("msg.save_fail", "存檔失敗").to_string();
                if let Some(p) = rfd::FileDialog::new().set_title(&title).set_file_name(self.doc().display_name()).save_file().map(Self::trim_file_name) {
                    match self.doc().save_copy(&p) {
                        Ok(()) => {
                            let shown = p.display().to_string();
                            self.set_status(self.tf("msg.saved_copy", "已另存副本 {q}", None, Some(&shown)));
                        }
                        Err(e) => self.message(&fail, e.to_string()),
                    }
                }
            }
            C::SaveAll => {
                for i in 0..self.tabs.len() {
                    if self.tabs[i].doc.is_modified() {
                        self.activate(i);
                        self.active = i;
                        if !self.save_tab(i, false) {
                            break;
                        }
                    }
                }
            }
            C::Reload => {
                let id = self.doc().id;
                if self.doc().path.is_none() {
                    return;
                }
                if self.doc().is_modified() {
                    self.confirm = Some(Confirm::ReloadModified { id });
                } else {
                    self.reload_tab(id);
                }
            }
            C::Close => {
                if self.is_preview_tab() {
                    self.tab_mut().md_preview = None;
                    self.preview_tab_active = false;
                    self.focus_editor = true;
                } else {
                    let id = self.doc().id;
                    self.request_close(vec![id], false);
                }
            }
            C::CloseAll => {
                let ids = self.tabs.iter().map(|t| t.doc.id).collect();
                self.request_close(ids, false);
            }
            C::CloseOthers | C::CloseLeft | C::CloseRight => {
                let a = self.active;
                let ids = self
                    .tabs
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| match c {
                        C::CloseOthers => *i != a,
                        C::CloseLeft => *i < a,
                        _ => *i > a,
                    })
                    .map(|(_, t)| t.doc.id)
                    .collect();
                self.request_close(ids, false);
            }
            C::ToggleReadOnly => {
                if let Err(e) = self.doc_mut().toggle_read_only() {
                    self.message("無法變更唯讀屬性", e.to_string());
                }
            }
            C::PageSetup => self.dialog = Some(Dialog::PageSetup),
            C::PrintPreview => {
                // 於 App 內開啟獨立的預覽列印視窗
                let doc = self.doc();
                let content = print::build_content(doc, &self.settings.print);
                self.preview = Some(super::preview::PrintPreview::new(doc.id, doc.display_name(), content, self.settings.font_size));
            }
            C::Print => self.print_doc(self.active),
            C::RecentFilesList => self.dialog = Some(Dialog::RecentFiles { filter: String::new() }),
            C::Exit => {
                let ids = self.tabs.iter().map(|t| t.doc.id).collect();
                self.request_close(ids, true);
            }

            // ── 編輯 ──
            C::Undo if !self.is_preview_tab() => {
                self.doc_mut().undo();
                self.tab_mut().view.ensure_visible = true;
            }
            C::Redo if !self.is_preview_tab() => {
                self.doc_mut().redo();
                self.tab_mut().view.ensure_visible = true;
            }
            C::Copy => self.copy(),
            C::Cut => {
                if editable {
                    self.copy();
                    self.delete_selection();
                }
            }
            C::Paste => {
                if editable {
                    self.paste();
                }
            }
            C::Delete => {
                if editable {
                    self.delete_selection();
                }
            }
            C::CutLine | C::DeleteLine | C::DuplicateLine | C::MoveLinesUp | C::MoveLinesDown if !hex && editable => {
                let doc = self.doc_mut();
                let (a, b) = selected_line_range(&doc.editor);
                let hint = a.saturating_sub(1)..(b + 2).min(doc.editor.lines.len());
                if c == C::CutLine {
                    let text: String = doc.editor.lines[a..=b].iter().map(|l| format!("{l}\n")).collect();
                    doc.editor.clipboard = text.clone();
                    doc.editor.clipboard_is_block = false;
                    self.copy_to_system(&text);
                }
                let doc = self.doc_mut();
                let full = doc.whole_hint();
                doc.edit(EditKind::Other, if c == C::DuplicateLine { full } else { hint }, |ed| {
                    if a != b || !ed.selection.is_empty() {
                        // 讓行操作涵蓋完整的選取行
                        ed.selection = Selection::new(TextPosition::new(a, 0), TextPosition::new(b, ed.line_char_count(b)));
                    }
                    match c {
                        C::CutLine | C::DeleteLine => ed.delete_lines(),
                        C::DuplicateLine => ed.duplicate_lines(),
                        C::MoveLinesUp => {
                            ed.move_lines_up();
                        }
                        _ => {
                            ed.move_lines_down();
                        }
                    }
                });
            }
            C::JoinLines if !hex && editable => {
                let doc = self.doc_mut();
                let (a, mut b) = selected_line_range(&doc.editor);
                if a == b {
                    b = (a + 1).min(doc.editor.lines.len() - 1);
                }
                doc.edit(EditKind::Other, a..b + 1, |ed| {
                    let joined: String = ed.lines[a..=b].concat();
                    let caret = ed.lines[a].chars().count();
                    ed.lines.splice(a..=b, [joined]);
                    // 被合併的行上的書籤移除，之後的書籤前移
                    let n = b - a;
                    ed.bookmarks = ed.bookmarks.iter().filter(|&&l| l <= a || l > b).map(|&l| if l > b { l - n } else { l }).collect();
                    ed.set_caret(a, caret);
                });
            }
            C::DeleteEmptyLines | C::DeleteEmptyLinesWithSpaces if !hex && editable => {
                let spaces = c == C::DeleteEmptyLinesWithSpaces;
                self.transform_lines(|lines| textops::delete_empty_lines(lines, spaces));
            }
            C::TrimTrailingSpaces if !hex && editable => self.transform_lines(|ls| ls.iter().map(|l| textops::trim_trailing(l)).collect()),
            C::TrimLeadingSpaces if !hex && editable => self.transform_lines(|ls| ls.iter().map(|l| textops::trim_leading(l)).collect()),
            C::SelectAll => {
                if hex {
                    if let Some(hb) = self.doc_mut().hex.as_mut() {
                        hb.move_to(0, false);
                        let len = hb.len();
                        hb.move_to(len.saturating_sub(1), true);
                    }
                } else {
                    self.doc_mut().editor.select_all();
                }
            }
            C::StartEndSelecting => {
                let doc = self.doc_mut();
                doc.selecting_mode = !doc.selecting_mode;
            }
            C::InsertTabChar if !hex && editable => self.insert_text("\t"),
            C::InsertDateTime if !hex && editable => {
                let s = chrono::Local::now().format(&self.settings.date_time_format).to_string();
                self.insert_text(&s);
            }
            C::BookmarkCopy | C::BookmarkCut | C::BookmarkDel | C::BookmarkCopyUnmarked | C::BookmarkCutUnmarked | C::BookmarkDelUnmarked if !hex => {
                let marked = matches!(c, C::BookmarkCopy | C::BookmarkCut | C::BookmarkDel);
                let copy = matches!(c, C::BookmarkCopy | C::BookmarkCut | C::BookmarkCopyUnmarked | C::BookmarkCutUnmarked);
                let remove = !matches!(c, C::BookmarkCopy | C::BookmarkCopyUnmarked);
                let doc = self.doc_mut();
                let pick = |l: usize, bm: &std::collections::BTreeSet<usize>| bm.contains(&l) == marked;
                if copy {
                    let text: String = doc.editor.lines.iter().enumerate().filter(|(i, _)| pick(*i, &doc.editor.bookmarks)).map(|(_, l)| format!("{l}\n")).collect();
                    doc.editor.clipboard = text.clone();
                    doc.editor.clipboard_is_block = false;
                    self.copy_to_system(&text);
                }
                if remove && editable {
                    let doc = self.doc_mut();
                    let hint = doc.whole_hint();
                    doc.edit(EditKind::Other, hint, |ed| {
                        let bm = ed.bookmarks.clone();
                        let kept: Vec<String> = ed.lines.iter().enumerate().filter(|(i, _)| !pick(*i, &bm)).map(|(_, l)| l.clone()).collect();
                        ed.lines = if kept.is_empty() { vec![String::new()] } else { kept };
                        ed.bookmarks = if marked {
                            Default::default()
                        } else {
                            (0..bm.len().min(ed.lines.len())).collect()
                        };
                        ed.clamp_caret();
                    });
                }
            }
            C::BookmarkReplace if !hex && editable => {
                self.sync_clipboard_from_system();
                let doc = self.doc_mut();
                let clip = doc.editor.clipboard.trim_end_matches('\n').to_string();
                let hint = doc.whole_hint();
                doc.edit(EditKind::Other, hint, |ed| {
                    let bm: Vec<usize> = ed.bookmarks.iter().copied().collect();
                    for &l in &bm {
                        if l < ed.lines.len() {
                            ed.lines[l] = clip.clone();
                        }
                    }
                });
            }
            C::CopyAsHexString | C::CopyAsHexStringWithSpace => {
                let bytes = if hex {
                    self.doc().hex.as_ref().map(|h| h.selected_bytes()).unwrap_or_default()
                } else {
                    encoding::encode_text(&self.doc().editor.selected_text(), self.doc().encoding, false).unwrap_or_default()
                };
                let s = textops::to_hex_string(&bytes, c == C::CopyAsHexStringWithSpace);
                self.copy_to_system(&s);
            }
            C::CopyRevertHex => {
                let sel = self.doc().editor.selected_text();
                match textops::from_hex_string(&sel) {
                    Some(bytes) => {
                        let d = encoding::decode_with(&bytes, self.doc().encoding);
                        self.copy_to_system(&d.text);
                    }
                    None => self.set_status(self.t("msg.bad_hex", "選取的內容不是有效的十六進位字串")),
                }
            }
            C::IncreaseIndent | C::DecreaseIndent if !hex && editable => {
                let settings = self.settings.clone();
                view::indent_selection(self.doc_mut(), &settings, c == C::IncreaseIndent);
            }
            C::Comment | C::Uncomment if !hex && editable => self.comment(c == C::Comment),
            C::WordWrapToNewLine if !hex && editable => self.wrap_to_newlines(),
            C::NewLineToWordWrap if !hex && editable => {
                let doc = self.doc_mut();
                let (a, b) = selected_line_range(&doc.editor);
                if a < b {
                    doc.edit(EditKind::Other, a..b + 1, |ed| {
                        let joined = ed.lines[a..=b].join(" ");
                        ed.lines.splice(a..=b, [joined]);
                        let n = b - a;
                        ed.bookmarks = ed.bookmarks.iter().filter(|&&l| l <= a || l > b).map(|&l| if l > b { l - n } else { l }).collect();
                        ed.set_caret(a, 0);
                    });
                    if self.settings.wrap_mode == WrapMode::NoWrap {
                        self.settings.wrap_mode = WrapMode::ByWindow;
                    }
                }
            }
            C::ToUpperCase if !hex && editable => self.transform(Scope::Word, &|s| Ok(textops::to_upper(s))),
            C::ToLowerCase if !hex && editable => self.transform(Scope::Word, &|s| Ok(textops::to_lower(s))),
            C::InvertCase if !hex && editable => self.transform(Scope::Word, &|s| Ok(textops::invert_case(s))),
            C::Capitalize if !hex && editable => self.transform(Scope::Word, &|s| Ok(textops::capitalize(s))),
            C::ToHalfWidth if !hex && editable => {
                let o = crate::core::settings::WidthOptions::default();
                self.transform(Scope::Whole, &|s| Ok(textops::to_halfwidth(s, &o)))
            }
            C::ToFullWidth if !hex && editable => {
                let o = crate::core::settings::WidthOptions::default();
                self.transform(Scope::Whole, &|s| Ok(textops::to_fullwidth(s, &o)))
            }
            C::ToHalfWidthByOptions => self.dialog = Some(Dialog::Width { full: false }),
            C::ToFullWidthByOptions => self.dialog = Some(Dialog::Width { full: true }),
            C::TabToSpace if !hex && editable => self.transform_lines(|ls| ls.iter().map(|l| textops::tabs_to_spaces(l)).collect()),
            C::SpaceToTab if !hex && editable => self.transform_lines(|ls| ls.iter().map(|l| textops::spaces_to_tabs(l)).collect()),
            C::InsertNumbers if !hex && editable => self.dialog = Some(Dialog::InsertNumbers),
            C::ColumnAlignLeft | C::ColumnAlignRight if !hex && editable => {
                let right = c == C::ColumnAlignRight;
                if !self.doc().editor.selection.is_block() {
                    self.set_status(self.t("msg.need_column", "請先以列塊模式選取要對齊的範圍"));
                    return;
                }
                let doc = self.doc_mut();
                let hint = doc.selection_hint();
                doc.edit(EditKind::Other, hint, |ed| {
                    let b = ed.selection.visual_bounds(ed);
                    let slices: Vec<String> = (b.min_line..=b.max_line).map(|l| extract_visual_slice(&ed.lines[l], b.min_vcol, b.max_vcol).0).collect();
                    let aligned = textops::align_column(&slices, right);
                    insert_block(ed, b.min_line, b.min_vcol, aligned);
                });
            }
            C::SortAscending | C::SortDescending | C::SortAscendingCase | C::SortDescendingCase | C::SortByOptions if !hex && editable => {
                let opts = match c {
                    C::SortByOptions => self.settings.sort.clone(),
                    _ => SortOptions {
                        descending: matches!(c, C::SortDescending | C::SortDescendingCase),
                        case_sensitive: matches!(c, C::SortAscendingCase | C::SortDescendingCase),
                        ..Default::default()
                    },
                };
                self.transform_lines(|ls| {
                    let mut v = ls.to_vec();
                    textops::sort_lines(&mut v, &opts);
                    v
                });
            }
            C::SortOptions => self.dialog = Some(Dialog::Sort),

            // ── 搜尋 ──
            C::Find | C::QuickFind | C::Replace => {
                self.find.visible = true;
                self.find.replace = c == C::Replace;
                self.find.focus = true;
                self.goto.visible = false;
                let sel = self.doc().editor.selected_text();
                if !sel.is_empty() && !sel.contains('\n') && !hex {
                    self.find.query = sel;
                }
            }
            C::FindNext => {
                self.find_next(false);
            }
            C::FindPrevious => {
                self.find_next(true);
            }
            C::FindInFiles => {
                if self.settings.find_in_files.directory.is_empty()
                    && let Some(d) = self.doc().path.as_ref().and_then(|p| p.parent())
                {
                    self.settings.find_in_files.directory = d.display().to_string();
                }
                let q = if self.find.query.is_empty() { self.doc().editor.selected_text() } else { self.find.query.clone() };
                self.dialog = Some(Dialog::FindInFiles { query: q, replacement: self.find.replacement.clone() });
            }
            C::ShowFindInFilesResults => self.fif.show_results = !self.fif.show_results,
            C::CollapseAllResults => self.fif.collapsed = !self.fif.collapsed,
            C::GoToLine | C::GoToPosition => {
                self.goto.visible = true;
                self.goto.by_position = c == C::GoToPosition;
                self.goto.focus = true;
                self.goto.error = None;
                self.find.visible = false;
            }
            C::GoBack => {
                if self.doc_mut().go_back() {
                    self.tab_mut().view.ensure_visible = true;
                }
            }
            C::GoForward => {
                if self.doc_mut().go_forward() {
                    self.tab_mut().view.ensure_visible = true;
                }
            }
            C::LeftBrace | C::RightBrace if !hex => {
                let doc = self.doc_mut();
                let head = doc.editor.selection.head;
                if let Some(p) = textops::enclosing_brace(&doc.editor.lines, head, c == C::LeftBrace) {
                    doc.record_jump(head);
                    doc.editor.set_caret(p.line, p.col);
                    self.tab_mut().view.ensure_visible = true;
                }
            }
            C::ToggleBookmark if !hex => {
                self.doc_mut().editor.toggle_bookmark();
            }
            C::NextBookmark | C::PreviousBookmark if !hex => {
                let doc = self.doc_mut();
                let head = doc.editor.selection.head;
                let moved = if c == C::NextBookmark { doc.editor.next_bookmark() } else { doc.editor.prev_bookmark() };
                if moved {
                    doc.record_jump(head);
                    self.tab_mut().view.ensure_visible = true;
                }
            }
            C::ClearAllBookmarks => self.doc_mut().editor.clear_all_bookmarks(),

            // ── 檢視 ──
            C::AlwaysOnTop => {
                self.settings.always_on_top = !self.settings.always_on_top;
                let level = if self.settings.always_on_top { egui::WindowLevel::AlwaysOnTop } else { egui::WindowLevel::Normal };
                ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
            }
            C::FullScreen => {
                self.fullscreen = !self.fullscreen;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
            }
            C::PostIt => {
                self.post_it = !self.post_it;
                let level = if self.post_it || self.settings.always_on_top { egui::WindowLevel::AlwaysOnTop } else { egui::WindowLevel::Normal };
                ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
                if self.post_it {
                    self.set_status(self.t("msg.sticky", "便利貼模式：按 F12 恢復"));
                }
            }
            C::SetFont => self.dialog = Some(Dialog::Font { filter: String::new(), family: self.settings.font_family.clone(), size: self.settings.font_size }),
            C::ZoomIn => self.settings.font_size = (self.settings.font_size + 1.0).min(72.0),
            C::ZoomOut => self.settings.font_size = (self.settings.font_size - 1.0).max(6.0),
            C::FixedWidthMode => self.settings.fixed_width = !self.settings.fixed_width,
            C::NoWrap => self.settings.wrap_mode = WrapMode::NoWrap,
            C::WrapByWindow => self.settings.wrap_mode = WrapMode::ByWindow,
            C::WrapByColumn => self.settings.wrap_mode = WrapMode::ByColumn,
            C::DisplayLineNumber => self.settings.show_line_numbers = !self.settings.show_line_numbers,
            C::DisplayBookmark => self.settings.show_bookmarks = !self.settings.show_bookmarks,
            C::Display80ColHint => self.settings.show_col_hint = !self.settings.show_col_hint,
            C::ShowRuler => self.settings.show_ruler = !self.settings.show_ruler,
            C::ShowEndOfLine => self.settings.show_eol = !self.settings.show_eol,
            C::ShowTabChar => self.settings.show_tab = !self.settings.show_tab,
            C::ShowSpaceChar => self.settings.show_space = !self.settings.show_space,
            C::ShowAllChar => {
                let s = &mut self.settings;
                let on = !(s.show_eol && s.show_tab && s.show_space);
                s.show_eol = on;
                s.show_tab = on;
                s.show_space = on;
            }
            C::RightToLeft => self.settings.right_to_left = !self.settings.right_to_left,
            C::MarkActiveLine => self.settings.mark_active_line = !self.settings.mark_active_line,
            C::MarkBracePair => self.settings.mark_brace_pair = !self.settings.mark_brace_pair,
            C::SpellCheck => {
                self.settings.spell_check = !self.settings.spell_check;
                if self.settings.spell_check && self.spell.is_none() {
                    self.load_spell();
                }
            }
            C::TypewriterMode => self.settings.typewriter_mode = !self.settings.typewriter_mode,
            C::ToggleToolbar => self.settings.show_toolbar = !self.settings.show_toolbar,
            C::ToggleMenubar => {
                self.settings.show_menubar = !self.settings.show_menubar;
                if !self.settings.show_menubar {
                    let key = self.shortcut_text(C::ToggleMenubar, ctx);
                    self.set_status(self.tf("msg.menu_hidden", "選單列已隱藏，按 {q} 或在工具列按右鍵可再次顯示", None, Some(&key)));
                }
            }
            C::ToggleStatusbar => self.settings.show_statusbar = !self.settings.show_statusbar,
            C::ToggleTheme => self.toggle_theme(ctx),
            C::TextMode | C::ColumnMode | C::HexMode => {
                let mode = match c {
                    C::TextMode => EditMode::Text,
                    C::ColumnMode => EditMode::Column,
                    _ => EditMode::Hex,
                };
                let doc = self.doc_mut();
                if let Err(e) = doc.set_mode(mode) {
                    self.message("無法切換模式", e);
                } else {
                    if mode != EditMode::Column && doc.editor.selection.is_block() && doc.editor.selection.is_empty() {
                        let h = doc.editor.selection.head;
                        doc.editor.selection = Selection::caret(h);
                    }
                    self.tab_mut().view.ensure_visible = true;
                    self.focus_editor = true;
                }
            }
            // 非 Markdown 檔不開預覽（選單與工具列已停用，這裡擋住快捷鍵與巨集）
            C::PreviewMarkdown if self.doc().is_markdown() => {
                // 於 App 內開啟 Markdown 預覽（含 Mermaid 圖表）。
                // 圖片載入器延到第一次預覽才安裝，不拖慢啟動；重複呼叫無妨。
                // 圖表載入器要後裝，egui 會先問後裝的載入器
                egui_extras::install_image_loaders(ctx);
                if !ctx.is_loader_installed(super::mdpreview::DiagramLoader::ID) {
                    super::mdpreview::DiagramLoader::install(ctx, self.fonts.diagram_font());
                }
                let pv = self.tab_mut().md_preview.get_or_insert_with(super::mdpreview::MarkdownPreview::new);
                if pv.placement != super::mdpreview::Placement::Tab {
                    pv.set_placement(super::mdpreview::Placement::Tab);
                }
                self.activate_view(self.active, true);
            }
            C::PreviewHtml => {
                let html = print::preview_html(self.doc(), false);
                if let Err(e) = print::open_html(&format!("preview-{}", self.doc().display_name()), &html) {
                    self.message("預覽", e);
                }
            }

            // ── 巨集 ──
            C::StartRecMacro => {
                self.macros.steps.clear();
                self.macros.recording = true;
                self.set_status(self.t("msg.rec_start", "開始錄製巨集"));
            }
            C::StopRecMacro => {
                self.macros.recording = false;
                self.set_status(self.tf("msg.rec_stop", "已錄製 {n} 個步驟", Some(self.macros.steps.len()), None));
            }
            C::PlayRecMacro => {
                let steps = self.macros.steps.clone();
                self.play_macro(steps);
            }
            C::SaveRecMacro => self.save_recorded_macro(),
            C::RunTempMacro => self.dialog = Some(Dialog::TempMacro { script: super::macros::to_json(&self.macros.steps), error: None }),
            C::LoadMacro => {
                let title = self.t("msg.load_macro", "載入巨集").to_string();
                let filter = self.t("msg.macro_filter", "MadEdit 巨集").to_string();
                let mut dlg = rfd::FileDialog::new().set_title(title).add_filter(filter, &["json"]);
                if let Some(d) = super::macros::macro_dir() {
                    dlg = dlg.set_directory(d);
                }
                if let Some(p) = dlg.pick_file() {
                    self.load_macro_file(&p);
                }
            }
            C::EditMacroFile => {
                let title = self.t("msg.edit_macro", "編輯巨集").to_string();
                let filter = self.t("msg.macro_filter", "MadEdit 巨集").to_string();
                let mut dlg = rfd::FileDialog::new().set_title(title).add_filter(filter, &["json"]);
                if let Some(d) = super::macros::macro_dir() {
                    dlg = dlg.set_directory(d);
                }
                if let Some(p) = dlg.pick_file() {
                    self.open_path(&p);
                }
            }

            // ── 工具 ──
            C::Options => self.dialog = Some(Dialog::Options(Box::default())),
            C::Highlighting => self.dialog = Some(Dialog::Highlighting),
            C::FileAssociation => self.message(
                "檔案類型關聯",
                "檔案類型關聯需由作業系統設定：\n• macOS：在 Finder 對檔案按右鍵 →「取得資訊」→「打開檔案的應用程式」選擇 MadEdit →「全部更改」。\n• Windows：在檔案按右鍵 →「開啟檔案」→「選擇其他應用程式」。",
            ),
            C::SourceFormatter => self.dialog = Some(Dialog::SourceFormatter),
            C::PurgeHistories => self.dialog = Some(Dialog::PurgeHistories),
            C::ToggleBOM => {
                if !encoding::supports_bom(self.doc().encoding) {
                    self.set_status(self.t("msg.no_bom", "目前編碼不支援 BOM"));
                } else {
                    let doc = self.doc_mut();
                    let b = !doc.has_bom;
                    doc.set_bom(b);
                }
            }
            C::ConvertToDOS | C::InsertDOS => self.doc_mut().convert_newline(NewLineFormat::Dos),
            C::ConvertToMac | C::InsertMac => self.doc_mut().convert_newline(NewLineFormat::Mac),
            C::ConvertToUNIX | C::InsertUNIX => self.doc_mut().convert_newline(NewLineFormat::Unix),
            C::ConvertEncoding => {
                let enc = self.doc().encoding.name().to_string();
                let bom = self.doc().has_bom;
                self.dialog = Some(Dialog::ConvertEncoding { encoding: enc, bom });
            }
            C::ChineseSafeMode => self.chinese_safe = !self.chinese_safe,
            C::Simp2TradChinese | C::Trad2SimpChinese | C::Kanji2TradChinese | C::Kanji2SimpChinese | C::Chinese2Kanji if !hex && editable => {
                let conv = match c {
                    C::Simp2TradChinese => ChineseConversion::SimpToTrad,
                    C::Trad2SimpChinese => ChineseConversion::TradToSimp,
                    C::Kanji2TradChinese => ChineseConversion::KanjiToTrad,
                    C::Kanji2SimpChinese => ChineseConversion::KanjiToSimp,
                    _ => ChineseConversion::ChineseToKanji,
                };
                let safe = self.chinese_safe;
                self.transform(Scope::Whole, &|s| Ok(chinese::convert(s, conv, safe).0));
            }
            C::Simp2TradClipboard | C::Trad2SimpClipboard | C::Kanji2TradClipboard | C::Kanji2SimpClipboard | C::Chinese2KanjiClipboard => {
                let conv = match c {
                    C::Simp2TradClipboard => ChineseConversion::SimpToTrad,
                    C::Trad2SimpClipboard => ChineseConversion::TradToSimp,
                    C::Kanji2TradClipboard => ChineseConversion::KanjiToTrad,
                    C::Kanji2SimpClipboard => ChineseConversion::KanjiToSimp,
                    _ => ChineseConversion::ChineseToKanji,
                };
                if let Some(t) = self.system_clipboard() {
                    let (out, n) = chinese::convert(&t, conv, self.chinese_safe);
                    self.copy_to_system(&out);
                    self.set_status(self.tf("msg.clip_converted", "剪貼簿已轉換 {n} 個字", Some(n), None));
                }
            }
            C::Markdown2Html if !hex && editable => self.transform(Scope::Whole, &|s| Ok(textops::markdown_to_html(s))),
            C::Html2PlainText if !hex && editable => self.transform(Scope::Whole, &|s| Ok(textops::html_to_text(s))),
            C::XmlFormat if !hex && editable => {
                let n = self.settings.indent_size;
                self.transform(Scope::Whole, &|s| textops::format_xml(s, n))
            }
            C::JsonFormat if !hex && editable => {
                let n = self.settings.indent_size;
                self.transform(Scope::Whole, &|s| textops::format_json(s, n))
            }
            C::AstyleFormat if !hex && editable => {
                let opts = self.settings.astyle_options.clone();
                self.transform(Scope::Whole, &|s| run_astyle(s, &opts));
            }
            C::DprintFormat if !hex && editable => {
                let doc = self.doc();
                match dprint::stdin_file_name(doc) {
                    Ok(file_name) => self.transform(Scope::Whole, &|s| dprint::format_text(s, &file_name)),
                    Err(e) => self.message("無法格式化", e),
                }
            }
            C::WordCount => {
                let doc = self.doc();
                let sel = doc.editor.selected_text();
                let text = if sel.is_empty() { doc.editor.lines.join("\n") } else { sel };
                self.dialog = Some(Dialog::WordCount(textops::word_count(&text), !self.doc().editor.selection.is_empty()));
            }

            // ── 視窗 ──
            C::ToggleWindow => {
                let p = self.prev_active.min(self.tabs.len() - 1);
                self.activate_view(p, self.prev_preview_tab);
            }
            C::PreviousWindow | C::NextWindow => {
                let pages = self.tab_pages();
                let current = pages.iter().position(|p| *p == (self.active, self.is_preview_tab())).unwrap_or(0);
                let step = if c == C::NextWindow { 1 } else { pages.len() - 1 };
                self.activate_page((current + step) % pages.len());
            }
            C::SwitchTab1 => self.activate_page(0),
            C::SwitchTab2 => self.activate_page(1),
            C::SwitchTab3 => self.activate_page(2),
            C::SwitchTab4 => self.activate_page(3),
            C::SwitchTab5 => self.activate_page(4),
            C::SwitchTab6 => self.activate_page(5),
            C::SwitchTab7 => self.activate_page(6),
            C::SwitchTab8 => self.activate_page(7),
            C::SwitchLastTab => self.activate_page(self.tab_pages().len() - 1),
            C::WindowList => self.dialog = Some(Dialog::WindowList),
            C::About => self.dialog = Some(Dialog::About { copied_at: None }),
            C::KeyboardShortcuts => self.dialog = Some(Dialog::Shortcuts { filter: String::new() }),

            _ => {
                if !editable {
                    self.set_status(self.t("msg.readonly", "文件為唯讀"));
                } else if hex {
                    self.set_status(self.t("msg.not_hex", "此命令不適用於十六進位模式"));
                }
            }
        }
    }

    // ── 剪貼簿 ──────────────────────────────────────────────────

    fn copy(&mut self) {
        let doc = self.doc_mut();
        if doc.mode == EditMode::Hex {
            let enc = doc.encoding;
            let Some(hb) = doc.hex.as_ref() else { return };
            let bytes = hb.selected_bytes();
            if bytes.is_empty() {
                return;
            }
            let text = if hb.in_text_pane { encoding::decode_with(&bytes, enc).text } else { textops::to_hex_string(&bytes, true) };
            self.copy_to_system(&text);
            return;
        }
        let text = doc.editor.copy_to_clipboard();
        if !text.is_empty() {
            self.copy_to_system(&text);
        }
    }

    fn paste(&mut self) {
        self.sync_clipboard_from_system();
        let fill = self.settings.auto_fill_column_paste;
        let doc = self.doc_mut();
        if doc.mode == EditMode::Hex {
            let clip = doc.editor.clipboard.clone();
            let enc = doc.encoding;
            let Some(hb) = doc.hex.as_mut() else { return };
            let bytes = if hb.in_text_pane {
                encoding::encode_text(&clip, enc, false).unwrap_or_default()
            } else {
                textops::from_hex_string(&clip).unwrap_or_else(|| encoding::encode_text(&clip, enc, false).unwrap_or_default())
            };
            hb.paste(&bytes);
            return;
        }
        let hint = doc.to_end_hint();
        doc.edit(EditKind::Other, hint, |ed| ed.paste_from_clipboard(fill));
        self.tab_mut().view.ensure_visible = true;
    }

    fn delete_selection(&mut self) {
        let doc = self.doc_mut();
        if doc.mode == EditMode::Hex {
            if let Some(hb) = doc.hex.as_mut() {
                hb.delete(false);
            }
            return;
        }
        let hint = doc.selection_hint();
        doc.edit(EditKind::Other, hint, |ed| {
            if !ed.delete_selection() {
                ed.delete_forward();
            }
        });
    }

    pub fn insert_text(&mut self, s: &str) {
        let doc = self.doc_mut();
        let hint = doc.selection_hint();
        doc.edit(EditKind::Other, hint, |ed| ed.insert_text(s));
        self.tab_mut().view.ensure_visible = true;
    }

    // ── 文字轉換 ────────────────────────────────────────────────

    /// 轉換選取範圍；無選取時轉換游標所在字詞或整份文件
    fn transform(&mut self, scope: Scope, f: &dyn Fn(&str) -> Result<String, String>) {
        let doc = self.doc_mut();
        let sel = doc.editor.selection;
        let result: Result<(), String> = if sel.is_block() && !sel.is_empty() {
            // 先確認每段都能轉換
            let b = sel.visual_bounds(&doc.editor);
            let outs: Result<Vec<String>, String> =
                (b.min_line..=b.max_line).map(|l| f(&extract_visual_slice(&doc.editor.lines[l], b.min_vcol, b.max_vcol).0)).collect();
            outs.map(|outs| {
                let hint = doc.selection_hint();
                doc.edit(EditKind::Other, hint, |ed| insert_block(ed, b.min_line, b.min_vcol, outs));
            })
        } else if !sel.is_empty() {
            match f(&doc.editor.selected_text()) {
                Ok(s) => {
                    let hint = doc.selection_hint();
                    let backward = sel.head < sel.anchor;
                    doc.edit(EditKind::Other, hint, |ed| {
                        ed.replace_selection(&s);
                        if backward {
                            let (a, h) = (ed.selection.anchor, ed.selection.head);
                            ed.selection = Selection::new(h, a);
                        }
                    });
                    Ok(())
                }
                Err(e) => Err(e),
            }
        } else if scope == Scope::Word {
            let (s, e) = doc.editor.word_range_at(sel.head);
            if s == e {
                return;
            }
            doc.editor.select_range(s, e);
            match f(&doc.editor.selected_text()) {
                Ok(t) => {
                    let hint = doc.selection_hint();
                    doc.edit(EditKind::Other, hint, |ed| {
                        ed.replace_selection(&t);
                        let h = ed.selection.end();
                        ed.set_caret(h.line, h.col);
                    });
                    Ok(())
                }
                Err(e) => Err(e),
            }
        } else {
            let text = doc.editor.lines.join("\n");
            match f(&text) {
                Ok(t) if t != text => {
                    doc.replace_all_text(&t);
                    Ok(())
                }
                Ok(_) => Ok(()),
                Err(e) => Err(e),
            }
        };
        if let Err(e) = result {
            self.message("無法轉換", e);
        }
        self.tab_mut().view.ensure_visible = true;
    }

    /// 轉換選取的行（無選取時為整份文件）；行數改變時維護書籤
    fn transform_lines(&mut self, f: impl FnOnce(&[String]) -> Vec<String>) {
        let doc = self.doc_mut();
        let (a, b) = if doc.editor.selection.is_empty() { (0, doc.editor.lines.len() - 1) } else { selected_line_range(&doc.editor) };
        let had_sel = !doc.editor.selection.is_empty();
        doc.edit(EditKind::Other, a..b + 1, |ed| {
            let mut new = f(&ed.lines[a..=b]);
            if new.is_empty() && ed.lines.len() == b - a + 1 {
                new.push(String::new());
            }
            let delta = new.len() as i64 - (b - a + 1) as i64;
            let new_len = new.len();
            ed.lines.splice(a..=b, new);
            if delta != 0 {
                // 範圍內的書籤無法對應，予以移除；之後的書籤依行數變化位移
                ed.bookmarks = ed
                    .bookmarks
                    .iter()
                    .filter(|&&l| l < a || l > b)
                    .map(|&l| if l > b { (l as i64 + delta) as usize } else { l })
                    .collect();
            }
            if had_sel && new_len > 0 {
                let last = a + new_len - 1;
                ed.select_range(TextPosition::new(a, 0), TextPosition::new(last, ed.line_char_count(last)));
            } else {
                ed.clamp_caret();
            }
        });
    }

    /// 註解或取消註解（優先使用行註解）
    fn comment(&mut self, add: bool) {
        let doc = self.doc_mut();
        let (line_tok, block_tok) = syntax::comment_tokens(&doc.syntax);
        if let Some(tok) = line_tok {
            let prefix = format!("{tok} ");
            self.transform_lines_keep(|ls| textops::toggle_line_comment(ls, &prefix, Some(add)));
        } else if let Some((open, close)) = block_tok {
            if doc.editor.selection.is_empty() {
                let (a, b) = selected_line_range(&doc.editor);
                let end = doc.editor.line_char_count(b);
                doc.editor.select_range(TextPosition::new(a, 0), TextPosition::new(b, end));
            }
            self.transform(Scope::Whole, &|s| Ok(textops::toggle_block_comment(s, open, close, Some(add))));
        } else {
            let name = self.doc().syntax.clone();
            self.set_status(self.tf("msg.no_comment", "{q} 沒有註解符號", None, Some(&name)));
        }
    }

    /// 轉換選取的行（無選取時為游標所在行）
    fn transform_lines_keep(&mut self, f: impl FnOnce(&[String]) -> Vec<String>) {
        let doc = self.doc_mut();
        let (a, b) = selected_line_range(&doc.editor);
        let had_sel = !doc.editor.selection.is_empty();
        doc.edit(EditKind::Other, a..b + 1, |ed| {
            let new = f(&ed.lines[a..=b]);
            ed.lines.splice(a..=b, new);
            if had_sel {
                ed.select_range(TextPosition::new(a, 0), TextPosition::new(b, ed.line_char_count(b)));
            } else {
                ed.clamp_caret();
            }
        });
    }

    /// 自動換行處改為實際換行字元
    fn wrap_to_newlines(&mut self) {
        let (a, b, breaks) = {
            let tab = &self.tabs[self.active];
            let doc = &tab.doc;
            let (a, b) = if doc.editor.selection.is_empty() { (0, doc.editor.lines.len() - 1) } else { selected_line_range(&doc.editor) };
            let breaks: Vec<Vec<usize>> = (a..=b).map(|l| tab.view.wrap_breaks(l)).collect();
            (a, b, breaks)
        };
        if breaks.iter().all(|v| v.is_empty()) {
            self.set_status(self.t("msg.need_wrap", "請先開啟自動換行"));
            return;
        }
        let doc = self.doc_mut();
        doc.edit(EditKind::Other, a..b + 1, |ed| {
            let mut out = Vec::new();
            for (k, l) in (a..=b).enumerate() {
                let chars: Vec<char> = ed.lines[l].chars().collect();
                let mut prev = 0;
                for &br in &breaks[k] {
                    out.push(chars[prev..br].iter().collect::<String>().trim_end().to_string());
                    prev = br;
                }
                out.push(chars[prev..].iter().collect());
            }
            let delta = out.len() - (b - a + 1);
            ed.lines.splice(a..=b, out);
            ed.bookmarks = ed.bookmarks.iter().filter(|&&l| l < a || l > b).map(|&l| if l > b { l + delta } else { l }).collect();
            ed.clamp_caret();
        });
    }
}

/// 呼叫外部 astyle 格式化程式碼
fn run_astyle(text: &str, options: &str) -> Result<String, String> {
    use std::io::Write;
    let mut child = std::process::Command::new("astyle")
        .args(options.split_whitespace())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|_| "找不到 astyle 執行檔。請先安裝 Artistic Style（例如 macOS：brew install astyle），並確認它在 PATH 中。".to_string())?;
    child.stdin.take().unwrap().write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim_end_matches('\n').to_string())
}
