//! Keyboard, IME, pointer selection, scrolling and editor context menus.

use super::{
    CharWidths, Drag, EdAction, Frame, HorizontalBar, Layout, SpellContext, ViewEnv, ViewOutput,
    ViewState,
};
use crate::app::{commands::C, i18n, menus};
use crate::core::document::{Document, EditKind};
use crate::core::editor::{
    EditorState, Selection, TextPosition, char_col_to_visual_col, tab_columns_at,
    visual_col_to_char_col,
};
use crate::core::model::EditMode;
use crate::core::settings::Settings;
use crate::core::textops;
use eframe::egui::{self, Event, Key, Modifiers, PointerButton, Pos2, Rect, Vec2};

/// 自動補全的成對字元
fn closing_pair(c: char) -> Option<char> {
    match c {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        '"' => Some('"'),
        '\'' => Some('\''),
        _ => None,
    }
}

/// 執行一個編輯動作
pub fn apply_action(
    doc: &mut Document,
    view: &mut ViewState,
    act: &EdAction,
    settings: &Settings,
    widths: &CharWidths,
    lay: &Layout,
) {
    if doc.read_only
        && matches!(
            act,
            EdAction::Insert(_)
                | EdAction::Newline { .. }
                | EdAction::Tab
                | EdAction::BackTab
                | EdAction::Backspace
                | EdAction::Delete
                | EdAction::DeleteWord { .. }
                | EdAction::DeleteLineEdge { .. }
        )
    {
        return;
    }
    let sel_mode = doc.selecting_mode;
    let ext = |e: bool| e || sel_mode;
    let is_vertical = matches!(
        act,
        EdAction::Up { .. }
            | EdAction::Down { .. }
            | EdAction::PageUp { .. }
            | EdAction::PageDown { .. }
    );
    if !is_vertical {
        view.preferred_x = None;
    }
    match act {
        EdAction::Insert(text) => insert_typed(doc, text, settings),
        EdAction::Newline { indent } => {
            let hint = doc.selection_hint();
            let auto = *indent && settings.auto_indent;
            doc.edit(EditKind::Other, hint, |ed| {
                let block = ed.selection.is_block();
                if block {
                    ed.selection = Selection::caret(ed.selection.head);
                }
                let head = ed.selection.start();
                let lead: String = if auto {
                    ed.lines[head.line]
                        .chars()
                        .take(head.col)
                        .take_while(|c| *c == ' ' || *c == '\t')
                        .collect()
                } else {
                    String::new()
                };
                ed.insert_text(&format!("\n{lead}"));
            });
        }
        EdAction::Tab => {
            let sel = doc.editor.selection;
            if !sel.is_block() && sel.start().line != sel.end().line {
                indent_selection(doc, settings, true);
            } else {
                let text = if settings.insert_spaces_for_tab {
                    let head = doc.editor.selection.head;
                    let line = &doc.editor.lines[head.line];
                    let vcol = char_col_to_visual_col(line, head.col);
                    " ".repeat(tab_columns_at(vcol))
                } else {
                    "\t".to_string()
                };
                let hint = doc.selection_hint();
                doc.edit(EditKind::Typing, hint, |ed| ed.insert_text(&text));
            }
        }
        EdAction::BackTab => indent_selection(doc, settings, false),
        EdAction::Backspace => {
            let hint = doc.selection_hint();
            doc.edit(EditKind::Typing, hint, |ed| ed.delete_backspace());
        }
        EdAction::Delete => {
            let hint = doc.selection_hint();
            doc.edit(EditKind::Other, hint, |ed| ed.delete_forward());
        }
        EdAction::DeleteWord { forward } => {
            let hint = doc.selection_hint();
            doc.edit(EditKind::Other, hint, |ed| ed.delete_word(*forward));
        }
        EdAction::DeleteLineEdge { forward } => {
            let hint = doc.selection_hint();
            doc.edit(EditKind::Other, hint, |ed| {
                if ed.selection.is_empty() {
                    if *forward {
                        ed.move_end(true);
                    } else {
                        ed.move_home(true);
                    }
                }
                ed.delete_selection();
            });
        }
        EdAction::Left { extend, word } => {
            if *word {
                doc.editor.move_word(false, ext(*extend))
            } else if !ext(*extend)
                && !doc.editor.selection.is_empty()
                && !doc.editor.selection.is_block()
            {
                let s = doc.editor.selection.start();
                doc.editor.move_caret_to(s, false);
            } else {
                doc.editor.move_left(ext(*extend))
            }
        }
        EdAction::Right { extend, word } => {
            if *word {
                doc.editor.move_word(true, ext(*extend))
            } else if !ext(*extend)
                && !doc.editor.selection.is_empty()
                && !doc.editor.selection.is_block()
            {
                let e = doc.editor.selection.end();
                doc.editor.move_caret_to(e, false);
            } else {
                doc.editor.move_right(ext(*extend))
            }
        }
        EdAction::Up { extend } => move_rows(doc, view, -1, ext(*extend), widths, lay),
        EdAction::Down { extend } => move_rows(doc, view, 1, ext(*extend), widths, lay),
        EdAction::PageUp { extend } => {
            let n = view.visible_rows.max(2) as i64 - 1;
            view.top_row = view.top_row.saturating_sub(n as usize);
            move_rows(doc, view, -n, ext(*extend), widths, lay)
        }
        EdAction::PageDown { extend } => {
            let n = view.visible_rows.max(2) as i64 - 1;
            let max_top = view.row_count(&doc.editor).saturating_sub(1);
            view.top_row = (view.top_row + n as usize).min(max_top);
            move_rows(doc, view, n, ext(*extend), widths, lay)
        }
        EdAction::Home { extend } => doc.editor.move_home_smart(ext(*extend)),
        EdAction::LineStart { extend } => doc.editor.move_home(ext(*extend)),
        EdAction::End { extend } => doc.editor.move_end(ext(*extend)),
        EdAction::DocStart { extend } => doc.editor.move_doc_edge(false, ext(*extend)),
        EdAction::DocEnd { extend } => doc.editor.move_doc_edge(true, ext(*extend)),
        EdAction::Block { dx, dy } => {
            let ed = &mut doc.editor;
            if !ed.selection.is_block() {
                let head = ed.selection.head;
                let v = char_col_to_visual_col(&ed.lines[head.line], head.col);
                ed.selection = Selection::block(head, head, v, v);
            }
            let head = ed.selection.head;
            let head_v = ed.selection.head_vcol.unwrap_or(0);
            let new_line =
                (head.line as i64 + *dy as i64).clamp(0, ed.lines.len() as i64 - 1) as usize;
            let new_v = (head_v as i64 + *dx as i64).max(0) as usize;
            let col = visual_col_to_char_col(&ed.lines[new_line], new_v);
            ed.update_block_selection(TextPosition::new(new_line, col), new_v);
        }
        EdAction::ScrollLines(n) => {
            let max_top = view.row_count(&doc.editor).saturating_sub(1);
            view.top_row = (view.top_row as i64 + *n as i64).clamp(0, max_top as i64) as usize;
            return;
        }
        EdAction::ToggleInsert => {
            doc.editor.toggle_overwrite_mode();
        }
        EdAction::ClearSelection => {
            let h = doc.editor.selection.head;
            doc.editor.selection = Selection::caret(h);
            doc.selecting_mode = false;
        }
    }
    view.ensure_visible = true;
}

/// 輸入文字（處理自動補全成對字元）
fn insert_typed(doc: &mut Document, text: &str, settings: &Settings) {
    let mut chars = text.chars();
    let single = match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    };
    let hint = doc.selection_hint();
    let ed = &doc.editor;
    let sel = ed.selection;
    if settings.auto_complete_pair
        && !sel.is_block()
        && !ed.overwrite_mode
        && let Some(c) = single
    {
        // 輸入的右括號正好是游標右側的字：直接跳過
        let next = ed.lines[sel.head.line].chars().nth(sel.head.col);
        if sel.is_empty() && next == Some(c) && matches!(c, ')' | ']' | '}' | '"' | '\'') {
            doc.editor.move_right(false);
            return;
        }
        if let Some(close) = closing_pair(c) {
            if !sel.is_empty() && settings.wrap_selection_with_pair {
                let inner = ed.selected_text();
                doc.edit(EditKind::Other, hint, |ed| {
                    ed.replace_selection(&format!("{c}{inner}{close}"));
                    let s = ed.selection.start();
                    let e = ed.selection.end();
                    ed.select_range(
                        TextPosition::new(s.line, s.col + 1),
                        TextPosition::new(e.line, e.col.saturating_sub(1)),
                    );
                });
                return;
            }
            doc.edit(EditKind::Typing, hint, |ed| {
                ed.insert_text(&format!("{c}{close}"));
                ed.move_left(false);
            });
            return;
        }
    }
    doc.edit(EditKind::Typing, hint, |ed| ed.insert_text(text));
}

/// 選取範圍所涵蓋的行（選取結束於行首時不含該行）
pub fn selected_line_range(ed: &EditorState) -> (usize, usize) {
    let s = ed.selection.start();
    let e = ed.selection.end();
    let last = if e.line > s.line && e.col == 0 && !ed.selection.is_block() {
        e.line - 1
    } else {
        e.line
    };
    (s.line, last)
}

/// 增加或減少選取行的縮排
pub fn indent_selection(doc: &mut Document, settings: &Settings, increase: bool) {
    let (a, b) = selected_line_range(&doc.editor);
    let unit = if settings.insert_spaces_for_tab {
        " ".repeat(settings.indent_size.max(1))
    } else {
        "\t".to_string()
    };
    let size = settings.indent_size.max(1);
    let had_sel = !doc.editor.selection.is_empty();
    doc.edit(EditKind::Other, a..b + 1, |ed| {
        for l in a..=b {
            ed.lines[l] = if increase {
                textops::indent_line(&ed.lines[l], &unit)
            } else {
                textops::unindent_line(&ed.lines[l], size)
            };
        }
        if had_sel || a != b {
            ed.select_range(
                TextPosition::new(a, 0),
                TextPosition::new(b, ed.line_char_count(b)),
            );
        } else {
            let h = ed.selection.head;
            ed.set_caret(h.line, h.col);
        }
    });
}

/// 以視覺列為單位上下移動游標（自動換行時移動到相鄰的視覺列）
fn move_rows(
    doc: &mut Document,
    view: &mut ViewState,
    delta: i64,
    extend: bool,
    widths: &CharWidths,
    lay: &Layout,
) {
    let ed = &mut doc.editor;
    if ed.selection.is_block() && extend {
        return;
    }
    let head = ed.selection.head;
    let row = view.row_of(head);
    let x = view
        .preferred_x
        .unwrap_or_else(|| view.x_of(ed, head, lay, widths));
    let total = view.row_count(ed) as i64;
    let target = (row as i64 + delta).clamp(0, total - 1) as usize;
    let pos = if target as i64 == row as i64 + delta || delta.abs() > 1 {
        view.pos_at_x(ed, target, x, lay, widths)
    } else if delta < 0 {
        TextPosition::new(0, 0)
    } else {
        let last = ed.lines.len() - 1;
        TextPosition::new(last, ed.line_char_count(last))
    };
    ed.move_caret_to(pos, extend);
    view.preferred_x = Some(x);
}

// ── 鍵盤事件轉換 ────────────────────────────────────────────────

fn key_to_action(key: Key, m: Modifiers) -> Option<EdAction> {
    let mac = cfg!(target_os = "macos");
    // macOS：Option 移動字詞、⌘ 移至行首尾或文件頭尾；其他平台：Ctrl 移動字詞
    let word = if mac { m.alt } else { m.ctrl };
    let line_edge = mac && m.command;
    let extend = m.shift;
    let block = m.alt && m.shift && (!mac || m.command);
    Some(match key {
        Key::ArrowLeft if block => EdAction::Block { dx: -1, dy: 0 },
        Key::ArrowRight if block => EdAction::Block { dx: 1, dy: 0 },
        Key::ArrowUp if block => EdAction::Block { dx: 0, dy: -1 },
        Key::ArrowDown if block => EdAction::Block { dx: 0, dy: 1 },
        Key::ArrowLeft if line_edge => EdAction::LineStart { extend },
        Key::ArrowRight if line_edge => EdAction::End { extend },
        Key::ArrowUp if line_edge => EdAction::DocStart { extend },
        Key::ArrowDown if line_edge => EdAction::DocEnd { extend },
        Key::ArrowLeft => EdAction::Left { extend, word },
        Key::ArrowRight => EdAction::Right { extend, word },
        Key::ArrowUp if m.command && !m.alt => EdAction::ScrollLines(-1),
        Key::ArrowDown if m.command && !m.alt => EdAction::ScrollLines(1),
        Key::ArrowUp if !m.alt => EdAction::Up { extend },
        Key::ArrowDown if !m.alt => EdAction::Down { extend },
        Key::PageUp => EdAction::PageUp { extend },
        Key::PageDown => EdAction::PageDown { extend },
        Key::Home if mac || m.command => EdAction::DocStart { extend },
        Key::End if mac || m.command => EdAction::DocEnd { extend },
        Key::Home => EdAction::Home { extend },
        Key::End => EdAction::End { extend },
        Key::Backspace if line_edge => EdAction::DeleteLineEdge { forward: false },
        Key::Delete if line_edge => EdAction::DeleteLineEdge { forward: true },
        Key::Backspace if word => EdAction::DeleteWord { forward: false },
        Key::Delete if word => EdAction::DeleteWord { forward: true },
        Key::Backspace if !m.command => EdAction::Backspace,
        Key::Delete if !m.command && !m.shift => EdAction::Delete,
        Key::Enter if !m.command && !m.alt => EdAction::Newline { indent: !m.shift },
        Key::Tab if m.shift && !m.command => EdAction::BackTab,
        Key::Tab if !m.command && !m.alt => EdAction::Tab,
        Key::Insert if m.is_none() => EdAction::ToggleInsert,
        Key::Escape => EdAction::ClearSelection,
        _ => return None,
    })
}

pub(super) fn interact(
    ui: &egui::Ui,
    doc: &mut Document,
    view: &mut ViewState,
    env: &ViewEnv<'_>,
    frame: &Frame,
    out: &mut ViewOutput,
) -> Rect {
    let ctx = ui.ctx();
    let settings = env.settings;
    let widths: &CharWidths = env.widths;
    let lay = &frame.lay;
    let full = frame.full;
    let text_rect = frame.text_rect;
    let gutter_rect = frame.gutter_rect;
    let mark_w = frame.mark_w;
    let id = frame.id;
    let response = &frame.response;
    let gutter_resp = &frame.gutter_resp;
    let vbar = frame.vbar;
    let vbar_resp = &frame.vbar_resp;
    if response.clicked()
        || response.drag_started()
        || gutter_resp.clicked()
        || gutter_resp.drag_started()
    {
        response.request_focus();
    }
    let focused = response.has_focus();
    out.focused = focused;
    if focused {
        ctx.memory_mut(|m| {
            m.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            )
        });
    }
    // ── 鍵盤與輸入法 ──
    if focused {
        let events = ctx.input(|i| i.events.clone());
        let mut actions: Vec<EdAction> = Vec::new();
        for ev in &events {
            match ev {
                Event::Text(t) if !t.chars().any(|c| c.is_control()) => {
                    view.preedit = None;
                    actions.push(EdAction::Insert(t.clone()));
                }
                Event::Ime(egui::ImeEvent::Preedit { text, .. }) => {
                    view.preedit = (!text.is_empty()).then(|| text.clone());
                }
                Event::Ime(egui::ImeEvent::Commit(t)) => {
                    view.preedit = None;
                    if !t.is_empty() {
                        actions.push(EdAction::Insert(t.clone()));
                    }
                }
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    if view.preedit.is_some() {
                        continue;
                    }
                    if let Some(a) = key_to_action(*key, *modifiers) {
                        actions.push(a);
                    }
                }
                _ => {}
            }
        }
        for a in actions {
            apply_action(doc, view, &a, settings, widths, lay);
            // 編輯後立即重建換行快取，後續動作與本幀繪製才不會用舊的斷行位置索引新內容
            view.update_wrap(doc, lay, widths);
            view.clear_frame_layout();
            out.actions.push(a);
        }
        if doc.mode == EditMode::Column && !doc.editor.selection.is_block() {
            // 列塊模式下游標一律為矩形（零寬）選取，方向鍵與輸入都以欄為單位
            let h = doc.editor.selection.head;
            if doc.editor.selection.is_empty() {
                let v = char_col_to_visual_col(&doc.editor.lines[h.line], h.col);
                doc.editor.selection = Selection::block(h, h, v, v);
            }
        }
    }

    // ── 滑鼠 ──
    let pointer = ctx.input(|i| i.pointer.clone());
    let modifiers = ctx.input(|i| i.modifiers);
    let hit = |view: &ViewState, ed: &EditorState, p: Pos2| -> (TextPosition, usize) {
        let row = view.top_row + ((p.y - text_rect.top()) / lay.row_h).floor().max(0.0) as usize;
        let row = row.min(view.row_count(ed).saturating_sub(1));
        let x_rel = if lay.rtl {
            text_rect.right() + view.scroll_x - p.x
        } else {
            p.x - text_rect.left() + view.scroll_x
        };
        let pos = view.pos_at_x(ed, row, x_rel.max(0.0), lay, widths);
        let vcol = (x_rel.max(0.0) / widths.cell_w).round() as usize;
        (pos, vcol)
    };

    if let Some(p) = pointer.interact_pos() {
        let block_drag = modifiers.alt || doc.mode == EditMode::Column;
        if response.drag_started_by(PointerButton::Primary)
            || (response.clicked_by(PointerButton::Primary) && !response.double_clicked())
        {
            // 選取的錨點必須是「按下」的位置：egui 的 drag_started 要等指標移動超過臨界距離
            // 才會觸發，若用當下的指標位置當錨點，按下的那個字元會被漏掉（往回拖曳時最明顯）。
            let origin = pointer.press_origin().unwrap_or(p);
            let (pos, vcol) = hit(view, &doc.editor, origin);
            let ed = &mut doc.editor;
            if block_drag && view.wrap.key.is_none() {
                let anchor = if modifiers.shift {
                    ed.selection.anchor
                } else {
                    pos
                };
                let anchor_v = if modifiers.shift {
                    ed.selection.anchor_vcol.unwrap_or_else(|| {
                        char_col_to_visual_col(&ed.lines[anchor.line], anchor.col)
                    })
                } else {
                    vcol
                };
                ed.selection = Selection::block(anchor, pos, anchor_v, vcol);
                view.drag = Some(Drag::Block);
            } else {
                ed.move_caret_to(pos, modifiers.shift || doc.selecting_mode);
                view.drag = Some(Drag::Select);
            }
            view.preferred_x = None;
            view.highlight_word = None;
        }
        if response.double_clicked() {
            let (pos, _) = hit(view, &doc.editor, p);
            let (s, e) = doc.editor.word_range_at(pos);
            doc.editor.select_range(s, e);
            if settings.double_click_highlight && s != e {
                let w: String = doc.editor.selected_text();
                view.highlight_word = Some(w);
            }
            view.drag = None;
        }
        if response.triple_clicked() {
            let (pos, _) = hit(view, &doc.editor, p);
            let end = if pos.line + 1 < doc.editor.lines.len() {
                TextPosition::new(pos.line + 1, 0)
            } else {
                TextPosition::new(pos.line, doc.editor.line_char_count(pos.line))
            };
            doc.editor.select_range(TextPosition::new(pos.line, 0), end);
            view.drag = None;
        }
        if response.dragged_by(PointerButton::Primary) {
            // 拖曳到文字區外時自動捲動
            if p.y < text_rect.top() {
                view.top_row = view.top_row.saturating_sub(1);
            } else if p.y > text_rect.bottom() {
                view.top_row =
                    (view.top_row + 1).min(view.row_count(&doc.editor).saturating_sub(1));
            }
            if !lay.rtl && view.wrap.key.is_none() {
                if p.x < text_rect.left() {
                    view.scroll_x = (view.scroll_x - widths.cell_w).max(0.0);
                } else if p.x > text_rect.right() {
                    view.scroll_x += widths.cell_w;
                }
            }
            let (pos, vcol) = hit(view, &doc.editor, p);
            match view.drag {
                Some(Drag::Block) => doc.editor.update_block_selection(pos, vcol),
                Some(Drag::Select) => doc.editor.move_caret_to(pos, true),
                _ => {}
            }
            ctx.request_repaint();
        }
        if response.middle_clicked() && settings.middle_click_paste {
            let (pos, _) = hit(view, &doc.editor, p);
            doc.editor.move_caret_to(pos, false);
            out.paste = true;
        }
        if response.secondary_clicked() {
            let (pos, _) = hit(view, &doc.editor, p);
            view.spell_ctx = None;
            if let Some(sc) = env.spell.filter(|_| settings.spell_check) {
                let (s, e) = doc.editor.word_range_at(pos);
                if s != e && s.line == e.line {
                    let word: String = doc.editor.lines[s.line]
                        .chars()
                        .skip(s.col)
                        .take(e.col - s.col)
                        .collect();
                    if sc.misspelled_ranges(&word).first() == Some(&(0, word.chars().count())) {
                        view.spell_ctx = Some(SpellContext {
                            line: s.line,
                            start: s.col,
                            end: e.col,
                            suggestions: sc.suggest(&word),
                            word,
                        });
                    }
                }
            }
            // 在選取範圍外按右鍵時移動游標
            let sel = doc.editor.selection;
            if sel.is_empty() || pos < sel.start() || pos > sel.end() {
                doc.editor.move_caret_to(pos, false);
            }
        }
    }
    if response.drag_stopped() {
        if settings.auto_copy_mouse_selection && !doc.editor.selection.is_empty() {
            out.copy = Some(doc.editor.copy_to_clipboard());
        }
        view.drag = None;
    }

    // 行號欄：點選或拖曳選取整行，點書籤區切換書籤
    if let Some(p) = pointer.interact_pos() {
        let row_at = |view: &ViewState, ed: &EditorState, at: Pos2| {
            let r = view.top_row + ((at.y - text_rect.top()) / lay.row_h).floor().max(0.0) as usize;
            view.row_info(ed, r.min(view.row_count(ed).saturating_sub(1)))
                .0
        };
        if gutter_resp.clicked() && settings.show_bookmarks && p.x < gutter_rect.left() + mark_w {
            let line = row_at(view, &doc.editor, p);
            doc.editor.move_caret_to(TextPosition::new(line, 0), false);
            doc.editor.toggle_bookmark();
        } else if gutter_resp.drag_started() || gutter_resp.clicked() {
            // 起始行同樣取「按下」的位置，否則拖曳起點會落在拖曳開始時的指標位置
            let origin = pointer.press_origin().unwrap_or(p);
            let line = row_at(view, &doc.editor, origin);
            view.drag = Some(Drag::Lines { anchor_line: line });
        }
        if let Some(Drag::Lines { anchor_line }) = view.drag {
            let line = row_at(view, &doc.editor, p);
            let (a, b) = (anchor_line.min(line), anchor_line.max(line));
            let end = if b + 1 < doc.editor.lines.len() {
                TextPosition::new(b + 1, 0)
            } else {
                TextPosition::new(b, doc.editor.line_char_count(b))
            };
            let start = TextPosition::new(a, 0);
            if line < anchor_line {
                doc.editor.select_range(end, start);
            } else {
                doc.editor.select_range(start, end);
            }
            if !gutter_resp.dragged() {
                view.drag = None;
            }
        }
    }

    // 滾輪：垂直捲動（Shift 水平），Ctrl/⌘ 交由 App 縮放
    if ui.rect_contains_pointer(full) && !modifiers.command {
        let delta = ctx.input(|i| i.smooth_scroll_delta());
        if delta.y != 0.0 {
            let rows = -delta.y / lay.row_h;
            let acc = ctx.data_mut(|d| {
                let acc: &mut f32 = d.get_temp_mut_or_default(id.with("wheel"));
                *acc += rows;
                let whole = acc.trunc();
                *acc -= whole;
                whole
            });
            let max_top = view.row_count(&doc.editor).saturating_sub(1) as i64;
            view.top_row = (view.top_row as i64 + acc as i64).clamp(0, max_top) as usize;
        }
        if delta.x != 0.0 && view.wrap.key.is_none() {
            view.scroll_x = (view.scroll_x - delta.x).max(0.0);
        }
    }

    // ── 捲軸 ──
    let total_rows = view.row_count(&doc.editor);
    let vis = view.visible_rows;
    let max_top = total_rows.saturating_sub(1);
    let v_track = vbar.shrink2(Vec2::new(2.0, 2.0));
    let v_thumb_h = (v_track.height() * vis as f32 / (total_rows + vis).max(1) as f32)
        .clamp(20.0, v_track.height());
    let v_frac = if max_top == 0 {
        0.0
    } else {
        view.top_row as f32 / max_top as f32
    };
    let v_thumb = Rect::from_min_size(
        Pos2::new(
            v_track.left(),
            v_track.top() + (v_track.height() - v_thumb_h) * v_frac,
        ),
        Vec2::new(v_track.width(), v_thumb_h),
    );
    if let Some(p) = vbar_resp.interact_pointer_pos() {
        if vbar_resp.drag_started() {
            view.drag = Some(Drag::VScroll {
                grab: if v_thumb.contains(p) {
                    p.y - v_thumb.top()
                } else {
                    v_thumb_h / 2.0
                },
            });
        }
        if let Some(Drag::VScroll { grab }) = view.drag {
            let frac = ((p.y - grab - v_track.top()) / (v_track.height() - v_thumb_h).max(1.0))
                .clamp(0.0, 1.0);
            view.top_row = (frac * max_top as f32).round() as usize;
        } else if vbar_resp.clicked() {
            if p.y < v_thumb.top() {
                view.top_row = view.top_row.saturating_sub(vis);
            } else if p.y > v_thumb.bottom() {
                view.top_row = (view.top_row + vis).min(max_top);
            }
        }
    }
    if vbar_resp.drag_stopped() {
        view.drag = None;
    }

    // ── 游標可見性 ──
    let caret_key = (
        doc.editor.selection.head,
        doc.version,
        doc.editor.selection.head_vcol,
    );
    if view.last_caret != Some(caret_key) || view.ensure_visible {
        if view.drag.is_none() || matches!(view.drag, Some(Drag::Select | Drag::Block)) {
            view.scroll_to_caret(
                &doc.editor,
                lay,
                widths,
                text_rect.width(),
                settings.typewriter_mode && view.drag.is_none(),
            );
        }
        view.last_caret = Some(caret_key);
        view.ensure_visible = false;
        view.blink_start = ctx.input(|i| i.time);
    }
    view.top_row = view.top_row.min(max_top);
    v_thumb
}

pub(super) fn horizontal_drag(view: &mut ViewState, frame: &Frame, bar: Option<&HorizontalBar>) {
    if let Some(bar) = bar {
        let hbar_resp = &frame.hbar_resp;
        let h_thumb = bar.thumb;
        let h_track = bar.track;
        let thumb_w = h_thumb.width();
        let max_scroll = bar.max_scroll;
        if let Some(p) = hbar_resp.interact_pointer_pos() {
            if hbar_resp.drag_started() {
                view.drag = Some(Drag::HScroll {
                    grab: if h_thumb.contains(p) {
                        p.x - h_thumb.left()
                    } else {
                        thumb_w / 2.0
                    },
                });
            }
            if let Some(Drag::HScroll { grab }) = view.drag {
                let f = ((p.x - grab - h_track.left()) / (h_track.width() - thumb_w).max(1.0))
                    .clamp(0.0, 1.0);
                view.scroll_x = f * max_scroll;
            }
        }
        if hbar_resp.drag_stopped() {
            view.drag = None;
        }
    }
}

pub(super) fn context_menu(
    doc: &mut Document,
    view: &mut ViewState,
    settings: &Settings,
    response: &egui::Response,
    out: &mut ViewOutput,
) {
    // 右鍵選單：拼字建議與編輯命令
    let spell_ctx = &mut view.spell_ctx;
    let mut replace_word: Option<(usize, usize, usize, String)> = None;
    let lang = i18n::resolve(settings.ui_language);
    let spell_none = i18n::translate(lang, "spell.none", "（沒有建議）");
    let spell_add = i18n::translate(lang, "spell.add", "將「{q}」加入字典");
    response.context_menu(|ui| {
        ui.set_min_width(menus::SUBMENU_MIN_WIDTH);
        if let Some(sc) = spell_ctx.as_ref() {
            if sc.suggestions.is_empty() {
                ui.label(spell_none);
            }
            for s in &sc.suggestions {
                if ui.button(menus::check_label(false, s)).clicked() {
                    replace_word = Some((sc.line, sc.start, sc.end, s.clone()));
                    ui.close();
                }
            }
            if ui
                .button(menus::check_label(
                    false,
                    &i18n::fill(spell_add, None, Some(&sc.word)),
                ))
                .clicked()
            {
                out.add_word = Some(sc.word.clone());
                ui.close();
            }
            ui.separator();
        }
        for c in [
            C::Undo,
            C::Redo,
            C::Cut,
            C::Copy,
            C::Paste,
            C::Delete,
            C::SelectAll,
        ] {
            if ui
                .button(menus::check_label(false, &i18n::command_text(lang, c)))
                .clicked()
            {
                out.commands.push(c);
                ui.close();
            }
        }
        ui.separator();
        for c in [C::ToUpperCase, C::ToLowerCase, C::Comment, C::Uncomment] {
            if ui
                .button(menus::check_label(false, &i18n::command_text(lang, c)))
                .clicked()
            {
                out.commands.push(c);
                ui.close();
            }
        }
    });
    if let Some((line, s, e, word)) = replace_word {
        let hint = line..line + 1;
        doc.edit(EditKind::Other, hint, |ed| {
            ed.select_range(TextPosition::new(line, s), TextPosition::new(line, e));
            ed.insert_text(&word);
        });
        view.clear_frame_layout();
        view.spell_cache.clear();
    }
    if out.add_word.is_some() {
        view.spell_cache.clear();
    }
}
