//! Text, syntax styles, selection, caret, gutter and editor chrome painting.

use super::layout::visible_chars;
use super::{CharWidths, Frame, HorizontalBar, ViewEnv, ViewState};
use crate::core::content_hash;
use crate::core::document::Document;
use crate::core::editor::char_col_to_visual_col;
use crate::core::syntax::{self, HighlightSpan};
use crate::core::textops;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Stroke, StrokeKind, Vec2,
};

fn rgb((r, g, b): (u8, u8, u8)) -> Color32 {
    Color32::from_rgb(r, g, b)
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct GlyphStyle {
    color: Color32,
    bold: bool,
    italic: bool,
}

/// Keep glyph advances unchanged: the caret and wrapping share unstyled widths.
fn paint_syntax_text(
    painter: &egui::Painter,
    pos: Pos2,
    text: String,
    font: FontId,
    style: GlyphStyle,
) {
    if !style.bold && !style.italic {
        painter.text(pos, Align2::LEFT_TOP, text, font, style.color);
        return;
    }
    let format = egui::TextFormat {
        font_id: font,
        color: style.color,
        italics: style.italic,
        ..Default::default()
    };
    let galley = painter.layout_job(egui::text::LayoutJob::simple_format(text, format));
    if style.bold {
        // Synthetic weight avoids loading a second font or changing grid metrics.
        let offset = Vec2::new(0.5 / painter.ctx().pixels_per_point(), 0.0);
        painter.galley(pos + offset, galley.clone(), style.color);
    }
    painter.galley(pos, galley, style.color);
}

pub(super) fn text(
    ui: &egui::Ui,
    doc: &mut Document,
    view: &mut ViewState,
    env: &ViewEnv<'_>,
    frame: &Frame,
    focused: bool,
) -> f32 {
    let ctx = ui.ctx();
    let settings = env.settings;
    let widths: &CharWidths = env.widths;
    let lay = &frame.lay;
    let full = frame.full;
    let text_rect = frame.text_rect;
    let mark_w = frame.mark_w;
    let num_w = frame.num_w;
    let ruler_h = frame.ruler_h;
    let total_rows = view.row_count(&doc.editor);
    let vis = view.visible_rows;
    let colors = syntax::theme_colors(env.theme_name, env.is_dark);
    let bg = rgb(colors.background);
    let fg = rgb(colors.foreground);
    let visuals = ui.visuals().clone();
    let sel_color = colors
        .selection
        .map(|c| rgb(c).gamma_multiply(0.85))
        .unwrap_or(visuals.selection.bg_fill.gamma_multiply(0.6));
    let faint = fg.gamma_multiply(0.35);
    // 主題的目前行底色常與註解色相同（如 base16-ocean.dark），降低不透明度以免文字被吃掉
    let active_line_color = colors
        .line_highlight
        .map(|c| rgb(c).gamma_multiply(0.3))
        .unwrap_or(if env.is_dark {
            Color32::from_white_alpha(14)
        } else {
            Color32::from_black_alpha(12)
        });
    // ── 繪製 ──
    let painter = ui.painter_at(full);
    painter.rect_filled(full, CornerRadius::ZERO, bg);
    let gutter_bg = if env.is_dark {
        bg.gamma_multiply(0.8)
    } else {
        Color32::from_gray(240)
    };
    painter.rect_filled(
        Rect::from_min_max(
            full.min,
            Pos2::new(text_rect.left() - 2.0, text_rect.bottom()),
        ),
        CornerRadius::ZERO,
        gutter_bg,
    );
    let text_painter = painter.with_clip_rect(text_rect.expand2(Vec2::new(0.0, 0.0)));

    let first_row = view.top_row;
    let last_row = (first_row + vis + 1).min(total_rows);
    let first_line = view.row_info(&doc.editor, first_row).0;
    let last_line = if last_row > first_row {
        view.row_info(&doc.editor, last_row - 1).0
    } else {
        first_line
    };
    // 每行水平可見的字元範圍：超長行只解析這段視窗，成本與行長無關
    let mut windows: Vec<(usize, usize, usize)> = Vec::with_capacity(last_line + 1 - first_line);
    for row in first_row..last_row {
        let line = view.row_line(&doc.editor, row);
        let start = view.row_start(&doc.editor, row);
        let layout = view.line_layout(&doc.editor, line, lay.fixed, widths);
        let row_end = if row + 1 < last_row && view.row_line(&doc.editor, row + 1) == line {
            view.row_start(&doc.editor, row + 1)
        } else {
            layout.0.len()
        };
        let (lo, hi) = visible_chars(
            &layout.1,
            start,
            row_end,
            view.scroll_x,
            text_rect.width(),
            lay.rtl,
        );
        match windows.last_mut() {
            Some(w) if w.0 == line => {
                w.1 = w.1.min(lo);
                w.2 = w.2.max(hi);
            }
            _ => windows.push((line, lo, hi)),
        }
    }
    let spans = doc.syntax_cache.highlight(
        &doc.editor.lines,
        first_line,
        last_line + 1 - first_line,
        &doc.syntax,
        env.theme_name,
        env.is_dark,
        &windows,
    );
    if doc.syntax_cache.has_pending() {
        // 分時解析尚未完成：要求下一幀續跑，把其餘行補上顏色
        ctx.request_repaint();
    }
    let ed = &doc.editor;
    let sel = ed.selection;
    let block_bounds = sel.is_block().then(|| sel.visual_bounds(ed));
    let head_row = view.row_of(sel.head);

    // 對應括號
    let braces = if settings.mark_brace_pair {
        let key = (sel.head, doc.version);
        match &view.brace_cache {
            Some((k, v)) if *k == key => *v,
            _ => {
                let v = textops::find_brace_pair(&ed.lines, sel.head);
                view.brace_cache = Some((key, v));
                v
            }
        }
    } else {
        None
    };

    let mut max_w = 0.0f32;
    let control_color = if env.is_dark {
        Color32::from_rgb(90, 110, 120)
    } else {
        Color32::from_rgb(170, 190, 200)
    };
    let eol_color = Color32::from_rgb(80, 170, 80);
    // 可見列依邏輯行分組：每行只建立一次字元與顏色版面，超長行不會逐視覺列重建
    let mut spell_ranges: Vec<(usize, usize)> = Vec::new();
    let mut row_iter = first_row;
    while row_iter < last_row {
        let line = view.row_line(ed, row_iter);
        let layout = view.line_layout(ed, line, lay.fixed, widths);
        let chars = &layout.0;
        let xs = &layout.1;
        let char_count = chars.len();
        // 拼字檢查每行一次（以內容雜湊快取，不複製整行文字當鍵）
        spell_ranges.clear();
        if settings.spell_check
            && let Some(sc) = env.spell
        {
            let hash = content_hash(&ed.lines[line]);
            if let Some(r) = view.spell_cache.get(&hash) {
                spell_ranges = r.clone();
            } else {
                if view.spell_cache.len() > 4000 {
                    view.spell_cache.clear();
                }
                let r = sc.misspelled_ranges(&ed.lines[line]);
                view.spell_cache.insert(hash, r.clone());
                spell_ranges = r;
            }
        }
        // Empty default spans avoid a per-character table for plain text.
        let line_spans: &[HighlightSpan] = spans
            .get(line - first_line)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        let default_style = GlyphStyle {
            color: fg,
            bold: false,
            italic: false,
        };
        let styles: Option<Vec<GlyphStyle>> = if line_spans.len() <= 1
            && line_spans
                .first()
                .is_none_or(|s| s.text.is_empty() && !s.bold && !s.italic)
        {
            None
        } else {
            let mut v = Vec::with_capacity(chars.len());
            for sp in line_spans {
                if sp.skip_chars > 0 {
                    // 超長行只解析可見視窗：視窗之前的字元以前景色顯示
                    v.resize(v.len() + sp.skip_chars, default_style);
                }
                let style = GlyphStyle {
                    color: rgb(sp.color_rgb),
                    bold: sp.bold,
                    italic: sp.italic,
                };
                v.extend(std::iter::repeat_n(style, sp.text.chars().count()));
            }
            v.resize(chars.len(), default_style);
            Some(v)
        };
        let style_at = |i: usize| styles.as_ref().map_or(default_style, |v| v[i]);
        let mut draw_row = |row: usize, start: usize, end: usize| {
            let y = text_rect.top() + (row - first_row) as f32 * lay.row_h;
            let text_y = y + (lay.row_h - widths.text_h) / 2.0;
            let x0 = xs[start];
            let row_w = xs[end] - x0;
            max_w = max_w.max(row_w);
            let sx = |x_rel: f32, w: f32| -> f32 {
                if lay.rtl {
                    text_rect.right() + view.scroll_x - x_rel - w
                } else {
                    text_rect.left() - view.scroll_x + x_rel
                }
            };
            let row_rect = Rect::from_min_size(
                Pos2::new(text_rect.left(), y),
                Vec2::new(text_rect.width(), lay.row_h),
            );

            // 目前行
            if settings.mark_active_line && sel.head.line == line && sel.is_empty() {
                painter.rect_filled(row_rect, CornerRadius::ZERO, active_line_color);
            }

            // 選取範圍
            if let Some(b) = block_bounds {
                if line >= b.min_line && line <= b.max_line {
                    let a = sx(
                        b.min_vcol as f32 * widths.cell_w,
                        (b.max_vcol - b.min_vcol) as f32 * widths.cell_w,
                    );
                    let w = ((b.max_vcol - b.min_vcol) as f32 * widths.cell_w).max(1.5);
                    text_painter.rect_filled(
                        Rect::from_min_size(Pos2::new(a, y), Vec2::new(w, lay.row_h)),
                        CornerRadius::ZERO,
                        sel_color,
                    );
                }
            } else if !sel.is_empty() {
                let (s, e) = (sel.start(), sel.end());
                if line >= s.line && line <= e.line {
                    let a = if line == s.line {
                        s.col.max(start)
                    } else {
                        start
                    };
                    let bcol = if line == e.line { e.col.min(end) } else { end };
                    let last_row_of_line = end == char_count;
                    if a <= bcol && (a < bcol || (line < e.line && last_row_of_line)) {
                        let xa = xs[a] - x0;
                        let mut xb = xs[bcol] - x0;
                        if line < e.line && last_row_of_line && bcol == end {
                            xb += widths.space_w; // 換行字元
                        }
                        let left = sx(xa, xb - xa);
                        text_painter.rect_filled(
                            Rect::from_min_size(Pos2::new(left, y), Vec2::new(xb - xa, lay.row_h)),
                            CornerRadius::ZERO,
                            sel_color,
                        );
                    }
                }
            }

            // 雙擊標示的相同字詞
            if let Some(w) = view.highlight_word.as_deref() {
                let wc: Vec<char> = w.chars().collect();
                if !wc.is_empty() {
                    let mut i = start;
                    while i + wc.len() <= end {
                        if chars[i..i + wc.len()] == wc[..]
                            && (i == 0 || !crate::core::editor::is_word_char(chars[i - 1]))
                            && (i + wc.len() >= chars.len()
                                || !crate::core::editor::is_word_char(chars[i + wc.len()]))
                        {
                            let xa = xs[i] - x0;
                            let xb = xs[i + wc.len()] - x0;
                            text_painter.rect_stroke(
                                Rect::from_min_size(
                                    Pos2::new(sx(xa, xb - xa), y),
                                    Vec2::new(xb - xa, lay.row_h),
                                ),
                                CornerRadius::same(2),
                                Stroke::new(1.0, Color32::from_rgb(230, 160, 40)),
                                StrokeKind::Inside,
                            );
                            i += wc.len();
                        } else {
                            i += 1;
                        }
                    }
                }
            }
            // 對應括號
            if let Some((a, b)) = braces {
                for p in [a, b] {
                    if p.line == line && p.col >= start && p.col < end {
                        let xa = xs[p.col] - x0;
                        let w = xs[p.col + 1] - xs[p.col];
                        text_painter.rect_filled(
                            Rect::from_min_size(Pos2::new(sx(xa, w), y), Vec2::new(w, lay.row_h)),
                            CornerRadius::same(2),
                            Color32::from_rgba_unmultiplied(80, 160, 255, 70),
                        );
                    }
                }
            }

            // 文字：依語法色彩分段；等寬模式下字寬與格網相符的連續字元合併繪製（比例模式逐字繪製）
            let mut run = String::new();
            let mut run_x = 0.0f32;
            let mut run_style = default_style;
            let flush = |run: &mut String, run_x: f32, style: GlyphStyle| {
                if !run.is_empty() {
                    paint_syntax_text(
                        &text_painter,
                        Pos2::new(sx(run_x, 0.0), text_y),
                        std::mem::take(run),
                        lay.font.clone(),
                        style,
                    );
                }
            };
            let view_left = view.scroll_x - widths.cell_w * 4.0;
            let view_right = view.scroll_x + text_rect.width() + widths.cell_w * 4.0;
            for i in start..end {
                let c = chars[i];
                let xa = xs[i] - x0;
                let w = xs[i + 1] - xs[i];
                if xa + w < view_left || xa > view_right {
                    flush(&mut run, run_x, run_style);
                    continue;
                }
                let style = style_at(i);
                if c == '\t' || c == ' ' || c == '\u{3000}' {
                    flush(&mut run, run_x, run_style);
                    let show = if c == '\t' {
                        settings.show_tab
                    } else {
                        settings.show_space
                    };
                    if show {
                        let sym = if c == '\t' { "»" } else { "·" };
                        let gw = widths.get(sym.chars().next().unwrap());
                        let px = sx(xa, w) + if lay.rtl { (w - gw).max(0.0) } else { 0.0 };
                        let px = if c == '\t' {
                            px
                        } else {
                            px + ((w - gw) / 2.0).max(0.0)
                        };
                        text_painter.text(
                            Pos2::new(px, text_y),
                            Align2::LEFT_TOP,
                            sym,
                            lay.font.clone(),
                            control_color,
                        );
                    }
                    continue;
                }
                if c.is_control() {
                    flush(&mut run, run_x, run_style);
                    let label = format!("{:02X}", c as u32);
                    let r = Rect::from_min_size(
                        Pos2::new(sx(xa, w), y + 1.0),
                        Vec2::new(w.max(widths.cell_w), lay.row_h - 2.0),
                    );
                    text_painter.rect_filled(
                        r,
                        CornerRadius::same(2),
                        control_color.gamma_multiply(0.6),
                    );
                    text_painter.text(
                        r.center(),
                        Align2::CENTER_CENTER,
                        label,
                        FontId::monospace(widths.text_h * 0.5),
                        bg,
                    );
                    continue;
                }
                let gw = widths.get(c);
                let grid_ok = (gw - w).abs() < 0.05;
                // Proportional layout draws every glyph at its own x. egui lays a run out with pair
                // kerning, which `xs` (summed advances, shared with the caret, selection and
                // wrapping) does not have, so a long run drifted from them: 11 px over a 360-char
                // base64 line in Noto Sans TC, showing as a gap before the next run.
                if lay.rtl || !grid_ok || !lay.fixed {
                    flush(&mut run, run_x, run_style);
                    // 字寬與格子不同（例如 CJK 後備字型）：在格子內置中
                    let px = sx(xa, w) + ((w - gw) / 2.0).max(0.0);
                    paint_syntax_text(
                        &text_painter,
                        Pos2::new(px, text_y),
                        c.to_string(),
                        lay.font.clone(),
                        style,
                    );
                    continue;
                }
                // 相鄰的 ASCII 符號分開繪製，避免程式字型把 -> => != 等連字成單一圖形
                let ligature_risk = c.is_ascii_punctuation()
                    && run.chars().last().is_some_and(|p| p.is_ascii_punctuation());
                if run.is_empty() || style != run_style || ligature_risk {
                    flush(&mut run, run_x, run_style);
                    run_x = xa;
                    run_style = style;
                }
                run.push(c);
            }
            flush(&mut run, run_x, run_style);

            // 行尾符號
            if settings.show_eol && end == char_count && line + 1 < ed.lines.len() {
                let px = sx(row_w, widths.get('↵'));
                text_painter.text(
                    Pos2::new(px, text_y),
                    Align2::LEFT_TOP,
                    "↵",
                    lay.font.clone(),
                    eol_color,
                );
            }

            // 拼字錯誤底線
            for &(a, b) in &spell_ranges {
                {
                    let (a, b) = (a.max(start), b.min(end));
                    if a < b {
                        let xa = xs[a] - x0;
                        let xb = xs[b] - x0;
                        let left = sx(xa, xb - xa);
                        let base = text_y + widths.text_h - 1.0;
                        let mut pts = Vec::new();
                        let mut x = left;
                        let mut up = true;
                        while x < left + (xb - xa) {
                            pts.push(Pos2::new(x, if up { base - 1.5 } else { base + 0.5 }));
                            x += 2.0;
                            up = !up;
                        }
                        if pts.len() > 1 {
                            text_painter.add(egui::Shape::line(
                                pts,
                                Stroke::new(1.0, Color32::from_rgb(230, 50, 50)),
                            ));
                        }
                    }
                }
            }

            // 行號與書籤（只畫在每行第一個視覺列）
            if start == 0 {
                if settings.show_line_numbers {
                    let num_color = if line == sel.head.line { fg } else { faint };
                    painter.text(
                        Pos2::new(full.left() + mark_w + num_w - 6.0, text_y),
                        Align2::RIGHT_TOP,
                        (line + 1).to_string(),
                        lay.font.clone(),
                        num_color,
                    );
                }
                if settings.show_bookmarks && ed.bookmarks.contains(&line) {
                    painter.circle_filled(
                        Pos2::new(full.left() + mark_w / 2.0 + 1.0, y + lay.row_h / 2.0),
                        (lay.row_h * 0.22).min(5.0),
                        Color32::from_rgb(60, 120, 230),
                    );
                }
            }

            // 游標
            if row == head_row && focused {
                let caret_x_rel = if let Some(v) = sel.head_vcol.filter(|_| sel.is_block()) {
                    v as f32 * widths.cell_w
                } else {
                    xs[sel.head.col.min(xs.len() - 1)] - x0
                };
                let mut caret_x = sx(caret_x_rel, 0.0);
                if let Some(pre) = &view.preedit {
                    // 輸入法組字：在游標處以底線標示組字中的文字
                    let w: f32 = pre.chars().map(|c| widths.get(c)).sum();
                    let r = Rect::from_min_size(Pos2::new(caret_x, y), Vec2::new(w, lay.row_h));
                    text_painter.rect_filled(r, CornerRadius::ZERO, bg);
                    text_painter.text(
                        Pos2::new(caret_x, text_y),
                        Align2::LEFT_TOP,
                        pre,
                        lay.font.clone(),
                        fg,
                    );
                    text_painter.hline(r.x_range(), text_y + widths.text_h, Stroke::new(1.0, fg));
                    caret_x += w;
                }
                let time = ctx.input(|i| i.time);
                let blink_on = ((time - view.blink_start) % 1.0) < 0.6;
                let caret_rect = if ed.overwrite_mode && view.preedit.is_none() {
                    let w = xs
                        .get(sel.head.col + 1)
                        .map(|x| x - xs[sel.head.col])
                        .unwrap_or(widths.cell_w);
                    Rect::from_min_size(Pos2::new(caret_x, y + lay.row_h - 3.0), Vec2::new(w, 2.0))
                } else {
                    Rect::from_min_size(Pos2::new(caret_x - 0.5, y), Vec2::new(2.0, lay.row_h))
                };
                if blink_on || view.preedit.is_some() {
                    // 列塊矩形選取時，於每一行畫出游標
                    if let Some(b) = block_bounds.filter(|b| b.min_vcol == b.max_vcol) {
                        for l in b.min_line..=b.max_line {
                            if l >= first_line && l <= last_line {
                                let ry = text_rect.top() + (l - first_row) as f32 * lay.row_h;
                                text_painter.rect_filled(
                                    Rect::from_min_size(
                                        Pos2::new(caret_rect.left(), ry),
                                        caret_rect.size(),
                                    ),
                                    CornerRadius::ZERO,
                                    fg,
                                );
                            }
                        }
                    } else {
                        text_painter.rect_filled(caret_rect, CornerRadius::ZERO, fg);
                    }
                }
                ctx.request_repaint_after(std::time::Duration::from_millis(300));
                // 輸入法候選窗位置
                ctx.output_mut(|o| {
                    o.ime = Some(egui::output::IMEOutput {
                        purpose: egui::IMEPurpose::Normal,
                        rect: text_rect,
                        cursor_rect: Rect::from_min_size(
                            Pos2::new(caret_x, y),
                            Vec2::new(2.0, lay.row_h),
                        ),
                        should_interrupt_composition: false,
                    })
                });
            }
        };
        // 這一行的每個可見視覺列
        while row_iter < last_row && view.row_line(ed, row_iter) == line {
            let row = row_iter;
            row_iter += 1;
            let start = view.row_start(ed, row);
            let end = if row_iter < last_row && view.row_line(ed, row_iter) == line {
                view.row_start(ed, row_iter)
            } else {
                char_count
            };
            draw_row(row, start, end);
        }
    }
    if focused && (head_row < first_row || head_row >= last_row) {
        // 游標不在可見範圍時仍啟用輸入法
        ctx.output_mut(|o| {
            o.ime = Some(egui::output::IMEOutput {
                purpose: egui::IMEPurpose::Normal,
                rect: text_rect,
                cursor_rect: Rect::from_min_size(text_rect.min, Vec2::new(2.0, lay.row_h)),
                should_interrupt_composition: false,
            })
        });
    }

    // 欄位提示線
    if settings.show_col_hint && !lay.rtl {
        let x =
            text_rect.left() - view.scroll_x + settings.col_hint_position as f32 * widths.cell_w;
        if text_rect.x_range().contains(x) {
            text_painter.vline(x, text_rect.y_range(), Stroke::new(1.0, faint));
        }
    }

    // 尺標
    if settings.show_ruler {
        let r = Rect::from_min_max(
            Pos2::new(full.left(), full.top()),
            Pos2::new(text_rect.right(), text_rect.top()),
        );
        painter.rect_filled(r, CornerRadius::ZERO, gutter_bg);
        painter.hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, faint));
        let rp = painter.with_clip_rect(Rect::from_min_max(
            Pos2::new(text_rect.left(), r.top()),
            r.max,
        ));
        let first_col = (view.scroll_x / widths.cell_w).floor() as usize;
        let cols = (text_rect.width() / widths.cell_w).ceil() as usize + 1;
        let small = FontId::proportional((ruler_h * 0.6).max(8.0));
        for col in first_col..first_col + cols {
            let x = if lay.rtl {
                text_rect.right() + view.scroll_x - col as f32 * widths.cell_w
            } else {
                text_rect.left() - view.scroll_x + col as f32 * widths.cell_w
            };
            let h = if col % 10 == 0 {
                ruler_h * 0.5
            } else if col % 5 == 0 {
                ruler_h * 0.3
            } else {
                ruler_h * 0.15
            };
            rp.vline(x, (r.bottom() - h)..=r.bottom(), Stroke::new(1.0, faint));
            if col % 10 == 0 {
                rp.text(
                    Pos2::new(x + 2.0, r.top()),
                    Align2::LEFT_TOP,
                    col.to_string(),
                    small.clone(),
                    faint,
                );
            }
        }
        // 游標欄位標記
        let caret_v = sel
            .head_vcol
            .filter(|_| sel.is_block())
            .unwrap_or_else(|| char_col_to_visual_col(&ed.lines[sel.head.line], sel.head.col));
        let cx = text_rect.left() - view.scroll_x + caret_v as f32 * widths.cell_w;
        if !lay.rtl {
            rp.vline(
                cx,
                r.y_range(),
                Stroke::new(2.0, Color32::from_rgb(220, 80, 60)),
            );
        }
    }
    max_w
}

pub(super) fn scrollbars(
    ui: &egui::Ui,
    frame: &Frame,
    v_thumb: Rect,
    horizontal: Option<&HorizontalBar>,
) {
    let painter = ui.painter_at(frame.full);
    let visuals = ui.visuals();
    let track_color = visuals.extreme_bg_color;
    let thumb_color = visuals.widgets.inactive.bg_fill;
    painter.rect_filled(frame.vbar, CornerRadius::ZERO, track_color);
    painter.rect_filled(
        v_thumb,
        CornerRadius::same(3),
        if frame.vbar_resp.dragged() {
            visuals.widgets.active.bg_fill
        } else {
            thumb_color
        },
    );
    painter.rect_filled(frame.hbar, CornerRadius::ZERO, track_color);
    if let Some(bar) = horizontal {
        painter.rect_filled(bar.thumb, CornerRadius::same(3), thumb_color);
    }
}
