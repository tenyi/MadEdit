//! 以 egui_kittest 模擬鍵盤與輸入法事件的整合測試（不開視窗、不需 GPU）

use eframe::egui::{self, Event, ImeEvent, Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::commands::C;
use super::dialogs::Dialog;
use super::{App, Confirm};

fn harness() -> Harness<'static, App> {
    let mut h = Harness::builder().allow_missing_glyphs().with_size([1000.0, 700.0]).build_eframe(|cc| App::new(cc, Vec::new(), None));
    // 測試機的系統語言不一定是繁體中文，按鈕文字固定用原文
    h.state_mut().settings.ui_language = crate::core::settings::UiLanguage::ZhTw;
    h.run_steps(3);
    h
}

fn type_text(h: &mut Harness<'static, App>, s: &str) {
    h.event(Event::Text(s.to_string()));
    h.run_steps(2);
}

fn text(h: &mut Harness<'static, App>) -> String {
    h.state_mut().doc().editor.lines.join("\n")
}

#[test]
fn test_instance_request_restores_window_without_running_ui() {
    let dir = std::env::temp_dir().join(format!("madedit-hidden-instance-{}", std::process::id()));
    let instance = crate::single_instance::Instance::start(&dir, vec![]).unwrap().unwrap();
    let file = dir.join("external.txt");
    std::fs::write(&file, "external document").unwrap();
    let mut h = harness();
    h.state_mut().instance = Some(instance);
    assert!(crate::single_instance::Instance::start(&dir, vec![file]).unwrap().is_none());
    let ctx = eframe::egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    // 模擬最小化時只有邏輯回呼的情況，不能依賴 ui 才取出要求。
    let output = ctx.run_logic(&Default::default(), |ctx| {
        eframe::App::logic(h.state_mut(), ctx, &mut frame);
    });
    assert_eq!(text(&mut h), "external document");
    let commands: Vec<_> = output.viewport_commands.values().flatten().collect();
    assert!(commands.iter().any(|cmd| matches!(cmd, eframe::egui::ViewportCommand::Minimized(false))));
    assert!(commands.iter().any(|cmd| matches!(cmd, eframe::egui::ViewportCommand::Focus)));
    drop(h.state_mut().instance.take());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn test_external_open_reuses_tabs_and_preserves_unsaved_text() {
    let dir = std::env::temp_dir().join(format!("madedit-external-open-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let first = dir.join("中文 空白.txt");
    let second = dir.join("second.txt");
    std::fs::write(&first, "first").unwrap();
    std::fs::write(&second, "second").unwrap();
    let mut h = harness();
    type_text(&mut h, "尚未儲存");
    let unsaved_id = h.state().doc().id;
    let ctx = eframe::egui::Context::default();
    h.state_mut().open_external_files(&ctx, vec![first.clone(), second.clone()]);
    assert_eq!(h.state().tabs.len(), 3);
    assert_eq!(h.state().doc().path.as_ref(), Some(&second.canonicalize().unwrap()));
    h.state_mut().open_external_files(&ctx, vec![first.clone(), first]);
    assert_eq!(h.state().tabs.len(), 3);
    assert_eq!(text(&mut h), "first");
    h.state_mut().open_external_files(&ctx, vec![]);
    assert_eq!(h.state().tabs.len(), 3);
    let unsaved = h.state().tabs.iter().find(|t| t.doc.id == unsaved_id).unwrap();
    assert_eq!(unsaved.doc.editor.lines.join("\n"), "尚未儲存");
    assert!(unsaved.doc.is_modified());
    std::fs::remove_dir_all(dir).unwrap();
}

/// 以指定文件量測開檔到並排預覽排版完成，以及輸入文字後的更新；不含原生視窗與 GPU 呈現。
/// 執行：MADEDIT_MD_BENCH_FILE=文件 cargo test --release benchmark_markdown_preview_ui -- --ignored --nocapture
#[test]
#[ignore]
fn benchmark_markdown_preview_ui() {
    use std::time::Instant;
    use super::mdpreview::Placement;
    let path = std::path::PathBuf::from(std::env::var_os("MADEDIT_MD_BENCH_FILE").expect("請設定 MADEDIT_MD_BENCH_FILE"));
    let mut first = Vec::new();
    let mut edit = Vec::new();
    for sample in 0..12 {
        let start = Instant::now();
        let mut h = Harness::builder().allow_missing_glyphs().with_size([1400.0, 900.0]).build_eframe(|cc| {
            let mut app = App::new(cc, vec![path.clone()], None);
            app.run(&cc.egui_ctx, C::PreviewMarkdown);
            app.tab_mut().md_preview.as_mut().unwrap().set_placement(Placement::Right);
            // Opening preview activates its tab; typing measurements require the source editor.
            app.activate_view(app.active, false);
            app
        });
        h.run_steps(5);
        let elapsed = start.elapsed();
        let pv = h.state_mut().tab_mut().md_preview.as_ref().unwrap();
        assert!(!pv.anchors.is_empty());
        assert!((pv.rect.width() - 700.0).abs() < 20.0, "指定文件也須維持左右各半，預覽寬度 {}", pv.rect.width());
        if sample >= 3 {
            first.push(elapsed);
        }
        let version = h.state_mut().doc().version;
        h.event(Event::Text("測試內文".into()));
        let start = Instant::now();
        h.run_steps(3);
        let elapsed = start.elapsed();
        assert!(h.state_mut().doc().version > version, "量測的輸入事件必須實際修改文件");
        if sample >= 3 {
            edit.push(elapsed);
        }
        // 選用的離屏截圖在計時完成後才執行，不將 GPU 初始化算入效能樣本。
        if sample == 11
            && let Some(out) = std::env::var_os("MADEDIT_MD_BENCH_SHOT")
        {
            if let Ok(line) = std::env::var("MADEDIT_MD_BENCH_TOP_LINE").unwrap_or_default().parse::<usize>() {
                h.state_mut().tab_mut().view.top_row = line;
            }
            h.run_steps(5);
            h.render().expect("預覽截圖失敗").save(out).unwrap();
        }
    }
    crate::test_support::report_benchmark("markdown_ui.open", first);
    crate::test_support::report_benchmark("markdown_ui.edit", edit);
}

/// 以含超長單行的文件量測開檔、輸入、移動游標、選取與捲動的畫面更新耗時；不含原生視窗與 GPU 呈現。
/// 執行：MADEDIT_LONGLINE_BENCH_FILE=文件 cargo test --release benchmark_long_line_ -- --ignored --nocapture
#[test]
#[ignore]
fn benchmark_long_line_editing() {
    use std::time::Instant;

    use crate::core::settings::WrapMode;

    let path = std::path::PathBuf::from(std::env::var_os("MADEDIT_LONGLINE_BENCH_FILE").expect("請設定 MADEDIT_LONGLINE_BENCH_FILE"));

    fn samples(wrap: WrapMode, path: &std::path::Path) {
        let mut open = Vec::new();
        let mut typing = Vec::new();
        let mut caret = Vec::new();
        let mut select = Vec::new();
        let mut scroll = Vec::new();
        let mut click = Vec::new();
        for sample in 0..6 {
            let start = Instant::now();
            let mut h = Harness::builder().allow_missing_glyphs().with_size([1400.0, 900.0]).build_eframe(|cc| {
                let mut app = App::new(cc, vec![path.to_path_buf()], None);
                app.settings.wrap_mode = wrap;
                app
            });
            h.run_steps(3);
            if sample >= 1 {
                open.push(start.elapsed());
            }
            for _ in 0..8 {
                let t = Instant::now();
                h.event(Event::Text("x".into()));
                h.run_steps(1);
                if sample >= 1 {
                    typing.push(t.elapsed());
                }
            }
            let t = Instant::now();
            h.key_press(Key::End);
            h.run_steps(1);
            if sample >= 1 {
                caret.push(t.elapsed());
            }
            let t = Instant::now();
            h.key_press_modifiers(Modifiers::SHIFT, Key::End);
            h.run_steps(1);
            if sample >= 1 {
                select.push(t.elapsed());
            }
            let t = Instant::now();
            for _ in 0..5 {
                h.key_press(Key::PageDown);
                h.run_steps(1);
            }
            if sample >= 1 {
                scroll.push(t.elapsed());
            }
            let t = Instant::now();
            h.hover_at(eframe::egui::pos2(600.0, 400.0));
            h.drag_at(eframe::egui::pos2(600.0, 400.0));
            h.drop_at(eframe::egui::pos2(600.0, 400.0));
            h.run_steps(1);
            if sample >= 1 {
                click.push(t.elapsed());
            }
            assert!(h.state_mut().doc().version > 0, "量測的文件必須實際開啟並編輯");
        }
        for (name, values) in [("open", open), ("keystroke", typing), ("end", caret),
            ("selection", select), ("page_down", scroll), ("click", click)] {
            crate::test_support::report_benchmark(&format!("long_line.{wrap:?}.{name}"), values);
        }
    }

    samples(WrapMode::NoWrap, &path);
    samples(WrapMode::ByWindow, &path);
}

/// 自動換行下超長行的視覺列、游標與編輯（涵蓋換行快取與逐行繪製重構）
#[test]
fn test_wrapped_long_line_rows_caret_and_edit() {
    use crate::core::settings::WrapMode;
    use crate::core::editor::TextPosition;
    let mut h = harness();
    h.state_mut().settings.wrap_mode = WrapMode::ByWindow;
    h.event(Event::Paste(format!("{}\n短行", "x".repeat(2000))));
    h.run_steps(3);
    let rows = {
        let tab = h.state_mut().tab_mut();
        tab.view.row_count(&tab.doc.editor)
    };
    assert!(rows > 3, "2000 字的行自動換行後必須佔多個視覺列，實際 {rows}");
    assert_eq!(h.state().doc().editor.lines.len(), 2, "換行不會產生新的邏輯行");

    // 行首往下：應走到同一邏輯行的下一個視覺列，而非第二行
    h.state_mut().doc_mut().editor.set_caret(0, 0);
    h.run_steps(2);
    h.key_press(Key::ArrowDown);
    h.run_steps(2);
    let head = h.state().doc().editor.selection.head;
    assert_eq!(head.line, 0, "第一個視覺列的下方仍是同一行");
    assert!(head.col > 0);

    // 行尾（第 2000 字）往下：應落到短行行尾（水平移動重設記憶欄位）
    h.state_mut().doc_mut().editor.set_caret(0, 2000);
    h.run_steps(2);
    h.key_press(Key::ArrowLeft);
    h.key_press(Key::ArrowRight);
    h.run_steps(2);
    h.key_press(Key::ArrowDown);
    h.run_steps(2);
    assert_eq!(h.state().doc().editor.selection.head, TextPosition::new(1, 2), "行尾的下方是短行行尾");

    // 在超長行中間輸入：換行快取增量更新，內容與游標正確
    h.state_mut().doc_mut().editor.set_caret(0, 1000);
    h.run_steps(1);
    type_text(&mut h, "abc");
    assert_eq!(h.state().doc().editor.lines[0].chars().count(), 2003);
    assert_eq!(h.state().doc().editor.selection.head, TextPosition::new(0, 1003));
}

#[test]
fn test_typing_enter_backspace_and_undo() {
    let mut h = harness();
    type_text(&mut h, "abc");
    h.key_press(Key::Enter);
    h.run_steps(2);
    type_text(&mut h, "xy");
    h.key_press(Key::Backspace);
    h.run_steps(2);
    assert_eq!(text(&mut h), "abc\nx");
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    // Typing and the following backspace form one undo group.
    assert_eq!(text(&mut h), "abc\n");
    assert_eq!(h.state().doc().editor.selection.head, crate::core::editor::TextPosition::new(1, 0));
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
    h.run_steps(2);
    assert_eq!(text(&mut h), "abc\nx");
    assert_eq!(h.state().doc().editor.selection.head, crate::core::editor::TextPosition::new(1, 1));
}

#[test]
fn test_ime_preedit_then_commit() {
    let mut h = harness();
    h.event(Event::Ime(ImeEvent::Preedit { text: "ㄋㄧˇ".into(), active_range_chars: None }));
    h.run_steps(2);
    assert_eq!(text(&mut h), "", "預編輯文字不應寫入文件");
    h.event(Event::Ime(ImeEvent::Commit("你".into())));
    h.run_steps(2);
    assert_eq!(text(&mut h), "你");
}

#[cfg(target_os = "macos")]
#[test]
fn test_macos_navigation_delete_and_undo() {
    let mut h = harness();
    h.event(Event::Paste("  hello world\n第二行".into()));
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::ArrowUp);
    h.run_steps(2);
    assert_eq!(h.state_mut().doc().editor.selection.head, crate::core::editor::TextPosition::new(0, 0));
    h.key_press_modifiers(Modifiers::COMMAND, Key::ArrowRight);
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::ALT, Key::Backspace);
    h.run_steps(2);
    assert_eq!(text(&mut h), "  hello \n第二行");
    h.key_press_modifiers(Modifiers::COMMAND, Key::Backspace);
    h.run_steps(2);
    assert_eq!(text(&mut h), "\n第二行");
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    assert_eq!(text(&mut h), "  hello \n第二行");
    h.key_press_modifiers(Modifiers::COMMAND, Key::ArrowLeft);
    h.run_steps(2);
    assert_eq!(h.state_mut().doc().editor.selection.head.col, 0, "⌘← 必須到實際行首");
}

#[cfg(target_os = "macos")]
#[test]
fn test_macos_close_and_quit_preserve_unsaved_document() {
    let mut h = harness();
    type_text(&mut h, "未儲存");
    h.key_press_modifiers(Modifiers::COMMAND, Key::W);
    h.run_steps(2);
    assert!(matches!(h.state_mut().confirm, Some(Confirm::Close { exit: false, .. })));
    h.state_mut().confirm = None;
    h.key_press_modifiers(Modifiers::COMMAND, Key::Q);
    h.run_steps(2);
    assert!(matches!(h.state_mut().confirm, Some(Confirm::Close { exit: true, .. })));
    assert_eq!(text(&mut h), "未儲存");
    assert!(!h.state_mut().allow_close);
}

#[test]
fn test_find_bar_selects_match() {
    let mut h = harness();
    type_text(&mut h, "one two one");
    h.key_press_modifiers(Modifiers::COMMAND, Key::F);
    h.run_steps(3);
    assert!(h.state_mut().find.visible);
    h.state_mut().find.query = "two".into();
    h.state_mut().pending.push(C::FindNext);
    h.run_steps(3);
    assert_eq!(h.state_mut().doc().editor.selected_text(), "two");
}

#[test]
fn test_find_bar_counts_and_highlights_matches_live() {
    use egui::accesskit::Role;
    let mut h = harness();
    let s = &mut h.state_mut().settings;
    (s.search_match_case, s.search_whole_word, s.search_regex) = (false, false, false);
    h.state_mut().doc_mut().editor.set_text("one two one\nONE");
    h.state_mut().doc_mut().version += 1;
    h.state_mut().pending.push(C::Find);
    h.run_steps(3);
    type_text(&mut h, "one");
    assert!(h.query_by_label("共 3 個符合").is_some());
    assert!(h.state_mut().tab_mut().view.find_regex.is_some());
    // The option chips are named checkboxes.
    h.get_by(|n| n.role() == Role::CheckBox && n.label().as_deref() == Some("區分大小寫")).click();
    h.run_steps(2);
    assert!(h.state_mut().settings.search_match_case);
    assert!(h.query_by_label("共 2 個符合").is_some());
    h.state_mut().pending.push(C::FindNext);
    h.run_steps(3);
    assert!(h.query_by_label("1 / 2").is_some());
    h.state_mut().find.query = "zzz".into();
    h.run_steps(2);
    assert!(h.query_by_label("無結果").is_some());
    h.state_mut().settings.search_regex = true;
    h.state_mut().find.query = "(".into();
    h.run_steps(2);
    assert!(h.query_all_by(|n| n.role() == Role::Label && n.value().is_some_and(|v| v.starts_with("無效的正規表達式"))).next().is_some());
    assert!(h.state_mut().tab_mut().view.find_regex.is_none());
    // Replacing is offered only where it can work.
    h.state_mut().doc_mut().read_only = true;
    h.state_mut().find.replace = true;
    h.run_steps(2);
    assert_eq!(h.query_all_by(|n| n.role() == Role::Button && n.label().as_deref() == Some("全部取代") && n.is_disabled()).count(), 1);
    // Closing the bar removes the highlight.
    h.state_mut().find.query = "one".into();
    h.run_steps(2);
    assert!(h.state_mut().tab_mut().view.find_regex.is_some());
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(!h.state_mut().find.visible);
    assert!(h.state_mut().tab_mut().view.find_regex.is_none());
}

#[test]
fn test_toggle_theme_command_switches_light_and_dark_like_the_icon() {
    use crate::core::settings::ThemeChoice;
    let mut h = harness();
    let ctx = h.ctx.clone();
    h.state_mut().set_theme(&ctx, ThemeChoice::System);
    h.run_steps(3);
    let mut dark = App::is_dark(&h.ctx);
    for _ in 0..3 {
        h.state_mut().pending.push(C::ToggleTheme);
        h.run_steps(3);
        dark = !dark;
        assert_eq!(App::is_dark(&h.ctx), dark);
        assert_eq!(h.state_mut().settings.theme, if dark { ThemeChoice::Dark } else { ThemeChoice::Light });
        // The icon names the same theme the command switched to.
        let label = format!("介面主題：{}", if dark { "深色" } else { "淺色" });
        assert!(h.query_by_label(&label).is_some(), "{label}");
    }
}

#[test]
fn test_theme_choice_can_return_to_system_and_styles_both_themes() {
    use crate::core::settings::ThemeChoice;
    use eframe::egui;
    let mut h = harness();
    let ctx = h.ctx.clone();
    h.state_mut().set_theme(&ctx, ThemeChoice::Dark);
    h.run_steps(2);
    assert!(App::is_dark(&h.ctx));
    h.state_mut().set_theme(&ctx, ThemeChoice::System);
    h.run_steps(2);
    assert_eq!(h.state_mut().settings.theme, ThemeChoice::System);
    assert_eq!(h.ctx.options(|o| o.theme_preference), egui::ThemePreference::System);
    // Both styles carry the shared look, so a system theme switch mid-session keeps it.
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let style = h.ctx.style_of(theme);
        assert_eq!(style.visuals.window_corner_radius, egui::CornerRadius::same(10));
        // Rows stay as compact as before, so long menus fit without scrolling where they can.
        assert_eq!(style.spacing.item_spacing.y, 3.0);
        assert_eq!(style.spacing.button_padding.y, 2.0);
    }
}

#[test]
fn test_light_selection_stays_visible_in_hex_view() {
    use eframe::egui;
    let h = harness();
    // The hex view paints the selection at 60% over a white background.
    let sel = h.ctx.style_of(egui::Theme::Light).visuals.selection.bg_fill.gamma_multiply(0.6);
    let over_white = |c: u8| c as f32 + 255.0 * (1.0 - sel.a() as f32 / 255.0);
    assert!(over_white(sel.r()) <= 210.0 && over_white(sel.g()) <= 230.0, "selection too pale: {sel:?}");
}

#[test]
fn test_options_ok_keeps_theme_picked_from_toolbar() {
    use crate::core::settings::ThemeChoice;
    let mut h = harness();
    h.state_mut().pending.push(C::Options);
    h.run_steps(3);
    // The dialog's draft was taken before this change; OK must not revert it.
    let ctx = h.ctx.clone();
    h.state_mut().set_theme(&ctx, ThemeChoice::Dark);
    h.run_steps(3);
    h.get_by_label("確定").click();
    h.run_steps(3);
    assert!(h.state_mut().dialog.is_none());
    assert_eq!(h.state_mut().settings.theme, ThemeChoice::Dark);
    assert!(App::is_dark(&h.ctx));
}

#[test]
fn test_recent_files_submenu_stays_in_place_near_window_bottom() {
    for height in [500.0, 700.0, 900.0] {
        let mut h = harness();
        h.set_size(egui::vec2(1000.0, height));
        h.state_mut().settings.recent_files = (0..25)
            .map(|i| std::path::PathBuf::from(format!("/tmp/recent-{i:02}.txt")))
            .collect();
        h.run_steps(3);
        h.get_by_label("檔案(F)").click();
        h.run_steps(3);
        h.get_by_label("　 最近使用的檔案").click();
        h.run_steps(5);
        let first = h.get_by_label("1 /tmp/recent-00.txt").rect();
        for _ in 0..20 {
            h.run_steps(1);
            let current = h.get_by_label("1 /tmp/recent-00.txt").rect();
            assert!((current.top() - first.top()).abs() < 1.0,
                "子選單不應在每幀跳動（視窗高度 {height}）：{first:?} → {current:?}");
        }
        // A stable popup must still allow reaching entries below its visible area.
        h.event(Event::PointerMoved(first.center()));
        h.event(Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -2000.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            modifiers: Modifiers::NONE,
        });
        h.run_steps(15);
        let last = h.get_by_label("25 /tmp/recent-24.txt").rect();
        assert!(last.top() >= 0.0 && last.bottom() <= height,
            "捲動後的最後一項應在視窗內：{last:?}，視窗高度 {height}");
    }
}

#[test]
fn test_short_menus_keep_the_shared_minimum_width() {
    let mut h = harness();
    // Help has a single item; without the floor its popup shrank to that item's width.
    h.get_by_label("說明(H)").click();
    h.run_steps(3);
    let help = h.get_by_label_contains("關於").rect().width();
    assert!(help >= 230.0, "Help menu item is only {help} wide");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    // Submenus (Theme has three short items) share a smaller floor.
    h.get_by_label("檢視(V)").click();
    h.run_steps(3);
    h.get_by_label("　 介面主題").click();
    h.run_steps(3);
    let theme = h.get_by_label("　 深色").rect().width();
    assert!(theme >= 170.0, "Theme submenu item is only {theme} wide");
}

#[test]
fn test_view_menu_groups_display_options_into_submenus() {
    let mut h = harness();
    h.get_by_label("檢視(V)").click();
    h.run_steps(3);
    // Display toggles live in View > Show, not at the top level.
    assert!(h.query_by_label_contains("顯示行號").is_none());
    h.get_by_label("　 顯示").click();
    h.run_steps(3);
    assert!(h.query_by_label_contains("顯示行號").is_some());
    // Word wrap lives in View > Layout and still runs its command.
    h.get_by_label("　 排版").click();
    h.run_steps(3);
    // The toolbar has a same-named button, so match the menu item by its check-mark column.
    h.get_by(|n| n.label().is_some_and(|l| l.starts_with("　 依視窗寬度換行"))).click();
    h.run_steps(3);
    assert_eq!(h.state_mut().settings.wrap_mode, crate::core::settings::WrapMode::ByWindow);
}

#[test]
fn test_quick_controls_move_to_toolbar_when_menu_bar_hidden() {
    use eframe::egui::accesskit::Role;
    let mut h = harness();
    // Wide enough: zoom / theme / quick search sit in the menu bar, toolbar is one row of icons.
    assert!(h.state_mut().quick_in_menubar);
    h.state_mut().settings.show_menubar = false;
    h.run_steps(3);
    assert!(!h.state_mut().quick_in_menubar);
    // Still reachable from the toolbar.
    let ctx = h.ctx.clone();
    h.state_mut().set_theme(&ctx, crate::core::settings::ThemeChoice::Light);
    h.run_steps(3);
    assert_eq!(h.query_all_by(|n| n.role() == Role::Button && n.label().is_some_and(|l| l.starts_with("介面主題："))).count(), 1);
    click_theme_toggle(&mut h);
    assert!(App::is_dark(&h.ctx));
}

/// Clicks the quick-controls theme icon, whose label names the current theme.
fn click_theme_toggle(h: &mut Harness<'static, App>) {
    h.get_by(|n| n.role() == eframe::egui::accesskit::Role::Button && n.label().is_some_and(|l| l.starts_with("介面主題："))).click();
    h.run_steps(3);
}

#[test]
fn test_saved_theme_is_applied_at_startup() {
    use crate::core::settings::{Settings, ThemeChoice};
    use eframe::egui::{Context, ThemePreference};
    // Built on a plain context: the kittest harness sets its own theme after creating the app,
    // which would hide what startup applied. eframe itself leaves the app's choice alone.
    let start = |theme: ThemeChoice| {
        let ctx = Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let app = App::with_settings(&cc, Vec::new(), None, Settings { theme, ..Settings::default() }, None);
        assert_eq!(app.settings.theme, theme);
        ctx.options(|o| o.theme_preference)
    };
    assert_eq!(start(ThemeChoice::Light), ThemePreference::Light);
    assert_eq!(start(ThemeChoice::Dark), ThemePreference::Dark);
    // Without a settings file the defaults apply, and they follow the system.
    assert_eq!(start(Settings::default().theme), ThemePreference::System);
}

#[test]
fn test_theme_toggle_switches_between_light_and_dark_only() {
    use crate::core::settings::ThemeChoice;
    let mut h = harness();
    let ctx = h.ctx.clone();
    // Following the system (a fresh install), the icon names the theme the system uses.
    h.state_mut().set_theme(&ctx, ThemeChoice::System);
    h.run_steps(3);
    let system_dark = App::is_dark(&h.ctx);
    let name = |dark: bool| format!("介面主題：{}", if dark { "深色" } else { "淺色" });
    // It also says that the theme follows the system, which a click ends.
    let following = format!("{} · 跟隨系統", name(system_dark));
    assert!(h.query_by_label(&following).is_some(), "{following}");
    // Each click flips the shown theme and saves it explicitly; System never comes back.
    for i in 1..=4 {
        click_theme_toggle(&mut h);
        let dark = system_dark ^ (i % 2 == 1);
        assert_eq!(App::is_dark(&h.ctx), dark);
        assert_eq!(h.state_mut().settings.theme, if dark { ThemeChoice::Dark } else { ThemeChoice::Light });
        assert!(h.query_by_label(&name(dark)).is_some());
    }
}

#[test]
fn test_options_window_keeps_its_size_across_tabs() {
    use eframe::egui::accesskit::Role;
    let mut h = harness();
    h.state_mut().pending.push(C::Options);
    h.run_steps(4);
    let window = |h: &Harness<'static, App>| h.get_by_role_and_label(Role::Window, "選項").rect();
    let first = window(&h);
    for tab in ["一般", "編輯", "新文件", "列印", "按鍵", "拼字檢查", "一般"] {
        // By role: the Keys page also lists a "拼字檢查" command.
        h.get_by_role_and_label(Role::Tab, tab).click();
        h.run_steps(4);
        let r = window(&h);
        assert!((r.width() - first.width()).abs() < 1.0 && (r.height() - first.height()).abs() < 1.0, "{tab}: {r:?} vs {first:?}");
    }
}

#[test]
fn test_dialog_shortcuts_keep_open_options() {
    let mut h = harness();
    h.state_mut().pending.push(C::Options);
    h.run_steps(3);
    h.get_by_label("編輯").click();
    h.run_steps(3);
    let tab = |h: &mut Harness<'static, App>| match &h.state_mut().dialog {
        Some(Dialog::Options(st)) => Some(st.tab),
        _ => None,
    };
    let edit_tab = tab(&mut h);
    assert!(edit_tab.is_some_and(|t| t != 0));
    // F9 (recent files) and the Options command itself leave the open dialog as it was; a
    // fresh Options dialog would start on the first tab with a new draft.
    h.key_press(egui::Key::F9);
    h.run_steps(3);
    h.state_mut().pending.push(C::Options);
    h.run_steps(3);
    assert_eq!(tab(&mut h), edit_tab);
    // F1 shows the Keys tab, where the shortcuts are listed.
    h.key_press(egui::Key::F1);
    h.run_steps(3);
    assert_eq!(tab(&mut h), Some(super::dialogs::OPTIONS_KEYS_TAB));
}

#[test]
fn test_dialogs_take_the_keyboard_and_answer_enter_and_escape() {
    let mut h = harness();
    h.state_mut().doc_mut().editor.set_text("b\na");
    h.state_mut().doc_mut().version += 1;
    h.run_steps(2);
    // Enter runs the primary action (Sort) instead of typing a newline into the document.
    h.state_mut().pending.push(C::SortOptions);
    h.run_steps(3);
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(h.state_mut().dialog.is_none());
    assert_eq!(text(&mut h), "a\nb");
    // Typing goes nowhere while a dialog is open, and its Esc leaves the find bar alone.
    h.state_mut().pending.push(C::Find);
    h.run_steps(3);
    h.state_mut().pending.push(C::WordCount);
    h.run_steps(3);
    type_text(&mut h, "x");
    assert_eq!(text(&mut h), "a\nb");
    assert_eq!(h.state_mut().find.query, "");
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(h.state_mut().dialog.is_none());
    assert!(h.state_mut().find.visible);
    // A confirmation on top takes Esc for itself.
    h.state_mut().pending.push(C::WordCount);
    h.run_steps(3);
    h.state_mut().confirm = Some(Confirm::Message { title: "t".into(), text: "x".into() });
    h.run_steps(3);
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(h.state_mut().confirm.is_none());
    assert!(h.state_mut().dialog.is_some());
    // While Options waits for a shortcut, Esc only cancels the capture.
    h.state_mut().dialog = None;
    h.run_steps(2);
    h.state_mut().pending.push(C::Options);
    h.run_steps(3);
    if let Some(Dialog::Options(st)) = &mut h.state_mut().dialog {
        st.capturing = Some(C::Find);
    }
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(matches!(&h.state_mut().dialog, Some(Dialog::Options(st)) if st.capturing.is_none()));
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(h.state_mut().dialog.is_none());
}

#[test]
fn test_recent_files_list_explains_an_empty_list() {
    let mut h = harness();
    h.state_mut().settings.recent_files.clear();
    h.state_mut().pending.push(C::RecentFilesList);
    h.run_steps(3);
    assert!(h.query_by_label("還沒有最近使用的檔案").is_some());
    h.state_mut().dialog = None;
    h.state_mut().settings.recent_files = vec![std::path::PathBuf::from("/tmp/a.txt")];
    h.run_steps(2);
    h.state_mut().pending.push(C::RecentFilesList);
    h.run_steps(3);
    assert!(h.query_by_label("沒有符合的檔案").is_none());
    // The filter has the keyboard as soon as the list opens.
    type_text(&mut h, "zzz");
    h.run_steps(2);
    assert!(h.query_by_label("沒有符合的檔案").is_some());
}

#[test]
fn test_fixed_size_dialogs_fit_a_small_window() {
    use eframe::egui::accesskit::Role;
    let mut h = Harness::builder().allow_missing_glyphs().with_size([500.0, 400.0]).build_eframe(|cc| App::new(cc, Vec::new(), None));
    h.state_mut().settings.ui_language = crate::core::settings::UiLanguage::ZhTw;
    h.run_steps(3);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500.0, 400.0));
    for (cmd, title, button) in [(C::Options, "選項", "確定"), (C::KeyboardShortcuts, "鍵盤快捷鍵", "關閉")] {
        h.state_mut().dialog = None;
        h.state_mut().pending.push(cmd);
        h.run_steps(4);
        let window = h.get_by_role_and_label(Role::Window, title).rect();
        let footer = h.get_by(|n| n.role() == Role::Button && n.label().as_deref() == Some(button)).rect();
        assert!(screen.contains_rect(window), "{title}: {window:?}");
        assert!(window.contains_rect(footer), "{title}: {footer:?} outside {window:?}");
    }
    // Too narrow for two columns: every menu title starts at the same x.
    let lefts: Vec<f32> = ["檔案", "編輯"].iter().filter_map(|t| h.query_by_label(t).map(|n| n.rect().left())).collect();
    assert_eq!(lefts.len(), 2);
    assert!((lefts[0] - lefts[1]).abs() < 1.0, "menu titles at x = {lefts:?}");
}

#[test]
fn test_keyboard_shortcuts_dialog_lists_and_filters_bindings() {
    let mut h = harness();
    h.key_press(egui::Key::F1);
    h.run_steps(3);
    assert!(matches!(h.state_mut().dialog, Some(Dialog::Shortcuts { .. })));
    // Rows scrolled out of view are not in the accessibility tree, so narrow the list first.
    if let Some(Dialog::Shortcuts { filter }) = &mut h.state_mut().dialog {
        *filter = "尋找".into();
    }
    h.run_steps(3);
    let find = h.ctx.format_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::F));
    assert!(dialog(&h).query_by_label("尋找").is_some());
    // Each binding is on its own line of the keys cell.
    let cell = |v: String| v.lines().any(|l| l == find);
    assert_eq!(h.query_all_by(|n| n.role() == egui::accesskit::Role::Label && n.value().is_some_and(cell)).count(), 1);
    // Spelled-out modifier names match too, whatever symbols the platform shows.
    if let Some(Dialog::Shortcuts { filter }) = &mut h.state_mut().dialog {
        *filter = "ctrl+f".into();
    }
    h.run_steps(3);
    assert!(dialog(&h).query_by_label("尋找").is_some());
    if let Some(Dialog::Shortcuts { filter }) = &mut h.state_mut().dialog {
        *filter = "zzz-no-such-command".into();
    }
    h.run_steps(3);
    assert!(h.query_by_label("沒有符合的快捷鍵").is_some());
    // "Customize" hands over to Options at the Keys tab, whose command filter is shown.
    h.get_by_label("自訂快捷鍵…").click();
    h.run_steps(3);
    assert!(matches!(&h.state_mut().dialog, Some(Dialog::Options(st)) if st.tab == super::dialogs::OPTIONS_KEYS_TAB));
    // The Keys page lists the commands, without their menu mnemonics.
    assert!(dialog(&h).query_by_label("新增檔案").is_some());
}

#[test]
fn test_options_rows_use_switches_and_flip_from_their_label() {
    use eframe::egui::accesskit::{Role, Toggled};
    use egui_kittest::kittest::NodeT;
    let mut h = harness();
    h.state_mut().pending.push(C::Options);
    h.run_steps(3);
    h.get_by_label("編輯").click();
    h.run_steps(3);
    let state = |h: &Harness<'static, App>| h.get_by_role_and_label(Role::Switch, "自動縮排").accesskit_node().toggled();
    let before = state(&h);
    assert!(matches!(before, Some(Toggled::True | Toggled::False)));
    // Clicking the row's text flips the switch like clicking the switch itself.
    h.get_by(|n| n.role() == Role::Label && n.value().as_deref() == Some("自動縮排")).click();
    h.run_steps(3);
    assert_ne!(state(&h), before);
    h.get_by_role_and_label(Role::Switch, "自動縮排").click();
    h.run_steps(3);
    assert_eq!(state(&h), before);
    // So does the empty space between the text and the switch.
    let switch = h.get_by_role_and_label(Role::Switch, "自動縮排").rect();
    let gap = egui::pos2(switch.left() - 80.0, switch.center().y);
    for pressed in [true, false] {
        h.event(Event::PointerMoved(gap));
        h.event(Event::PointerButton { pos: gap, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    }
    h.run_steps(3);
    assert_ne!(state(&h), before);
}

#[test]
fn test_options_ok_keeps_live_changes_and_drops_stale_capture() {
    use eframe::egui::accesskit::Role;
    let mut h = harness();
    h.state_mut().pending.push(C::Options);
    h.run_steps(3);
    h.get_by_role_and_label(Role::Tab, "編輯").click();
    h.run_steps(3);
    let indent = h.state_mut().settings.auto_indent;
    h.get_by_role_and_label(Role::Switch, "自動縮排").click();
    h.run_steps(3);
    // Meanwhile, outside the dialog: a View toggle and a newly opened file.
    let ruler = !h.state_mut().settings.show_ruler;
    h.state_mut().settings.show_ruler = ruler;
    h.state_mut().settings.recent_files.insert(0, "/tmp/opened-meanwhile.txt".into());
    // Another dialog cannot replace Options, and the status bar says why.
    h.state_mut().pending.push(C::About);
    h.run_steps(3);
    assert!(matches!(h.state_mut().dialog, Some(Dialog::Options(_))));
    assert!(h.state_mut().status.as_ref().is_some_and(|(m, _)| m == "請先關閉選項對話框"));
    // A key capture started on the Keys page ends when another page is shown.
    h.get_by_role_and_label(Role::Tab, "按鍵").click();
    h.run_steps(3);
    dialog(&h).get_by_label("新增檔案").click();
    h.run_steps(3);
    h.get_by_label("新增快捷鍵…").click();
    h.run_steps(3);
    let capturing = |h: &mut Harness<'static, App>| matches!(&h.state_mut().dialog, Some(Dialog::Options(st)) if st.capturing.is_some());
    assert!(capturing(&mut h));
    h.get_by_role_and_label(Role::Tab, "一般").click();
    h.run_steps(3);
    assert!(!capturing(&mut h));
    h.get_by_label("確定").click();
    h.run_steps(3);
    let s = &h.state_mut().settings;
    assert_eq!(s.auto_indent, !indent, "the dialog's change is applied");
    assert_eq!(s.show_ruler, ruler, "the View toggle is not reverted");
    assert_eq!(s.recent_files.first().map(|p| p.to_string_lossy().into_owned()).as_deref(), Some("/tmp/opened-meanwhile.txt"));
}

#[test]
fn test_keyboard_shortcuts_dialog_lists_menus_in_two_columns() {
    let mut h = harness();
    h.state_mut().pending.push(C::KeyboardShortcuts);
    h.run_steps(4);
    // Menu titles visible at the top of the list; the two columns start at different x.
    let lefts: Vec<f32> = ["檔案", "編輯", "搜尋", "檢視", "巨集", "工具", "視窗", "說明"]
        .iter()
        .filter_map(|t| h.query_by_label(t).map(|n| n.rect().left()))
        .collect();
    let (min, max) = lefts.iter().fold((f32::MAX, f32::MIN), |(a, b), &x| (a.min(x), b.max(x)));
    assert!(lefts.len() >= 2 && max - min > 250.0, "menu titles at x = {lefts:?}");
}

#[test]
fn test_theme_toggle_stays_in_menu_bar_when_toolbar_hidden() {
    use eframe::egui::accesskit::Role;
    let mut h = harness();
    h.state_mut().settings.show_toolbar = false;
    h.run_steps(3);
    assert!(h.state_mut().quick_in_menubar);
    assert_eq!(h.query_all_by(|n| n.role() == Role::Button && n.label().is_some_and(|l| l.starts_with("介面主題："))).count(), 1);
}

#[test]
fn test_about_dialog_shows_version_and_opens_shortcuts() {
    let mut h = harness();
    h.state_mut().pending.push(C::About);
    h.run_steps(3);
    assert!(h.query_by_label_contains(env!("CARGO_PKG_VERSION")).is_some());
    h.get_by_label("複製資訊").click();
    h.run_steps(3);
    assert!(h.query_by_label("已複製").is_some());
    // The confirmation goes away after a moment.
    if let Some(Dialog::About { copied_at }) = &mut h.state_mut().dialog {
        *copied_at = Some(-10.0);
    }
    h.run_steps(3);
    assert!(h.query_by_label("複製資訊").is_some());
    h.get_by_label("鍵盤快捷鍵").click();
    h.run_steps(3);
    assert!(matches!(h.state_mut().dialog, Some(Dialog::Shortcuts { .. })));
}

#[test]
fn test_options_choice_wins_over_later_toolbar_change() {
    use crate::core::settings::ThemeChoice;
    use eframe::egui::accesskit::Role;
    let mut h = harness();
    h.state_mut().pending.push(C::Options);
    h.run_steps(3);
    // Pick Light inside the dialog (match the radio button, not the View > Theme menu item).
    h.get_by_role_and_label(Role::RadioButton, "淺色").click();
    h.run_steps(3);
    let ctx = h.ctx.clone();
    h.state_mut().set_theme(&ctx, ThemeChoice::Dark);
    h.run_steps(3);
    h.get_by_label("確定").click();
    h.run_steps(3);
    assert_eq!(h.state_mut().settings.theme, ThemeChoice::Light);
    assert!(!App::is_dark(&h.ctx));
}

#[test]
fn test_paste_event_inserts_text() {
    let mut h = harness();
    h.event(Event::Paste("貼上\n第二行".into()));
    h.run_steps(2);
    assert_eq!(text(&mut h), "貼上\n第二行");
}

#[test]
fn test_close_modified_document_asks_first() {
    let mut h = harness();
    type_text(&mut h, "x");
    h.state_mut().pending.push(C::Close);
    h.run_steps(2);
    assert!(matches!(h.state_mut().confirm, Some(Confirm::Close { .. })));
    assert_eq!(h.state_mut().tabs.len(), 1);
}

#[test]
fn test_quit_keeps_tabs_for_session_and_cancel() {
    let mut h = harness();
    let first = h.state_mut().doc().id;
    h.state_mut().pending.push(C::New);
    h.run_steps(2);
    type_text(&mut h, "保留修改");
    h.state_mut().pending.push(C::Exit);
    h.run_steps(2);
    assert_eq!(h.state_mut().tabs.len(), 2);
    assert_eq!(h.state_mut().tabs[0].doc.id, first);
    h.get_by_label("取消").click();
    h.run_steps(2);
    assert!(!h.state_mut().allow_close);
    assert_eq!(h.state_mut().tabs.len(), 2);
    h.state_mut().pending.push(C::Exit);
    h.run_steps(2);
    h.get_by_label("不儲存").click();
    h.run_steps(2);
    assert!(h.state_mut().allow_close);
    assert_eq!(h.state_mut().tabs.len(), 2, "結束前仍須保留分頁供工作階段記錄");
}

#[test]
fn test_close_save_with_second_confirm_still_closes() {
    // 關閉時選「儲存」而存檔需要再確認（檔案已被外部修改）：確認覆寫後仍要完成關閉
    let p = std::env::temp_dir().join(format!("madedit_ui_close_{}.txt", std::process::id()));
    std::fs::write(&p, "old").unwrap();
    let mut h = Harness::builder().allow_missing_glyphs().with_size([1000.0, 700.0]).build_eframe(|cc| App::new(cc, vec![p.clone()], None));
    h.run_steps(3);
    type_text(&mut h, "x");
    std::fs::write(&p, "changed outside").unwrap();
    h.state_mut().pending.push(C::Close);
    h.run_steps(2);
    h.get_by_label("儲存").click();
    h.run_steps(2);
    assert!(matches!(h.state_mut().confirm, Some(Confirm::OverwriteExternal { .. })));
    h.get_by_label("覆寫").click();
    h.run_steps(2);
    assert!(h.state_mut().confirm.is_none());
    assert!(h.state_mut().doc().path.is_none(), "分頁應已關閉");
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "xold");
    let _ = std::fs::remove_file(&p);
}

#[test]
fn test_open_missing_file_asks_to_create_new_file() {
    // 開啟不存在的檔案時要像 Notepad 一樣詢問是否建立新檔；
    // 選擇建立後分頁綁定原路徑，存檔會真的把檔案寫出來
    let p = std::env::temp_dir().join(format!("madedit_ui_create_{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&p);
    let mut h = Harness::builder().allow_missing_glyphs().with_size([1000.0, 700.0]).build_eframe(|cc| App::new(cc, vec![p.clone()], None));
    h.state_mut().settings.ui_language = crate::core::settings::UiLanguage::ZhTw;
    h.run_steps(3);
    assert!(matches!(h.state_mut().confirm, Some(Confirm::CreateNew { .. })), "檔案不存在時應詢問是否建立");
    h.get_by_label("建立").click();
    h.run_steps(2);
    assert!(h.state_mut().confirm.is_none());
    assert_eq!(h.state_mut().doc().path.as_deref(), Some(p.as_path()), "建立後分頁應綁定原路徑");
    h.state_mut().pending.push(C::Save);
    h.run_steps(3);
    assert!(p.exists(), "存檔後檔案應真的被建立");
    let _ = std::fs::remove_file(&p);
}

#[test]
fn test_open_missing_file_cancel_creates_nothing() {
    // 取消建立時不得新增分頁，也不得在磁碟上留下檔案
    let p = std::env::temp_dir().join(format!("madedit_ui_create_cancel_{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&p);
    let mut h = Harness::builder().allow_missing_glyphs().with_size([1000.0, 700.0]).build_eframe(|cc| App::new(cc, vec![p.clone()], None));
    h.state_mut().settings.ui_language = crate::core::settings::UiLanguage::ZhTw;
    h.run_steps(3);
    let before = h.state_mut().tabs.len();
    h.get_by_label("取消").click();
    h.run_steps(2);
    assert!(h.state_mut().confirm.is_none());
    assert_eq!(h.state_mut().tabs.len(), before, "取消建立不得新增分頁");
    assert!(!p.exists());
}

#[test]
fn test_mode_switch_keeps_content() {
    let mut h = harness();
    type_text(&mut h, "hello");
    h.state_mut().pending.push(C::HexMode);
    h.run_steps(2);
    h.state_mut().pending.push(C::TextMode);
    h.run_steps(2);
    assert_eq!(text(&mut h), "hello");
}

#[test]
fn test_switch_tab_by_number_shortcut() {
    let mut h = harness();
    h.state_mut().pending.push(C::New);
    h.state_mut().pending.push(C::New);
    h.run_steps(2);
    assert_eq!(h.state_mut().tabs.len(), 3);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Num1);
    h.run_steps(2);
    assert_eq!(h.state_mut().active, 0);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Num9);
    h.run_steps(2);
    assert_eq!(h.state_mut().active, 2, "第 9 個快捷鍵切至最後一個分頁");
    h.key_press_modifiers(Modifiers::COMMAND, Key::Num5);
    h.run_steps(2);
    assert_eq!(h.state_mut().active, 2, "分頁不存在時不動作");
}

#[cfg(unix)]
#[test]
fn test_replace_in_files_skips_opened_file_via_symlink_alias() {
    // 以符號連結別名指定資料夾時，已開啟的檔案仍須只在記憶體中取代、不寫入磁碟
    let base = std::env::temp_dir().join(format!("madedit_ui_fif_{}", std::process::id()));
    let real = base.join("real");
    let alias = base.join("alias");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&real).unwrap();
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    let opened = real.join("opened.txt");
    let closed = real.join("closed.txt");
    std::fs::write(&opened, "old").unwrap();
    std::fs::write(&closed, "old").unwrap();

    let mut h = Harness::builder().allow_missing_glyphs().with_size([1000.0, 700.0]).build_eframe(|cc| App::new(cc, vec![opened.clone()], None));
    h.run_steps(3);
    h.state_mut().settings.find_in_files.directory = alias.to_string_lossy().into_owned();
    h.state_mut().start_find_in_files("old".into(), Some("new".into()));
    for _ in 0..200 {
        h.run_steps(1);
        if !h.state_mut().fif.running {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(!h.state_mut().fif.running);
    assert_eq!(h.state_mut().doc().text(), "new");
    assert_eq!(std::fs::read_to_string(&opened).unwrap(), "old", "已開啟的檔案不可被背景寫入");
    assert_eq!(std::fs::read_to_string(&closed).unwrap(), "new");
    let _ = std::fs::remove_dir_all(&base);
}

/// Find in Files results: matches grouped under foldable file rows, each opening its match;
/// the header and the per-file notes are translated text built from the outcome.
#[test]
fn test_find_in_files_results_group_files_and_report_replacements() {
    use egui::accesskit::Role;
    let dir = std::env::temp_dir().join(format!("madedit_ui_fif_panel_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.txt");
    std::fs::write(&a, "foo\n  bar foo\n").unwrap();
    std::fs::write(dir.join("b.txt"), "x foo\n").unwrap();
    std::fs::write(dir.join("c.txt"), "none\n").unwrap();

    let mut h = Harness::builder().allow_missing_glyphs().with_size([1000.0, 700.0]).build_eframe(|cc| App::new(cc, vec![], None));
    h.run_steps(3);
    let run = |h: &mut Harness<'_, App>, replace: Option<&str>| {
        let opts = &mut h.state_mut().settings.find_in_files;
        opts.directory = dir.to_string_lossy().into_owned();
        opts.opened_files_only = false;
        opts.file_filter = "*.txt".into();
        opts.exclude_filter.clear();
        h.state_mut().start_find_in_files("foo".into(), replace.map(str::to_owned));
        for _ in 0..200 {
            h.run_steps(1);
            if !h.state_mut().fif.running {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        h.run_steps(2);
    };
    let has = |h: &Harness<'_, App>, role: Role, name: &str| h.query_all_by(|n| n.role() == role && (n.label().as_deref() == Some(name) || n.value().as_deref() == Some(name))).count() > 0;

    run(&mut h, None);
    assert!(has(&h, Role::Label, "3 筆結果，共掃描 3 個檔案"));
    for row in ["a.txt (2)", "b.txt (1)", "1: foo", "2: bar foo", "1: x foo"] {
        assert!(has(&h, Role::Button, row), "missing row {row}");
    }

    h.get_by(|n| n.role() == Role::Button && n.label().as_deref() == Some("a.txt (2)")).click();
    h.run_steps(2);
    assert!(!has(&h, Role::Button, "2: bar foo"), "a folded file hides its matches");
    assert!(has(&h, Role::Button, "1: x foo"), "other files stay open");
    h.state_mut().pending.push(C::CollapseAllResults);
    h.run_steps(2);
    assert!(!has(&h, Role::Button, "1: x foo") && !has(&h, Role::Button, "2: bar foo"));
    h.state_mut().pending.push(C::CollapseAllResults);
    h.run_steps(2);

    h.get_by(|n| n.role() == Role::Button && n.label().as_deref() == Some("2: bar foo")).click();
    h.run_steps(3);
    let doc = h.state_mut().doc();
    assert_eq!(doc.path.as_deref().map(|p| p.canonicalize().unwrap()), Some(a.canonicalize().unwrap()));
    let sel = doc.editor.selection;
    assert_eq!((sel.anchor.line, sel.anchor.col, sel.head.col), (1, 6, 9));

    // a.txt is open now, so only b.txt is written; the opened copy is replaced in memory.
    run(&mut h, Some("baz"));
    assert!(has(&h, Role::Label, "共掃描 3 個檔案 · 已取代 3 處"));
    assert!(has(&h, Role::Button, "a.txt: 已取代 2 處"));
    assert!(has(&h, Role::Button, "b.txt: 已取代 1 處"));
    assert_eq!(h.state_mut().doc().text(), "baz\n  bar baz\n");
    assert_eq!(std::fs::read_to_string(dir.join("b.txt")).unwrap(), "x baz\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Markdown 預覽：並排顯示時，編輯區捲動後預覽會記錄同步的行號，且錨點隨內容量得
#[test]
fn test_markdown_preview_follows_editor_scroll() {
    let dir = std::env::temp_dir().join(format!("madedit-mdsync-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("long.md");
    let body: String = (0..150).map(|i| format!("## 標題 {i}\n\n段落 {i}\n\n")).collect();
    std::fs::write(&p, body).unwrap();
    let mut h = Harness::builder().allow_missing_glyphs().with_size([1000.0, 700.0]).build_eframe(|cc| App::new(cc, vec![p.clone()], None));
    h.run_steps(3);
    h.state_mut().pending.push(super::C::PreviewMarkdown);
    h.run_steps(2);
    // 改為並排，檢查預覽的實際位置與來源錨點。
    h.state_mut().tab_mut().md_preview.as_mut().unwrap().set_placement(super::mdpreview::Placement::Right);
    h.run_steps(5);
    {
        let pv = h.state_mut().tab_mut().md_preview.as_ref().expect("預覽未開啟");
        assert!(pv.anchors.len() > 100, "錨點數 {}", pv.anchors.len());
        // 錨點 y 應隨行號遞增
        assert!(pv.anchors.windows(2).all(|w| w[0].0 <= w[1].0 && w[0].1 <= w[1].1));
        assert_eq!(pv.synced_line, Some(0));
    }
    h.state_mut().tab_mut().view.top_row = 200;
    h.run_steps(5);
    assert_eq!(h.state_mut().tab_mut().md_preview.as_ref().unwrap().synced_line, Some(200));
    let pv = h.state_mut().tab_mut().md_preview.as_ref().unwrap();
    let expected = pv.anchors.iter().find(|a| a.0 == 200).unwrap().1;
    assert!((pv.scroll_position.0 - expected.min(pv.scroll_position.1)).abs() < 1.0, "須實際捲到來源區塊，不能只更新同步行號");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_markdown_preview_scrolls_within_wrapped_editor_line() {
    use super::mdpreview::Placement;
    use crate::core::settings::WrapMode;
    let mut h = harness();
    h.state_mut().doc_mut().syntax = "Markdown".into();
    h.state_mut().doc_mut().editor.set_text(&format!("{}\n\n結尾\n", "long text ".repeat(1000)));
    h.state_mut().doc_mut().version += 1;
    h.state_mut().settings.wrap_mode = WrapMode::ByWindow;
    h.state_mut().pending.push(C::PreviewMarkdown);
    h.run_steps(3);
    h.state_mut().tab_mut().md_preview.as_mut().unwrap().set_placement(Placement::Right);
    h.run_steps(5);
    h.state_mut().tab_mut().view.top_row = 10;
    h.run_steps(5);
    let tab = h.state_mut().tab_mut();
    let (line, fraction) = tab.view.top_line(&tab.doc.editor);
    assert_eq!(line, 0);
    assert!(fraction > 0.0);
    assert!(tab.md_preview.as_ref().unwrap().scroll_position.0 > 10.0, "同一邏輯行內的換行也須推進預覽");
}

/// 直接送入滾輪事件，驗證實際互動與平滑捲動的後續幀，而非直接修改預覽狀態。
fn scroll_preview(h: &mut Harness<'static, App>, delta: f32) {
    let rect = h.state_mut().tab_mut().md_preview.as_ref().unwrap().rect;
    h.hover_at(eframe::egui::pos2(rect.center().x, rect.bottom() - 80.0));
    h.run_steps(1);
    h.event(Event::MouseWheel {
        unit: eframe::egui::MouseWheelUnit::Point,
        delta: eframe::egui::vec2(0.0, delta),
        phase: eframe::egui::TouchPhase::Move,
        source: eframe::egui::MouseWheelSource::Wheel,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(30);
}

#[test]
fn test_markdown_preview_scrolls_editor_without_feedback_or_selection_change() {
    use super::mdpreview::Placement;
    use crate::core::editor::{Selection, TextPosition};
    for placement in [Placement::Right, Placement::Bottom, Placement::Tab] {
        let mut h = harness();
        h.state_mut().doc_mut().syntax = "Markdown".into();
        h.state_mut().doc_mut().editor.set_text(&(0..150).map(|i| format!("## Heading {i}\n\nParagraph {i}\n\n")).collect::<String>());
        h.state_mut().doc_mut().version += 1;
        let selection = Selection::new(TextPosition::new(0, 1), TextPosition::new(2, 3));
        h.state_mut().doc_mut().editor.selection = selection;
        h.state_mut().pending.push(C::PreviewMarkdown);
        h.run_steps(3);
        h.state_mut().tab_mut().md_preview.as_mut().unwrap().set_placement(placement);
        h.run_steps(8);
        scroll_preview(&mut h, -500.0);
        let tab = h.state_mut().tab_mut();
        let first = tab.view.top_row;
        assert!(first > 10, "預覽捲動後須帶動編輯區：{placement:?}, {first}");
        assert_eq!(tab.doc.editor.selection, selection, "同步不能移動游標或選取");
        let offset = tab.md_preview.as_ref().unwrap().scroll_position.0;
        h.run_steps(15);
        let tab = h.state_mut().tab_mut();
        assert_eq!(tab.view.top_row, first, "閒置後來源位置不能繼續來回變動");
        assert!((tab.md_preview.as_ref().unwrap().scroll_position.0 - offset).abs() < 0.1, "預覽不能被整行取整誤差拉回");
        // 接著改由編輯區捲動，確認主導權能立即交回。
        tab.view.top_row = first + 40;
        h.run_steps(5);
        assert_eq!(h.state_mut().tab_mut().md_preview.as_ref().unwrap().synced_line, Some(first + 40));
        h.get_by_label("同步捲動").click();
        h.run_steps(3);
        scroll_preview(&mut h, -400.0);
        assert_eq!(h.state_mut().tab_mut().view.top_row, first + 40, "停用同步後兩側須可各自閱讀");
    }
}

#[test]
fn test_markdown_preview_reverse_scrolls_within_wrapped_line() {
    use super::mdpreview::Placement;
    use crate::core::settings::WrapMode;
    let mut h = harness();
    h.state_mut().doc_mut().syntax = "Markdown".into();
    h.state_mut().doc_mut().editor.set_text(&format!("{}\n\nend\n", "long text ".repeat(1000)));
    h.state_mut().doc_mut().version += 1;
    h.state_mut().settings.wrap_mode = WrapMode::ByWindow;
    h.state_mut().pending.push(C::PreviewMarkdown);
    h.run_steps(3);
    h.state_mut().tab_mut().md_preview.as_mut().unwrap().set_placement(Placement::Right);
    h.run_steps(8);
    scroll_preview(&mut h, -700.0);
    let tab = h.state_mut().tab_mut();
    let (line, fraction) = tab.view.top_line(&tab.doc.editor);
    assert_eq!(line, 0);
    assert!(fraction > 0.0 && tab.view.top_row > 0, "長段落內須帶動來源的換行列");
    let before = tab.md_preview.as_ref().unwrap().scroll_position.0;
    h.run_steps(15);
    assert!((h.state_mut().tab_mut().md_preview.as_ref().unwrap().scroll_position.0 - before).abs() < 0.1);
}

/// Markdown 預覽：切成左右／上下並排時各佔一半，拖曳後須保留新的尺寸。
#[test]
fn test_markdown_preview_docks_at_half_size() {
    use super::mdpreview::Placement;
    let dir = std::env::temp_dir().join(format!("madedit-mddock-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("dock.md");
    // 內含寬圖表，確認內容不會把面板撐大
    let table = format!("\n| {} |\n| {} |\n| {} |\n", ["欄位"; 12].join(" | "), ["---"; 12].join(" | "), ["LongUnbrokenCellValue"; 12].join(" | "));
    std::fs::write(&p, format!("# 標題\n\n```mermaid\nflowchart LR\n  A[很長很長的節點一] --> B[很長很長的節點二] --> C[很長很長的節點三] --> D[很長很長的節點四]\n```\n{table}")).unwrap();
    let mut h = Harness::builder().allow_missing_glyphs().with_size([1000.0, 700.0]).build_eframe(|cc| App::new(cc, vec![p.clone()], None));
    h.run_steps(3);
    h.state_mut().pending.push(super::C::PreviewMarkdown);
    h.run_steps(3);
    let mut half_y = 0.0;
    for placement in [Placement::Right, Placement::Bottom, Placement::Right] {
        h.get_by_label(if placement == Placement::Right { "左右並排" } else { "上下並排" }).click();
        h.run_steps(8);
        // 模擬原生預覽關閉後，主視窗才收到新的可用尺寸；仍須重新平分。
        h.set_size(eframe::egui::vec2(900.0, 650.0));
        h.run_steps(4);
        let resized = h.get_by_role_and_label(eframe::egui::accesskit::Role::Splitter, "Resize panel").rect();
        if placement == Placement::Right { assert!((resized.center().x - 450.0).abs() < 1.0); }
        h.set_size(eframe::egui::vec2(1000.0, 700.0));
        h.run_steps(4);
        let handle = h.get_by_role_and_label(eframe::egui::accesskit::Role::Splitter, "Resize panel").rect();
        let pv = h.state_mut().tab_mut().md_preview.as_ref().unwrap();
        let (w, hgt) = (pv.rect.width(), pv.rect.height());
        if placement == Placement::Right {
            assert!((w - 500.0).abs() < 20.0, "左右並排寬度 {w}");
            assert!((handle.center().x - 500.0).abs() < 1.0, "分隔線應在正中央");
            half_y = handle.center().y;
        } else {
            assert!(hgt > 200.0 && hgt < 400.0 && w > 900.0, "上下並排 {w}x{hgt}");
            assert!((handle.center().y - half_y).abs() < 1.0, "上下並排分隔線應在可用高度中央");
        }
        // 第二次拖曳幅度不同，留下非一半的尺寸，再驗證切換擺放方式時確實重設。
        for direction in [1.0, -1.5] {
            let before = h.state_mut().tab_mut().md_preview.as_ref().unwrap().rect;
            let handle = h.get_by_role_and_label(eframe::egui::accesskit::Role::Splitter, "Resize panel").rect();
            // 從分隔線中央抓取，才能抓到與編輯區捲軸重疊的回歸問題。
            let start = handle.center();
            let end = start + direction * if placement == Placement::Right { eframe::egui::vec2(100.0, 0.0) } else { eframe::egui::vec2(0.0, 70.0) };
            h.hover_at(start);
            h.drag_at(start);
            h.run_steps(2);
            if direction > 0.0 {
                h.hover_at(end);
                h.run_steps(3);
                h.drop_at(end);
            } else {
                // Harness 的 event() 會將各事件拆成不同幀；直接寫入 RawInput，
                // 才能重現 Computer Use 的「移動與放開同幀」。
                h.input_mut().events.extend([
                    Event::PointerMoved(end),
                    Event::PointerButton { pos: end, button: eframe::egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE },
                ]);
            }
            h.run_steps(8);
            // 文件更新與圖表重新排版後，也不能覆蓋使用者調整的大小。
            h.state_mut().doc_mut().version += 1;
            h.run_steps(8);
            let after = h.state_mut().tab_mut().md_preview.as_ref().unwrap().rect;
            let (before_size, after_size, delta) = if placement == Placement::Right {
                (before.width(), after.width(), end.x - start.x)
            } else { (before.height(), after.height(), end.y - start.y) };
            assert!((before_size - after_size).abs() > 40.0, "拖曳應改變面板大小：{placement:?}, {before_size} → {after_size}");
            assert!((before_size - delta - after_size).abs() < 2.0, "放開後不能彈回：{placement:?}, 預期 {}，實際 {after_size}", before_size - delta);
        }
        let manual = h.state_mut().tab_mut().md_preview.as_ref().unwrap().rect;
        h.set_size(eframe::egui::vec2(1100.0, 760.0));
        h.run_steps(5);
        let resized = h.state_mut().tab_mut().md_preview.as_ref().unwrap().rect;
        let difference = if placement == Placement::Right { manual.width() - resized.width() } else { manual.height() - resized.height() };
        assert!(difference.abs() < 2.0, "使用者調整後，主視窗改變大小也不能強制改回一半");
        h.set_size(eframe::egui::vec2(1000.0, 700.0));
        h.run_steps(5);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_backspace_wrapped_selection_does_not_panic() {
    // 自動換行時刪除多行選取：同一幀內換行快取須先更新再繪製，否則以舊的斷行位置索引新內容而 panic
    use crate::core::editor::{Selection, TextPosition};
    use crate::core::settings::WrapMode;
    let mut h = Harness::builder().allow_missing_glyphs().with_size([400.0, 700.0]).build_eframe(|cc| App::new(cc, Vec::new(), None));
    h.run_steps(3);
    h.state_mut().settings.wrap_mode = WrapMode::ByWindow;
    let long = "word ".repeat(60);
    h.state_mut().doc_mut().editor.set_text(&format!("{long}\n{long}\n\nend"));
    h.state_mut().doc_mut().version += 1;
    h.run_steps(2);
    h.state_mut().doc_mut().editor.selection = Selection::new(TextPosition::new(0, 0), TextPosition::new(2, 0));
    h.run_steps(2);
    h.key_press(Key::Backspace);
    h.run_steps(2);
    assert_eq!(text(&mut h), "\nend");
}

/// Markdown 預覽屬於各自的分頁：切到別的分頁就不顯示，切回來仍保留原本的擺放方式
#[test]
fn test_markdown_preview_follows_active_tab() {
    use super::mdpreview::Placement;
    let dir = std::env::temp_dir().join(format!("madedit-mdtab-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (a, b) = (dir.join("a.md"), dir.join("b.md"));
    std::fs::write(&a, "# A\n").unwrap();
    std::fs::write(&b, "# B\n").unwrap();
    let mut h = Harness::builder().allow_missing_glyphs().with_size([1000.0, 700.0]).build_eframe(|cc| App::new(cc, vec![a, b], None));
    h.run_steps(3);
    h.state_mut().activate(0);
    h.state_mut().pending.push(C::PreviewMarkdown);
    h.run_steps(3);
    h.state_mut().tab_mut().md_preview.as_mut().unwrap().set_placement(Placement::Right);
    h.run_steps(3);
    h.state_mut().pending.push(C::NextWindow);
    h.run_steps(3);
    assert_eq!(h.state_mut().active, 1);
    assert!(h.state_mut().tab_mut().md_preview.is_none(), "另一個分頁不應顯示預覽");
    h.state_mut().pending.push(C::NextWindow);
    h.run_steps(3);
    let pv = h.state_mut().tab_mut().md_preview.as_ref().expect("切回後應恢復預覽");
    assert_eq!(pv.placement, Placement::Right);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 預覽分頁共享來源、支援切換與關閉，且不能新增原生視窗或修改隱藏的來源。
#[test]
fn test_markdown_preview_tab_reuses_source_and_never_opens_a_viewport() {
    let mut h = harness();
    h.state_mut().doc_mut().syntax = "Markdown".into();
    type_text(&mut h, "# Preview heading");
    let source_id = h.state().doc().id;
    let source_name = format!("{} *", h.state().doc().display_name());
    let preview_name = format!("{} — Markdown 預覽", h.state().doc().display_name());
    h.state_mut().pending.push(C::PreviewMarkdown);
    h.run_steps(5);
    assert!(h.state().is_preview_tab());
    assert_eq!(h.state().tabs.len(), 1, "預覽不得複製來源文件");
    assert_eq!(h.state().tab_pages(), vec![(0, false), (0, true)]);
    assert_eq!(h.output().viewport_output.len(), 1, "預覽不能建立額外原生視窗");
    h.get_by_label(&preview_name);
    h.get_by_label("Preview heading");
    // 預覽分頁不能讓貼上、輸入或復原修改隱藏中的來源文件。
    h.event(Event::Paste("should not edit".into()));
    type_text(&mut h, "should not edit");
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    assert_eq!(text(&mut h), "# Preview heading");
    h.get_by_label(&source_name).click();
    h.run_steps(3);
    assert!(!h.state().is_preview_tab());
    type_text(&mut h, " updated");
    h.get_by_label(&preview_name).click();
    h.run_steps(5);
    h.get_by_label("Preview heading updated");
    h.state_mut().pending.push(C::PreviewMarkdown);
    h.run_steps(3);
    assert_eq!(h.state().tab_pages().len(), 2, "重複開啟只選取既有預覽");
    h.key_press_modifiers(Modifiers::COMMAND, Key::Num1);
    h.run_steps(2);
    assert!(!h.state().is_preview_tab());
    h.key_press_modifiers(if cfg!(target_os = "macos") { Modifiers::CTRL } else { Modifiers::COMMAND }, Key::Tab);
    h.run_steps(2);
    assert!(h.state().is_preview_tab(), "切換分頁須包含預覽");
    h.state_mut().pending.push(C::Close);
    h.run_steps(3);
    assert!(!h.state().is_preview_tab());
    assert!(h.state_mut().tab_mut().md_preview.is_none());
    assert_eq!(h.state().doc().id, source_id);
    assert!(h.state().doc().is_modified());
    assert_eq!(text(&mut h), "# Preview heading updated");
    assert!(h.state().confirm.is_none(), "關閉預覽不應關閉或詢問儲存來源");
}

/// 在分頁與並排間切換，並確認關閉來源時不留下預覽，也不影響其他文件。
#[test]
fn test_markdown_preview_tab_layout_and_document_lifetime() {
    let mut h = harness();
    h.state_mut().doc_mut().syntax = "Markdown".into();
    type_text(&mut h, "# First");
    let first_id = h.state().doc().id;
    h.state_mut().pending.push(C::PreviewMarkdown);
    h.run_steps(4);
    h.get_by_label("左右並排").click();
    h.run_steps(5);
    assert!(!h.state().is_preview_tab());
    assert_eq!(h.state().tab_pages().len(), 1);
    h.get_by_label("獨立分頁").click();
    h.run_steps(5);
    assert!(h.state().is_preview_tab());
    assert_eq!(h.state().tab_pages().len(), 2);
    h.state_mut().pending.push(C::New);
    h.run_steps(3);
    h.state_mut().doc_mut().syntax = "Markdown".into();
    type_text(&mut h, "# Second");
    let second_id = h.state().doc().id;
    h.state_mut().pending.push(C::PreviewMarkdown);
    h.run_steps(4);
    assert_eq!(h.state().tab_pages().len(), 4);
    assert_eq!(h.output().viewport_output.len(), 1);
    // 關閉別份來源時，仍在閱讀的預覽要跟著原文件，而不是跟著陣列索引。
    h.state_mut().close_now(first_id);
    h.run_steps(3);
    assert!(h.state().is_preview_tab());
    assert_eq!(h.state().doc().id, second_id);
    h.get_by_label("Second");
    h.state_mut().activate(0);
    h.state_mut().close_now(second_id);
    h.run_steps(3);
    assert!(!h.state().is_preview_tab());
    assert_eq!(h.state().tab_pages().len(), 1, "關閉來源必須移除它的預覽分頁");
    assert!(h.state().tabs.iter().all(|t| t.md_preview.is_none()));
}

/// Markdown 預覽只對 Markdown 檔開啟：其他檔案即使以快捷鍵觸發也不會開
#[test]
fn test_markdown_preview_only_for_markdown() {
    let dir = std::env::temp_dir().join(format!("madedit-mdonly-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (txt, md) = (dir.join("a.txt"), dir.join("b.md"));
    std::fs::write(&txt, "# A\n").unwrap();
    std::fs::write(&md, "# B\n").unwrap();
    let mut h = Harness::builder().allow_missing_glyphs().with_size([1000.0, 700.0]).build_eframe(|cc| App::new(cc, vec![txt, md], None));
    h.run_steps(3);
    h.state_mut().activate(0);
    assert!(!h.state_mut().doc().is_markdown());
    h.state_mut().pending.push(C::PreviewMarkdown);
    h.run_steps(3);
    assert!(h.state_mut().tab_mut().md_preview.is_none(), "純文字檔不應開啟預覽");
    h.state_mut().activate(1);
    assert!(h.state_mut().doc().is_markdown());
    h.state_mut().pending.push(C::PreviewMarkdown);
    h.run_steps(3);
    assert!(h.state_mut().tab_mut().md_preview.is_some(), "Markdown 檔應開啟預覽");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 另存檔名去除前後空白與結尾句點() {
    use std::path::PathBuf;
    let t = |s: &str| App::trim_file_name(PathBuf::from(s));
    assert_eq!(t("/a/ b.txt"), PathBuf::from("/a/b.txt"));
    assert_eq!(t("/a/b.txt "), PathBuf::from("/a/b.txt"));
    assert_eq!(t("/a/b. "), PathBuf::from("/a/b"));
    assert_eq!(t("/a/ b . . "), PathBuf::from("/a/b"));
    assert_eq!(t("/a/.hidden"), PathBuf::from("/a/.hidden"));
    assert_eq!(t("/a /b.txt"), PathBuf::from("/a /b.txt"));
}

#[test]
fn test_every_dialog_keeps_its_buttons_inside_its_window() {
    // The smallest main window, and a normal one where tall dialogs once clipped their footer.
    for size in [egui::vec2(400.0, 300.0), egui::vec2(1100.0, 720.0)] {
        dialogs_keep_their_buttons_inside(size);
    }
}

fn dialogs_keep_their_buttons_inside(size: egui::Vec2) {
    use eframe::egui::accesskit::Role;
    let mut h = Harness::builder().allow_missing_glyphs().with_size(size).build_eframe(|cc| App::new(cc, Vec::new(), None));
    h.state_mut().settings.ui_language = crate::core::settings::UiLanguage::ZhTw;
    h.run_steps(3);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
    let dialogs = [
        (C::SetFont, "確定"),
        (C::PageSetup, "關閉"),
        (C::RecentFilesList, "關閉"),
        (C::ToFullWidthByOptions, "確定"),
        (C::InsertNumbers, "確定"),
        (C::SortOptions, "排序"),
        (C::FindInFiles, "全部尋找"),
        (C::RunTempMacro, "執行"),
        (C::Highlighting, "關閉"),
        (C::SourceFormatter, "關閉"),
        (C::PurgeHistories, "清除"),
        (C::ConvertEncoding, "轉換"),
        (C::WordCount, "關閉"),
        (C::WindowList, "關閉清單"),
        (C::About, "確定"),
    ];
    for (cmd, button) in dialogs {
        h.state_mut().dialog = None;
        h.state_mut().pending.push(cmd);
        h.run_steps(4);
        assert!(h.state_mut().dialog.is_some(), "{cmd:?} opened no dialog");
        let windows: Vec<egui::Rect> = h.query_all_by(|n| n.role() == Role::Window).map(|n| n.rect()).collect();
        let rects: Vec<egui::Rect> = h.query_all_by(|n| n.role() == Role::Button && n.label().as_deref() == Some(button)).map(|n| n.rect()).collect();
        assert!(!rects.is_empty(), "{cmd:?}: no {button} button");
        for r in rects {
            assert!(screen.contains_rect(r), "{cmd:?} at {size:?}: {button} at {r:?} is off screen");
            assert!(windows.iter().any(|w| w.contains_rect(r)), "{cmd:?} at {size:?}: {button} at {r:?} is outside its window {windows:?}");
        }
    }
}

/// Opens a dialog through its command and clicks one of its buttons.
fn click_in_dialog(h: &mut Harness<'static, App>, cmd: C, button: &str) {
    h.state_mut().pending.push(cmd);
    h.run_steps(3);
    h.get_by(|n| n.role() == egui::accesskit::Role::Button && n.label().as_deref() == Some(button)).click();
    h.run_steps(3);
}

/// The open dialog's window, to search it without matching toolbar buttons of the same name
/// (toolbar icons are named after their commands, like the rows of the shortcut lists).
fn dialog<'t>(h: &'t Harness<'static, App>) -> egui_kittest::Node<'t> {
    h.get_by(|n| n.role() == egui::accesskit::Role::Window && n.parent().is_some())
}

/// Clicks a button of the open dialog or confirmation by its exact label.
fn click_button(h: &mut Harness<'static, App>, label: &str) {
    h.get_by(|n| n.role() == egui::accesskit::Role::Button && n.label().as_deref() == Some(label)).click();
    h.run_steps(3);
}

#[test]
fn test_clear_history_dialog_clears_and_closes() {
    let mut h = harness();
    h.state_mut().settings.recent_files = vec![std::path::PathBuf::from("/tmp/a.txt")];
    click_in_dialog(&mut h, C::PurgeHistories, "清除");
    assert!(h.state_mut().settings.recent_files.is_empty());
    assert!(h.state_mut().dialog.is_none());
}

#[test]
fn test_full_width_dialog_converts_the_whole_document() {
    let mut h = harness();
    type_text(&mut h, "ab1");
    click_in_dialog(&mut h, C::ToFullWidthByOptions, "確定");
    assert_eq!(text(&mut h), "ａｂ１");
    assert!(h.state_mut().dialog.is_none());
}

#[test]
fn test_insert_numbers_dialog_inserts_one_per_line() {
    let mut h = harness();
    h.state_mut().settings.numbers.total = 3;
    click_in_dialog(&mut h, C::InsertNumbers, "確定");
    assert_eq!(text(&mut h), "1\n2\n3");
}

#[test]
fn test_sort_dialog_uses_its_options() {
    let mut h = harness();
    h.state_mut().doc_mut().replace_all_text("b\na\nc");
    h.state_mut().settings.sort.descending = true;
    click_in_dialog(&mut h, C::SortOptions, "排序");
    assert_eq!(text(&mut h), "c\nb\na");
}

#[test]
fn test_convert_encoding_dialog_switches_the_encoding() {
    let mut h = harness();
    type_text(&mut h, "abc");
    h.state_mut().pending.push(C::ConvertEncoding);
    h.run_steps(3);
    if let Some(Dialog::ConvertEncoding { encoding, .. }) = &mut h.state_mut().dialog {
        *encoding = "UTF-16LE".into();
    }
    h.run_steps(2);
    click_button(&mut h, "轉換");
    assert_eq!(h.state_mut().doc().encoding.name(), "UTF-16LE");
    assert!(h.state_mut().dialog.is_none());
}

#[test]
fn test_temp_macro_dialog_reports_errors_and_runs_valid_scripts() {
    let mut h = harness();
    h.state_mut().pending.push(C::RunTempMacro);
    h.run_steps(3);
    let run = |h: &mut Harness<'static, App>, source: &str| {
        if let Some(Dialog::TempMacro { script, .. }) = &mut h.state_mut().dialog {
            *script = source.into();
        }
        h.run_steps(2);
        click_button(h, "執行");
    };
    run(&mut h, "not json");
    assert!(matches!(h.state_mut().dialog, Some(Dialog::TempMacro { error: Some(_), .. })));
    run(&mut h, "[]");
    assert!(h.state_mut().dialog.is_none());
}

#[test]
fn test_highlighting_dialog_applies_the_ui_theme_at_once() {
    let mut h = harness();
    h.state_mut().pending.push(C::Highlighting);
    h.run_steps(3);
    h.get_by_role_and_label(egui::accesskit::Role::RadioButton, "深色").click();
    h.run_steps(3);
    assert_eq!(h.state_mut().settings.theme, crate::core::settings::ThemeChoice::Dark);
}

/// First installed proportional font from a list common on Windows, macOS and Linux.
fn installed_proportional_font(h: &mut Harness<'static, App>) -> Option<String> {
    let families = &h.state_mut().fonts.families;
    ["Arial", "Segoe UI", "Helvetica", "DejaVu Sans", "Liberation Sans", "Noto Sans"]
        .into_iter()
        .find(|f| families.iter().any(|n| n == f))
        .map(str::to_string)
}

/// Proportional layout must draw each glyph at the x the caret and selection use. egui kerns a
/// multi-character run, so a long run drawn as one string drifted from those positions and left
/// a visible gap before the next run (a closing quote after a base64 string).
#[test]
fn test_proportional_text_is_drawn_at_layout_positions() {
    let mut h = harness();
    let Some(font) = installed_proportional_font(&mut h) else {
        eprintln!("skipped: no known proportional font installed");
        return;
    };
    h.state_mut().settings.font_family = font;
    h.state_mut().settings.fixed_width = false;
    h.run_steps(3);
    type_text(&mut h, "\"AVAWAVTo Yo Wa LT=\"");
    h.run_steps(2);
    assert_eq!(text(&mut h), "\"AVAWAVTo Yo Wa LT=\"");
    let rect = h.state_mut().tabs[0].view.text_rect;
    let mut drawn = String::new();
    let mut shapes: Vec<&egui::Shape> = h.output().shapes.iter().map(|c| &c.shape).collect();
    while let Some(shape) = shapes.pop() {
        match shape {
            egui::Shape::Vec(v) => shapes.extend(v.iter()),
            // The first row's glyphs start a fraction of a point above the text area.
            egui::Shape::Text(t) if rect.expand(2.0).contains(t.pos) && t.galley.text().chars().any(|c| "AVWTLoaY\"=".contains(c)) => {
                assert_eq!(t.galley.text().chars().count(), 1, "run {:?} drawn as one string", t.galley.text());
                drawn.push_str(t.galley.text());
            }
            _ => {}
        }
    }
    assert!(drawn.contains('=') && drawn.matches('"').count() == 2, "editor text not found among shapes: {drawn:?}");
}

#[test]
fn test_proportional_font_in_fixed_width_mode_offers_to_turn_it_off() {
    let mut h = harness();
    let Some(font) = installed_proportional_font(&mut h) else {
        eprintln!("skipped: no known proportional font installed");
        return;
    };
    assert!(!h.state_mut().settings.fixed_width, "fixed-width mode is off by default");
    // Off: picking a proportional font asks nothing.
    h.state_mut().settings.font_family = font.clone();
    h.run_steps(3);
    assert!(h.state_mut().confirm.is_none());
    // On: picking it again (after another font) asks to turn the mode off, and the button does.
    h.state_mut().settings.fixed_width = true;
    h.state_mut().settings.font_family = String::new();
    h.run_steps(3);
    h.state_mut().confirm = None;
    h.state_mut().settings.font_family = font;
    h.run_steps(3);
    assert!(matches!(h.state_mut().confirm, Some(Confirm::ProportionalFont)));
    click_button(&mut h, "關閉等寬模式");
    assert!(h.state_mut().confirm.is_none());
    assert!(!h.state_mut().settings.fixed_width);
}

#[test]
fn test_confirmations_keep_their_buttons_on_a_small_window() {
    use eframe::egui::accesskit::Role;
    let mut h = Harness::builder().allow_missing_glyphs().with_size([400.0, 300.0]).build_eframe(|cc| App::new(cc, Vec::new(), None));
    h.state_mut().settings.ui_language = crate::core::settings::UiLanguage::ZhTw;
    h.run_steps(3);
    let id = h.state_mut().doc().id;
    let path = Some(std::path::PathBuf::from("/tmp/x.txt"));
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0));
    let cases = [
        (Confirm::Close { ids: vec![id], exit: false }, "儲存"),
        (Confirm::SaveLossy { id, path: path.clone() }, "仍要存檔"),
        (Confirm::SaveUnmappable { id, path: path.clone(), message: "x ".repeat(300) }, "改用 UTF-8 存檔"),
        (Confirm::OverwriteExternal { id, path: path.clone() }, "覆寫"),
        (Confirm::ReloadModified { id }, "重新載入"),
        (Confirm::CreateNew { path: std::path::PathBuf::from("/tmp/x-new.txt") }, "建立"),
        (Confirm::Message { title: "t".into(), text: "long ".repeat(200) }, "確定"),
        (Confirm::ProportionalFont, "關閉等寬模式"),
    ];
    for (confirm, button) in cases {
        h.state_mut().confirm = Some(confirm);
        h.run_steps(4);
        let r = h.get_by(|n| n.role() == Role::Button && n.label().as_deref() == Some(button)).rect();
        assert!(screen.contains_rect(r), "{button} at {r:?} is off screen");
        h.state_mut().confirm = None;
        h.run_steps(2);
    }
}

#[test]
fn test_dialog_reopens_scrolled_to_the_top() {
    use eframe::egui::accesskit::Role;
    let mut h = Harness::builder().allow_missing_glyphs().with_size([400.0, 300.0]).build_eframe(|cc| App::new(cc, Vec::new(), None));
    h.state_mut().settings.ui_language = crate::core::settings::UiLanguage::ZhTw;
    h.run_steps(3);
    let first_row = |h: &Harness<'static, App>| h.query_by(|n| n.role() == Role::Switch && n.label().as_deref() == Some("列印語法色彩")).map(|n| n.rect());
    h.state_mut().pending.push(C::PageSetup);
    h.run_steps(4);
    let top = first_row(&h).expect("first row visible on opening");
    h.event(Event::PointerMoved(top.center()));
    h.event(Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0.0, -400.0), modifiers: Modifiers::NONE, phase: egui::TouchPhase::Move, source: egui::MouseWheelSource::Wheel });
    h.run_steps(4);
    assert!(first_row(&h).is_none_or(|r| r.top() < top.top() - 50.0), "the wheel did not scroll the dialog");
    click_button(&mut h, "關閉");
    assert!(h.state_mut().dialog.is_none());
    h.state_mut().pending.push(C::PageSetup);
    h.run_steps(4);
    let again = first_row(&h).expect("first row visible after reopening");
    assert!((again.top() - top.top()).abs() < 1.0, "reopened at {again:?}, first opened at {top:?}");
}

#[test]
fn test_window_list_close_buttons_are_named_for_screen_readers() {
    let mut h = harness();
    h.state_mut().pending.push(C::New);
    h.run_steps(2);
    assert_eq!(h.state_mut().tabs.len(), 2);
    let names: Vec<String> = h.state_mut().tabs.iter().map(|t| format!("關閉 {}", t.doc.display_name())).collect();
    h.state_mut().pending.push(C::WindowList);
    h.run_steps(3);
    for name in &names {
        // One in the tab bar and one in the list below it.
        assert_eq!(h.query_all_by(|n| n.role() == egui::accesskit::Role::Button && n.label().as_deref() == Some(name.as_str())).count(), 2, "{name}");
    }
    let mut pair: Vec<_> = h.query_all_by(|n| n.role() == egui::accesskit::Role::Button && n.label().as_deref() == Some(names[1].as_str())).collect();
    pair.sort_by(|a, b| a.rect().top().total_cmp(&b.rect().top()));
    assert!(pair[1].rect().top() > pair[0].rect().bottom(), "the list's button sits below the tab bar's");
    pair[1].click();
    h.run_steps(3);
    assert_eq!(h.state_mut().tabs.len(), 1);
    assert_eq!(format!("關閉 {}", h.state_mut().tabs[0].doc.display_name()), names[0]);
}

#[test]
fn test_tab_close_buttons_are_named_for_screen_readers() {
    let mut h = harness();
    h.state_mut().pending.push(C::New);
    h.run_steps(2);
    let names: Vec<String> = h.state_mut().tabs.iter().map(|t| format!("關閉 {}", t.doc.display_name())).collect();
    assert_eq!(names.len(), 2);
    for name in &names {
        assert_eq!(h.query_all_by(|n| n.role() == egui::accesskit::Role::Button && n.label().as_deref() == Some(name.as_str())).count(), 1, "{name}");
    }
    h.get_by(|n| n.role() == egui::accesskit::Role::Button && n.label().as_deref() == Some(names[0].as_str())).click();
    h.run_steps(3);
    assert_eq!(h.state_mut().tabs.len(), 1);
}

#[test]
fn test_preview_tab_close_button_is_named_and_closes_only_the_preview() {
    use super::mdpreview::Placement;
    let mut h = harness();
    h.state_mut().doc_mut().syntax = "Markdown".into();
    h.state_mut().doc_mut().editor.set_text("# Title\n");
    h.state_mut().doc_mut().version += 1;
    let original = h.state_mut().doc_mut().text();
    h.state_mut().pending.push(C::PreviewMarkdown);
    h.run_steps(3);
    h.state_mut().tab_mut().md_preview.as_mut().unwrap().set_placement(Placement::Tab);
    h.run_steps(3);
    let name = format!("關閉預覽 {}", h.state_mut().doc_mut().display_name());
    h.get_by(|n| n.role() == egui::accesskit::Role::Button && n.label().as_deref() == Some(name.as_str())).click();
    h.run_steps(3);
    let app = h.state_mut();
    assert!(app.tab_mut().md_preview.is_none());
    assert_eq!(app.tabs.len(), 1, "the document stays open");
    assert_eq!(app.doc_mut().text(), original);
}

/// Where the buttons a screen reader would announce as nothing or as symbols only ("×", "−")
/// are. Glyphs that include a letter, such as "A+", are not caught; check those by name.
fn unnamed_buttons(h: &Harness<'static, App>) -> Vec<String> {
    h.query_all_by(|n| n.role() == egui::accesskit::Role::Button && n.label().is_none_or(|l| !l.chars().any(char::is_alphanumeric)))
        .map(|n| format!("{:?}", n.rect()))
        .collect()
}

#[test]
fn test_icon_buttons_are_named_for_screen_readers() {
    use super::mdpreview::Placement;
    let mut h = harness();
    h.state_mut().doc_mut().syntax = "Markdown".into();
    h.state_mut().pending.push(C::PreviewMarkdown);
    h.run_steps(3);
    h.state_mut().tab_mut().md_preview.as_mut().unwrap().set_placement(Placement::Right);
    h.state_mut().pending.push(C::Find);
    h.run_steps(3);
    h.state_mut().fif.show_results = true;
    h.run_steps(3);
    // Toolbar icons, the find bar, the docked preview toolbar and the Find in Files results.
    let find = super::commands::plain_label(&h.state_mut().cmd_text(C::Find));
    assert!(h.query_all_by(|n| n.role() == egui::accesskit::Role::Button && n.label().as_deref() == Some(find.as_str())).count() >= 1, "toolbar icon {find}");
    for zoom in ["縮小字型", "放大字型"] {
        assert_eq!(h.query_all_by(|n| n.role() == egui::accesskit::Role::Button && n.label().as_deref() == Some(zoom)).count(), 1, "{zoom}");
    }
    assert!(h.query_all_by(|n| n.role() == egui::accesskit::Role::Button && n.label().is_some_and(|l| l == "A−" || l == "A+")).next().is_none());
    assert_eq!(unnamed_buttons(&h), Vec::<String>::new());
    h.state_mut().find.visible = false;
    h.state_mut().pending.push(C::GoToLine);
    h.run_steps(3);
    assert_eq!(unnamed_buttons(&h), Vec::<String>::new(), "go-to bar");
    h.state_mut().goto.visible = false;
    h.run_steps(2);

    // Options: a shortcut's remove button on the Keys page and a word's on the spelling page.
    h.state_mut().settings.personal_dictionary = vec!["madedit".into()];
    let save = super::commands::plain_label(&h.state_mut().cmd_text(C::Save));
    h.state_mut().pending.push(C::Options);
    h.run_steps(3);
    h.key_press(egui::Key::F1);
    h.run_steps(3);
    dialog(&h).get_by(|n| n.role() == egui::accesskit::Role::Button && n.label().as_deref() == Some(save.as_str())).click();
    h.run_steps(3);
    assert!(h.query_all_by(|n| n.role() == egui::accesskit::Role::Button && n.label().is_some_and(|l| l.starts_with("移除 "))).count() >= 1, "remove {save}'s shortcut");
    assert_eq!(unnamed_buttons(&h), Vec::<String>::new(), "Keys page");
    h.get_by(|n| n.role() == egui::accesskit::Role::Tab && n.label().as_deref() == Some("拼字檢查")).click();
    h.run_steps(3);
    h.get_by(|n| n.role() == egui::accesskit::Role::Button && n.label().as_deref() == Some("移除 madedit")).click();
    h.run_steps(3);
    assert_eq!(unnamed_buttons(&h), Vec::<String>::new(), "spelling page");

    // Find in Files: the button that picks the folder.
    h.state_mut().dialog = None;
    h.state_mut().pending.push(C::FindInFiles);
    h.run_steps(3);
    assert_eq!(dialog(&h).query_all_by(|n| n.role() == egui::accesskit::Role::Button && n.label().as_deref() == Some("選擇資料夾")).count(), 1);
    assert_eq!(unnamed_buttons(&h), Vec::<String>::new(), "Find in Files");
}

#[test]
fn test_print_preview_toolbar_is_translated_and_named() {
    use crate::core::settings::UiLanguage;
    let mut h = harness();
    h.state_mut().doc_mut().editor.set_text("one\ntwo\n");
    h.state_mut().doc_mut().version += 1;
    h.state_mut().pending.push(C::PrintPreview);
    h.run_steps(4);
    assert!(h.state_mut().preview.is_some());
    let button = |h: &Harness<'static, App>, label: &str| h.query_all_by(|n| n.role() == egui::accesskit::Role::Button && n.label().as_deref() == Some(label)).count();
    // The first / last page buttons are named instead of read as "⏮" / "⏭".
    assert_eq!(button(&h, "第一頁"), 1);
    assert_eq!(button(&h, "最後一頁"), 1);
    assert!(h.query_by_label("第 1 / 1 頁").is_some());
    assert_eq!(unnamed_buttons(&h), Vec::<String>::new());
    // Every toolbar text follows the UI language.
    h.state_mut().settings.ui_language = UiLanguage::En;
    h.run_steps(3);
    for label in ["Print...", "First page", "◀ Previous", "Next ▶", "Last page"] {
        assert_eq!(button(&h, label), 1, "{label}");
    }
    assert!(button(&h, "Close") >= 1);
    assert!(h.query_by_label("Page 1 of 1").is_some());
    assert!(h.query_by_label("Zoom:").is_some());
    for zh in ["列印...", "第一頁", "上一頁", "下一頁", "最後一頁", "第 1 / 1 頁", "縮放：", "關閉"] {
        assert!(h.query_all_by(|n| n.label().is_some_and(|l| l.contains(zh)) || n.value().is_some_and(|v| v.contains(zh))).next().is_none(), "{zh} left untranslated");
    }
}

/// A temporary folder removed when dropped, so a failing test does not leave it behind.
struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Recent Files open with two existing files, alpha.txt then beta.txt.
fn recent_files_fixture(name: &str) -> (TempDir, Harness<'static, App>, std::path::PathBuf) {
    let dir = TempDir(std::env::temp_dir().join(format!("madedit-{name}-{}", std::process::id())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let (a, b) = (dir.0.join("alpha.txt"), dir.0.join("beta.txt"));
    std::fs::write(&a, "alpha").unwrap();
    std::fs::write(&b, "beta").unwrap();
    let mut h = harness();
    h.state_mut().settings.recent_files = vec![a, b.clone()];
    h.state_mut().pending.push(C::RecentFilesList);
    h.run_steps(3);
    (dir, h, b)
}

/// The active document's file name; compared by name because the temporary folder can be a
/// symlink (/var → /private/var on macOS).
fn open_file_name(h: &mut Harness<'static, App>) -> Option<std::ffi::OsString> {
    h.state_mut().doc().path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_os_string())
}

#[test]
fn test_recent_files_enter_opens_the_first_match_from_the_filter() {
    let (_dir, mut h, beta) = recent_files_fixture("recent");
    type_text(&mut h, "beta");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(open_file_name(&mut h).as_deref(), beta.file_name());
    assert!(h.state_mut().dialog.is_none());
}

#[test]
fn test_recent_files_enter_on_a_row_opens_that_row() {
    let (_dir, mut h, beta) = recent_files_fixture("recent-row");
    // From the filter, Tab reaches the first row and then the second.
    for _ in 0..2 {
        h.key_press(Key::Tab);
        h.run_steps(2);
    }
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(open_file_name(&mut h).as_deref(), beta.file_name());
}

#[test]
fn test_enter_on_a_missing_recent_file_asks_to_create_it() {
    let mut h = harness();
    h.state_mut().settings.recent_files = vec![std::path::PathBuf::from("/nonexistent-madedit-dir/missing.txt")];
    h.state_mut().pending.push(C::RecentFilesList);
    h.run_steps(3);
    type_text(&mut h, "missing");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(matches!(h.state_mut().confirm, Some(Confirm::CreateNew { .. })), "the same Enter must not dismiss the prompt");
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(h.state_mut().confirm.is_none());
}

#[test]
fn test_message_ignores_the_key_pressed_in_the_frame_it_opens() {
    let mut h = harness();
    // The Enter that led to an error (opening a file, say) arrives in the message's first frame.
    h.state_mut().confirm = Some(Confirm::Message { title: "t".into(), text: "x".into() });
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(h.state_mut().confirm.is_some(), "dismissed before it was drawn");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(h.state_mut().confirm.is_none());
}

#[test]
fn test_dialog_handed_over_counts_as_opened() {
    let mut h = harness();
    h.state_mut().pending.push(C::About);
    h.run_steps(3);
    let about = h.state_mut().dialog.as_ref().map(std::mem::discriminant);
    h.get_by(|n| n.role() == egui::accesskit::Role::Button && n.label().as_deref() == Some("鍵盤快捷鍵")).click();
    // Frame by frame up to the hand-over: the frame that made it drew About, which is what
    // must be recorded, so Shortcuts counts as opened on its first frame.
    for _ in 0..5 {
        h.run_steps(1);
        if matches!(h.state_mut().dialog, Some(Dialog::Shortcuts { .. })) {
            break;
        }
    }
    assert!(matches!(h.state_mut().dialog, Some(Dialog::Shortcuts { .. })));
    assert_eq!(h.state_mut().shown_dialog, about);
    h.run_steps(1);
    let app = h.state_mut();
    assert_eq!(app.shown_dialog, app.dialog.as_ref().map(std::mem::discriminant));
}

#[test]
fn test_recent_files_filter_takes_focus_after_another_dialog_replaced_it() {
    let mut h = harness();
    h.state_mut().settings.recent_files = vec![std::path::PathBuf::from("/tmp/x-alpha.txt")];
    h.state_mut().pending.push(C::RecentFilesList);
    h.run_steps(3);
    h.state_mut().pending.push(C::WordCount);
    h.run_steps(3);
    h.state_mut().dialog = None;
    h.run_steps(2);
    h.state_mut().pending.push(C::RecentFilesList);
    h.run_steps(3);
    type_text(&mut h, "zzz");
    assert!(matches!(&h.state_mut().dialog, Some(Dialog::RecentFiles { filter }) if filter == "zzz"));
}
/// 含全形字的超長行水平捲動：視窗起點的位元組格線會落在多位元組字元中間，不得 panic
#[test]
fn test_cjk_long_line_horizontal_scroll() {
    let mut h = harness();
    // 一長串全形字（字串）：視窗起點會落在多位元組字元中間
    let line = format!("let s = \"{}\";", "中文內容測試".repeat(4_000));
    {
        let doc = h.state_mut().doc_mut();
        doc.syntax = "Rust".into();
        doc.syntax_cache.clear();
    }
    h.event(Event::Paste(line.clone()));
    h.run_steps(3);
    let chars = h.state().doc().editor.line_char_count(0);
    assert_eq!(chars, line.chars().count());
    // 逐段把游標移到行內各處：畫面捲動會推進可見視窗，沿途每個視窗都要能切片與解析
    for step in 0..=32 {
        let col = chars * step / 32;
        h.state_mut().doc_mut().editor.set_caret(0, col);
        h.state_mut().tab_mut().view.ensure_visible = true;
        h.run_steps(1);
        assert_eq!(h.state().doc().editor.selection.head.col, col);
    }
    // 回到行首：視窗反向移動也不得 panic
    h.state_mut().doc_mut().editor.set_caret(0, 0);
    h.state_mut().tab_mut().view.ensure_visible = true;
    h.run_steps(2);
    assert_eq!(h.state().doc().editor.lines.len(), 1, "水平捲動不會產生新行");
}

/// 滑鼠拖曳選取：選取範圍必須從按下的位置開始，往回拖曳時第一個字元不得被跳過
#[test]
fn test_mouse_drag_selects_from_press_position() {
    use eframe::egui;
    let mut h = harness();
    h.state_mut().settings.fixed_width = true;
    type_text(&mut h, "abcdefghij");
    h.run_steps(2);
    let (rect, cell_w) = {
        let s = h.state_mut();
        let rect = s.tab_mut().view.text_rect;
        (rect, s.widths.cell_w)
    };
    let y = rect.top() + 4.0;
    let x = |col: f32| rect.left() + col * cell_w;

    // 往後拖曳：按在 col 2，拖到 col 5，應選到 "cde"
    h.drag_at(egui::pos2(x(2.0), y));
    h.run_steps(1);
    h.hover_at(egui::pos2(x(3.0), y));
    h.run_steps(1);
    h.hover_at(egui::pos2(x(5.0), y));
    h.run_steps(1);
    h.drop_at(egui::pos2(x(5.0), y));
    h.run_steps(1);
    assert_eq!(h.state().doc().editor.selected_text(), "cde", "往後拖曳的選取範圍");

    // 往前（往回）拖曳：按在 col 5，拖到 col 2，應選到 "cde"
    h.drag_at(egui::pos2(x(5.0), y));
    h.run_steps(1);
    h.hover_at(egui::pos2(x(4.0), y));
    h.run_steps(1);
    h.hover_at(egui::pos2(x(2.0), y));
    h.run_steps(1);
    h.drop_at(egui::pos2(x(2.0), y));
    h.run_steps(1);
    assert_eq!(h.state().doc().editor.selected_text(), "cde", "往前拖曳的選取範圍");

    // 手快速甩動：一次就超過臨界距離，起點仍要是按下的位置
    h.drag_at(egui::pos2(x(1.0), y));
    h.run_steps(1);
    h.hover_at(egui::pos2(x(8.0), y));
    h.run_steps(1);
    h.drop_at(egui::pos2(x(8.0), y));
    h.run_steps(1);
    assert_eq!(h.state().doc().editor.selected_text(), "bcdefgh", "快速拖曳的選取範圍");
}

/// 滑鼠跨行往回拖曳：起點的行與欄都要是按下的位置
#[test]
fn test_mouse_backward_drag_across_lines() {
    use eframe::egui;
    let mut h = harness();
    h.state_mut().settings.fixed_width = true;
    type_text(&mut h, "abcdefghij");
    h.key_press(Key::Enter);
    h.run_steps(1);
    type_text(&mut h, "klmnopqrst");
    h.run_steps(2);
    let (rect, cell_w, row_h) = {
        let s = h.state_mut();
        let rect = s.tab_mut().view.text_rect;
        let row_h = (s.widths.text_h * s.settings.line_spacing.clamp(100, 250) as f32 / 100.0).round().max(1.0);
        (rect, s.widths.cell_w, row_h)
    };
    let y = |row: f32| rect.top() + row * row_h + 4.0;
    let x = |col: f32| rect.left() + col * cell_w;

    // 按在第二行 col 3，往回拖到第一行 col 1，應選到第一行 col 1 起至第二行 col 3
    h.drag_at(egui::pos2(x(3.0), y(1.0)));
    h.run_steps(1);
    h.hover_at(egui::pos2(x(2.0), y(1.0)));
    h.run_steps(1);
    h.hover_at(egui::pos2(x(1.0), y(0.0)));
    h.run_steps(1);
    h.drop_at(egui::pos2(x(1.0), y(0.0)));
    h.run_steps(1);
    assert_eq!(h.state().doc().editor.selected_text(), "bcdefghij\nklm", "跨行往回拖曳的選取範圍");
}

/// 有結果時按下「清除」：同一幀仍在繪製舊結果，動作必須延後到繪製結束，不能越界
#[test]
fn test_find_in_files_clear_survives_the_same_frame_drawing() {
    use egui::accesskit::Role;
    let dir = std::env::temp_dir().join(format!("madedit_ui_fif_clear_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("clear.txt");
    std::fs::write(&file, "needle one\nplain\nneedle two\n").unwrap();
    let mut h = Harness::builder().allow_missing_glyphs().with_size([1000.0, 700.0]).build_eframe(|cc| App::new(cc, vec![file.clone()], None));
    h.state_mut().settings.find_in_files.opened_files_only = true;
    h.run_steps(3);
    h.state_mut().start_find_in_files("needle".into(), None);
    for _ in 0..200 {
        h.run_steps(1);
        if !h.state_mut().fif.running {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    h.run_steps(2);
    assert_eq!(h.state_mut().fif.results.len(), 2);
    h.get_by(|n| n.role() == Role::Button && n.label().as_deref() == Some("清除")).click();
    h.run_steps(2);
    let s = h.state_mut();
    assert!(s.fif.results.is_empty(), "results must be cleared");
    assert!(s.fif.notes.is_empty());
    assert_eq!(s.fif.replaced, 0);
    assert_eq!(s.fif.status, super::findfiles::Status::Idle);
    // 清除後可以再次搜尋
    h.state_mut().start_find_in_files("two".into(), None);
    for _ in 0..200 {
        h.run_steps(1);
        if !h.state_mut().fif.running {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    h.run_steps(2);
    assert_eq!(h.state_mut().fif.results.len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 第 150 欄的符合：片段以符合處為中心，列上找得到、點擊後仍選取原始欄位
#[test]
fn test_find_in_files_row_shows_and_opens_matches_beyond_the_excerpt_budget() {
    use egui::accesskit::Role;
    let dir = std::env::temp_dir().join(format!("madedit_ui_fif_deep_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("deep.txt"), format!("{}needle tail\n", "x".repeat(150))).unwrap();

    let mut h = Harness::builder().allow_missing_glyphs().with_size([1000.0, 700.0]).build_eframe(|cc| App::new(cc, vec![], None));
    h.run_steps(3);
    {
        let opts = &mut h.state_mut().settings.find_in_files;
        opts.directory = dir.to_string_lossy().into_owned();
        opts.opened_files_only = false;
        opts.file_filter = "*.txt".into();
        opts.exclude_filter.clear();
    }
    h.state_mut().start_find_in_files("needle".into(), None);
    for _ in 0..200 {
        h.run_steps(1);
        if !h.state_mut().fif.running {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    h.run_steps(2);
    assert_eq!(h.state_mut().fif.results.len(), 1);
    // 片段不是行首前 100 字：名稱含符合前 24 字與符合字串本身（繪製時才加「…」）
    let row = format!("1: {}needle tail", "x".repeat(24));
    assert!(
        h.query_all_by(|n| n.role() == Role::Button && n.label().as_deref() == Some(row.as_str())).count() > 0,
        "missing deep-match row {row}"
    );
    h.get_by(|n| n.role() == Role::Button && n.label().as_deref() == Some(row.as_str())).click();
    h.run_steps(3);
    assert_eq!(h.state_mut().doc().editor.selected_text(), "needle", "點擊結果應選取原始欄位的符合文字");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 查詢與文件不變時，移動游標或 Find Next 不得重啟全文計數，只更新目前序號
#[test]
fn test_find_bar_navigation_does_not_restart_the_full_text_count() {
    let mut h = harness();
    {
        let s = h.state_mut();
        (s.settings.search_match_case, s.settings.search_whole_word, s.settings.search_regex) = (false, false, false);
        s.doc_mut().editor.set_text("one two one\nONE");
        s.doc_mut().version += 1;
        s.pending.push(C::Find);
    }
    h.run_steps(3);
    type_text(&mut h, "one");
    h.run_steps(2);
    assert!(h.query_by_label("共 3 個符合").is_some());
    let key = h.state_mut().find.live.counts_key;
    let pattern = h.state_mut().find.live.pattern_id;
    // Find Next（F3）只移動目前序號
    h.state_mut().pending.push(C::FindNext);
    h.run_steps(3);
    assert_eq!(h.state_mut().find.live.counts_key, key, "navigation must not restart the full-text count");
    assert_eq!(h.state_mut().find.live.pattern_id, pattern);
    assert!(h.query_by_label("1 / 3").is_some());
    // 一般游標移動也一樣
    h.state_mut().doc_mut().editor.set_caret(1, 1);
    h.run_steps(2);
    assert_eq!(h.state_mut().find.live.counts_key, key);
    assert!(h.query_by_label("共 3 個符合").is_some());
    // 編輯文件才重新計數
    h.state_mut().doc_mut().editor.set_text("one two one two one\nONE");
    h.state_mut().doc_mut().version += 1;
    h.run_steps(2);
    assert_ne!(h.state_mut().find.live.counts_key, key, "an edit must recompute the counts");
    assert!(h.query_by_label("共 4 個符合").is_some());
}
