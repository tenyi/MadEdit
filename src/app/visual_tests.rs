//! 離屏繪製截圖（供無法開啟實體視窗的環境檢查畫面）
//!
//! 預設略過，執行方式：
//! `MADEDIT_SHOT_OUT=out.png MADEDIT_SHOT_FILE=檔案 MADEDIT_SHOT_CMDS=menuToggleTheme,menuHexMode cargo test shot -- --ignored`
//!
//! `MADEDIT_SHOT_CLICK` / `MADEDIT_SHOT_RCLICK` click (left / right) widgets by their accessible
//! name before drawing, e.g. `MADEDIT_SHOT_RCLICK=README.md` for a tab's context menu;
//! `MADEDIT_SHOT_HOVER` hovers one so its tooltip is drawn.
//!
//! The window starts in the dark theme (it follows the system); `menuToggleTheme` switches to
//! the light one.

use egui_kittest::kittest::Queryable;

use super::commands::C;
use super::App;

/// Keep the app's real drawing code, but eliminate installed-font and locale differences.
fn regression_harness(markdown: bool) -> egui_kittest::Harness<'static, App> {
    use crate::core::settings::{ThemeChoice, UiLanguage};
    use eframe::egui::{FontData, FontDefinitions, FontFamily};
    let mut h = egui_kittest::Harness::builder()
        .with_size([1000.0, 700.0])
        .with_pixels_per_point(1.0)
        .with_theme(eframe::egui::Theme::Light)
        .wgpu()
        .build_eframe(|cc| {
            let mut app = App::new(cc, Vec::new(), None);
            app.settings.ui_language = UiLanguage::En;
            app.settings.theme = ThemeChoice::Light;
            app.apply_theme(&cc.egui_ctx);
            let mut defs = FontDefinitions::default();
            defs.font_data.insert("test-cjk".into(), std::sync::Arc::new(FontData::from_static(
                include_bytes!("../../tests/fixtures/MadEditTestSans.otf")
            )));
            defs.font_data.insert("test-emoji".into(), std::sync::Arc::new(FontData::from_static(
                include_bytes!("../../tests/fixtures/MadEditTestEmoji.ttf")
            )));
            defs.font_data.insert("test-icons".into(), std::sync::Arc::new(FontData::from_static(
                include_bytes!("../../tests/fixtures/emoji-icon-font.ttf")
            )));
            for chain in defs.families.values_mut() {
                chain.insert(0, "test-cjk".into());
                chain.push("test-emoji".into());
                chain.push("test-icons".into());
            }
            let chain = defs.families[&FontFamily::Monospace].clone();
            defs.families.insert(FontFamily::Name(super::fonts::EDITOR_FAMILY.into()), chain);
            cc.egui_ctx.set_fonts(defs);
            app.fonts.editor_family.clear();
            let doc = app.doc_mut();
            doc.path = Some(if markdown { "visual-fixture.md" } else { "visual-fixture.txt" }.into());
            doc.replace_all_text(if markdown {
                "# Preview\n\n中文 test\n\n- first item\n- second item\n\n```rust\nlet value = 42;\n```\n"
            } else {
                "中文 test\nfirst\tcolumn\nsecond column\n"
            });
            doc.syntax = if markdown { "Markdown" } else { "Plain Text" }.into();
            app
        });
    h.run_steps(5);
    h
}

fn diff_pixels(
    expected: &image::RgbaImage,
    actual: &image::RgbaImage,
) -> (usize, image::RgbaImage) {
    assert_eq!(
        expected.dimensions(),
        actual.dimensions(),
        "snapshot dimensions changed"
    );
    let mut count = 0;
    let diff = image::RgbaImage::from_fn(actual.width(), actual.height(), |x, y| {
        let differs = expected
            .get_pixel(x, y)
            .0
            .iter()
            .zip(actual.get_pixel(x, y).0)
            .any(|(&a, b)| a.abs_diff(b) > 2);
        if differs {
            count += 1;
            image::Rgba([255, 0, 0, 255])
        } else {
            image::Rgba([0, 0, 0, 255])
        }
    });
    (count, diff)
}

fn check_snapshot(name: &str, actual: image::RgbaImage) {
    use std::path::Path;
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let baseline = root
        .join("tests/visual")
        .join(std::env::consts::OS)
        .join(format!("{name}.png"));
    let artifacts = root.join("target/visual").join(std::env::consts::OS);
    std::fs::create_dir_all(&artifacts).unwrap();
    actual
        .save(artifacts.join(format!("{name}.actual.png")))
        .unwrap();
    if std::env::var("MADEDIT_UPDATE_VISUAL").as_deref() == Ok("1") {
        std::fs::create_dir_all(baseline.parent().unwrap()).unwrap();
        actual.save(&baseline).unwrap();
        return;
    }
    assert!(
        baseline.is_file(),
        "missing {}; generate explicitly with MADEDIT_UPDATE_VISUAL=1 make test-visual, then review the images",
        baseline.display()
    );
    let expected = image::open(&baseline).unwrap().into_rgba8();
    expected
        .save(artifacts.join(format!("{name}.expected.png")))
        .unwrap();
    let (count, diff) = diff_pixels(&expected, &actual);
    diff.save(artifacts.join(format!("{name}.diff.png")))
        .unwrap();
    assert_eq!(
        count,
        0,
        "{name}: changed pixels; inspect {}",
        artifacts.display()
    );
}

#[test]
fn test_snapshot_diff_detects_changed_pixel() {
    let expected = image::RgbaImage::from_pixel(2, 2, image::Rgba([255; 4]));
    assert_eq!(diff_pixels(&expected, &expected).0, 0);
    let mut actual = expected.clone();
    actual.put_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
    let (count, diff) = diff_pixels(&expected, &actual);
    assert_eq!(count, 1);
    assert_eq!(diff.get_pixel(1, 1).0, [255, 0, 0, 255]);
}

#[test]
#[ignore = "offscreen renderer required; run make test-visual"]
fn visual_regression_scenes() {
    use crate::core::editor::{Selection, TextPosition};
    let mut h = regression_harness(false);
    let ctx = h.ctx.clone();
    check_snapshot("text", h.render().unwrap());
    h.state_mut().run(&ctx, C::ColumnMode);
    h.state_mut().doc_mut().editor.selection =
        Selection::block(TextPosition::new(1, 1), TextPosition::new(2, 5), 1, 5);
    h.run_steps(3);
    check_snapshot("column", h.render().unwrap());
    h.state_mut().run(&ctx, C::HexMode);
    let hex = h.state_mut().doc_mut().hex.as_mut().unwrap();
    hex.move_to(0, false);
    hex.top_row = 0;
    h.run_steps(3);
    check_snapshot("hex", h.render().unwrap());
    let mut h = regression_harness(true);
    let ctx = h.ctx.clone();
    h.state_mut().run(&ctx, C::PreviewMarkdown);
    h.state_mut()
        .tab_mut()
        .md_preview
        .as_mut()
        .unwrap()
        .set_placement(super::mdpreview::Placement::Right);
    h.run_steps(5);
    check_snapshot("markdown", h.render().unwrap());
}

#[test]
#[ignore]
fn shot() {
    let out = std::env::var("MADEDIT_SHOT_OUT").unwrap_or_else(|_| "shot.png".into());
    let files: Vec<std::path::PathBuf> = std::env::var("MADEDIT_SHOT_FILE").ok().into_iter().map(Into::into).collect();
    let cmds = std::env::var("MADEDIT_SHOT_CMDS").unwrap_or_default();
    let mut harness = egui_kittest::Harness::builder()
        .allow_missing_glyphs()
        .with_size([1100.0, 720.0])
        .wgpu()
        .build_eframe(|cc| App::new(cc, files, None));
    harness.run_steps(3);
    for id in cmds.split(',').filter(|s| !s.is_empty()) {
        let c = C::from_id(id).unwrap_or_else(|| panic!("未知命令 {id}"));
        harness.state_mut().pending.push(c);
        harness.run_steps(3);
    }
    if let Ok(text) = std::env::var("MADEDIT_SHOT_TYPE") {
        harness.event(eframe::egui::Event::Text(text));
        harness.run_steps(3);
    }
    // Comma-separated widget labels to click in order (e.g. a menu title, then a submenu),
    // so open menus can be inspected too.
    for label in std::env::var("MADEDIT_SHOT_CLICK").unwrap_or_default().split(',').filter(|s| !s.is_empty()) {
        harness.get_by_label(label).click();
        harness.run_steps(3);
    }
    // Same for a right click, e.g. a tab's own context menu. `MADEDIT_SHOT_CLICK` draws with the
    // left button only, so context menus need their own variable.
    for label in std::env::var("MADEDIT_SHOT_RCLICK").unwrap_or_default().split(',').filter(|s| !s.is_empty()) {
        harness.get_by_label(label).click_secondary();
        harness.run_steps(3);
    }
    // And for hovering, so tooltips (such as a tab's full path) can be inspected too.
    for label in std::env::var("MADEDIT_SHOT_HOVER").unwrap_or_default().split(',').filter(|s| !s.is_empty()) {
        harness.get_by_label(label).hover();
        harness.run_steps(4);
    }
    // MADEDIT_SHOT_STEPS=N：畫圖前多跑 N 幀，觀察會隨時間變化的佈局（例如面板高度回饋）
    if let Some(n) = std::env::var("MADEDIT_SHOT_STEPS").ok().and_then(|s| s.parse::<usize>().ok()) {
        harness.run_steps(n);
    }
    harness.render().expect("繪製失敗").save(&out).expect("無法儲存截圖");
}
