//! Building blocks for settings pages (Options, Page Setup): titled cards whose rows put the
//! label at the left and the control at the right edge, so every control lines up in one
//! column whatever the language, with switches and segmented choices instead of checkboxes and
//! radio buttons.

use eframe::egui::{self, Align, Color32, Layout, Response, RichText, Role, Sense, TextStyle, TextWrapMode, Ui, WidgetInfo, WidgetText};

/// Accent colour shared with the dialogs' primary buttons.
pub const ACCENT: Color32 = Color32::from_rgb(37, 99, 235);

/// Height of one row: switches, combo boxes and number fields all fit with room to spare.
const ROW_HEIGHT: f32 = 30.0;

/// An icon-only button (such as × or a toolbar icon) showing `tip` on hover and announced as
/// `name` by screen readers, which would otherwise read the icon character or nothing at all.
pub fn icon_button(ui: &mut Ui, button: egui::Button<'_>, tip: &str, name: &str) -> Response {
    let response = ui.add(button).on_hover_text(tip);
    response.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), name));
    response
}

/// The glyph of a Windows caption button (title-bar minimize / maximize / close).
#[cfg(windows)]
#[derive(Clone, Copy, PartialEq)]
pub enum CaptionIcon {
    Minimize,
    Maximize,
    Restore,
    Close,
}

/// Windows 11 style caption button: a flat full-height hit area with a thin stroke glyph and no
/// frame of its own (the previous `Button`-based ones looked like boxed chips against the bar).
/// Hovering paints the whole tile — Close turns system red with a white glyph, like the native
/// title bar. `name` is the accessible label; `tip` the tooltip.
#[cfg(windows)]
pub fn caption_button(ui: &mut Ui, icon: CaptionIcon, tip: &str, name: &str) -> Response {
    // Tile fills the whole bar height, so the three buttons read as one caption strip.
    let height = ui.available_height().max(ui.spacing().interact_size.y);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(46.0, height), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), name));
    if ui.is_rect_visible(rect) {
        let visuals = ui.visuals();
        let text = visuals.widgets.inactive.fg_stroke.color;
        // The palette's own hovered fill keeps the strip consistent with every other control.
        let hover_fill = visuals.widgets.hovered.weak_bg_fill;
        // Windows' fixed caption red (the system ClearType constant, not a theme colour).
        let close_red = Color32::from_rgb(232, 17, 35);
        let hovered = response.hovered() || response.is_pointer_button_down_on();
        let (fill, glyph) = match (icon, hovered) {
            (CaptionIcon::Close, true) => (close_red, Color32::WHITE),
            (_, true) => (hover_fill, text),
            (_, false) => (Color32::TRANSPARENT, text),
        };
        let painter = ui.painter().clone();
        if fill != Color32::TRANSPARENT {
            painter.rect_filled(rect, 0.0, fill);
        }
        let stroke = egui::Stroke::new(1.0, glyph);
        let c = rect.center();
        match icon {
            CaptionIcon::Minimize => {
                painter.line_segment([c + egui::vec2(-5.0, 0.0), c + egui::vec2(5.0, 0.0)], stroke);
            }
            CaptionIcon::Maximize => {
                painter.rect_stroke(
                    egui::Rect::from_center_size(c, egui::Vec2::splat(10.0)),
                    0.0,
                    stroke,
                    egui::StrokeKind::Inside,
                );
            }
            CaptionIcon::Restore => {
                // Two overlapping squares, the back one hidden behind the bar fill isn't needed
                // on a flat strip: both outlines show, as in the native restore glyph.
                for offset in [egui::vec2(-2.0, 2.0), egui::vec2(2.0, -2.0)] {
                    painter.rect_stroke(
                        egui::Rect::from_center_size(c + offset, egui::Vec2::splat(8.0)),
                        0.0,
                        stroke,
                        egui::StrokeKind::Inside,
                    );
                }
            }
            CaptionIcon::Close => {
                painter.line_segment(
                    [c + egui::vec2(-5.0, -5.0), c + egui::vec2(5.0, 5.0)],
                    stroke,
                );
                painter.line_segment(
                    [c + egui::vec2(5.0, -5.0), c + egui::vec2(-5.0, 5.0)],
                    stroke,
                );
            }
        }
    }
    response.on_hover_text(tip)
}

/// Outlines text fields and other inputs drawn in `ui`, which would otherwise vanish into the
/// light theme's panels and cards, and gives dark fields a softer fill than near-black.
pub fn outline_fields(ui: &mut Ui) {
    let visuals = ui.visuals_mut();
    visuals.widgets.inactive.bg_stroke = visuals.widgets.noninteractive.bg_stroke;
    if visuals.dark_mode {
        visuals.text_edit_bg_color = Some(Color32::from_gray(24));
    }
}

/// A small on/off button such as a search option ("Aa" for match case): accent-filled while
/// on, with `name` as its tooltip and screen reader name.
pub fn toggle_chip(ui: &mut Ui, on: &mut bool, text: &str, name: &str) -> Response {
    let galley = WidgetText::from(text).into_galley(ui, Some(TextWrapMode::Extend), f32::INFINITY, TextStyle::Button);
    let size = egui::vec2((galley.size().x + 12.0).max(26.0), ui.spacing().interact_size.y.max(22.0));
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| WidgetInfo::selected(Role::CheckBox, ui.is_enabled(), *on, name));
    if ui.is_rect_visible(rect) {
        let visuals = ui.visuals();
        let color = if *on {
            ui.painter().rect_filled(rect, 5.0, ACCENT);
            Color32::WHITE
        } else {
            let fill = if response.hovered() { visuals.widgets.hovered.weak_bg_fill } else { Color32::TRANSPARENT };
            ui.painter().rect(rect, 5.0, fill, visuals.widgets.noninteractive.bg_stroke, egui::StrokeKind::Inside);
            visuals.widgets.inactive.fg_stroke.color
        };
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
        focus_ring(ui, &response, rect, 5.0, ACCENT);
    }
    response.on_hover_text(name)
}

/// A filter box across the row with a magnifier in front, for the lists in dialogs.
pub fn search_field(ui: &mut Ui, text: &mut String, hint: &str) -> Response {
    let edit = egui::TextEdit::singleline(text).hint_text(hint.to_owned()).prefix(RichText::new("🔍").weak()).desired_width(f32::INFINITY);
    ui.add(edit)
}

/// A multiline text box of [`scrolling_text_edit`] and the rectangle it occupies.
pub struct ScrollEdit {
    /// The box as drawn, border included.
    pub rect: egui::Rect,
    pub response: Response,
}

/// A multiline text box of `rows` lines for an auto-sized window. The edit grows with its
/// text, which would resize the window as the user types, so it sits in a clipped viewport
/// that scrolls internally and keeps its height. Draw the outline with [`field_border`]:
/// decorating the edit itself breaks as soon as the content scrolls
/// (see `docs/multiline-text-input.md`).
///
/// `return_key` is the combination that inserts a line break, handled by the edit itself so
/// that it moves the caret after the break and scrolls to keep the caret in view. `None`
/// lets no key break the line.
///
/// The box paints its own caret: as tall as the text row and thin, where the edit's is a row
/// plus a margin tall and two points wide (see [`paint_caret`]).
pub fn scrolling_text_edit(ui: &mut Ui, id_salt: &'static str, id: egui::Id, text: &mut String, hint: &str, rows: usize, return_key: Option<egui::KeyboardShortcut>) -> ScrollEdit {
    let pad = egui::vec2(6.0, 4.0);
    const EXTRA_HEIGHT: f32 = 16.0;
    let height = ui.text_style_height(&TextStyle::Body) * rows as f32 + 2.0 * pad.y + EXTRA_HEIGHT;
    let rect = egui::Rect::from_min_size(egui::pos2(ui.max_rect().left(), ui.cursor().top()), egui::vec2(ui.max_rect().width(), height));
    // The edit draws neither a background nor a frame of its own.
    ui.painter().rect_filled(rect, 6.0, ui.visuals().extreme_bg_color);
    // Reserve the row: a child UI does not advance the parent cursor, so whatever follows would stack on top.
    ui.allocate_exact_size(rect.size(), Sense::hover());
    let inner = rect.shrink2(pad);
    let mut viewport = ui.new_child(
        egui::UiBuilder::new().id_salt(format!("{id_salt}-viewport")).max_rect(inner).layout(Layout::top_down(Align::LEFT)),
    );
    let caret_color = ui.visuals().text_cursor.stroke.color;
    let out = egui::ScrollArea::vertical()
        .id_salt(format!("{id_salt}-scroll"))
        .auto_shrink([false, false])
        .min_scrolled_height(inner.height())
        .max_height(inner.height())
        .show(&mut viewport, |ui| {
            let edit = egui::TextEdit::multiline(text)
                .id(id)
                .return_key(return_key)
                .desired_width(f32::INFINITY)
                .desired_rows(rows)
                .frame(egui::Frame::NONE)
                .margin(egui::Margin::symmetric(6, 4))
                .hint_text(hint.to_owned());
            let composing = ime_composing(ui, id);
            let out = ui
                .scope(|ui| {
                    // egui 的游標比整行還高上下各 1.5 點、兩點寬，在這個字號下像根粗棒：平時把它
                    // 藏起來，改畫一根與行等高、較細的（見 `paint_caret`）。
                    if !composing {
                        ui.visuals_mut().text_cursor.stroke.color = Color32::TRANSPARENT;
                    }
                    edit.show(ui)
                })
                .inner;
            if !composing {
                paint_caret(ui, id, &out, caret_color);
            }
            out
        })
        .inner;
    ScrollEdit { rect, response: out.response.response }
}

/// Width of the caret that [`scrolling_text_edit`] paints.
const CARET_WIDTH: f32 = 1.5;

/// Whether the text box `id` is in an IME composition, kept from the `Ime` events of the
/// focused box, on the same conditions as the edit itself starts and ends one. egui keeps
/// that state to itself. While composing, the box keeps egui's own caret, which marks the
/// composition's active position.
fn ime_composing(ui: &Ui, id: egui::Id) -> bool {
    let key = id.with("ime-composition");
    if ui.ctx().memory(|m| m.owns_ime_events(id)) {
        let change = ui.input(|i| composition_change(&i.events));
        if let Some(composing) = change {
            ui.ctx().data_mut(|d| d.insert_temp(key, composing));
        }
    }
    ui.ctx().data(|d| d.get_temp::<bool>(key)).unwrap_or(false)
}

/// What a frame's `Ime` events do to a composition, read as the edit itself reads them: a
/// non-empty preedit starts one, an emptied preedit or a commit ends it. `None` when nothing
/// changed; with several events, the last one wins.
fn composition_change(events: &[egui::Event]) -> Option<bool> {
    events.iter().rev().find_map(|e| match e {
        egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => Some(!text.is_empty()),
        // The edit ignores a committed line break, so it ends no composition here either.
        egui::Event::Ime(egui::ImeEvent::Commit(text)) if text != "\n" && text != "\r" => Some(false),
        _ => None,
    })
}

/// When the caret of a text box last moved, for its blink.
#[derive(Clone, Copy)]
struct CaretSince {
    range: Option<egui::text::CCursorRange>,
    since: f64,
}

/// Paints the caret of `out` in place of the edit's own: as tall as the text row and no
/// taller, thin, blinking with the style's rhythm. Skipped while the box has no focus or the
/// viewport is unfocused, exactly like the edit's.
fn paint_caret(ui: &Ui, id: egui::Id, out: &egui::widgets::text_edit::TextEditOutput, color: Color32) {
    if !out.response.has_focus() || !ui.input(|i| i.focused) {
        return;
    }
    let Some(range) = out.cursor_range else { return };
    let now = ui.input(|i| i.time);
    let key = id.with("caret");
    let mut since = now;
    if let Some(prev) = ui.ctx().data(|d| d.get_temp::<CaretSince>(key))
        && prev.range == Some(range)
        && !out.response.changed()
        && !out.response.gained_focus()
    {
        since = prev.since;
    }
    ui.ctx().data_mut(|d| d.insert_temp(key, CaretSince { range: Some(range), since }));
    let cursor = &ui.visuals().text_cursor;
    if cursor.blink {
        let cycle = (cursor.on_duration + cursor.off_duration) as f64;
        if ((now - since) % cycle) as f32 >= cursor.on_duration {
            return;
        }
    }
    // The edit paints its text at `galley_pos`, with the galley's own left edge taken off.
    let row = out.galley.pos_from_cursor(range.primary);
    let x = out.galley_pos.x + row.center().x - out.galley.rect.left();
    let top = egui::pos2(x, out.galley_pos.y + row.top());
    ui.painter().line_segment([top, egui::pos2(x, top.y + row.height())], egui::Stroke::new(CARET_WIDTH, color));
}

/// The outline of a text box drawn by [`scrolling_text_edit`], on its viewport: red while
/// `invalid`, the active stroke while it has focus, the hovered one under the pointer, plain
/// otherwise.
pub fn field_border(ui: &Ui, field: &ScrollEdit, invalid: bool) {
    let visuals = ui.visuals();
    let border = if invalid {
        egui::Stroke::new(1.0, visuals.error_fg_color)
    } else if field.response.has_focus() {
        visuals.widgets.active.bg_stroke
    } else if ui.rect_contains_pointer(field.rect) {
        visuals.widgets.hovered.bg_stroke
    } else {
        visuals.widgets.noninteractive.bg_stroke
    };
    ui.painter().rect_stroke(field.rect, 6.0, border, egui::StrokeKind::Inside);
}

/// A shortcut such as "Ctrl+S" drawn as a key cap.
pub fn keycap(ui: &mut Ui, keys: &str) -> Response {
    let visuals = ui.visuals();
    let frame = egui::Frame::new().fill(visuals.extreme_bg_color).stroke(visuals.widgets.noninteractive.bg_stroke).corner_radius(4.0).inner_margin(egui::Margin::symmetric(6, 1));
    frame.show(ui, |ui| ui.label(RichText::new(keys).monospace())).inner
}

/// The label without its trailing colon; rows separate label and value by position instead.
pub fn bare(label: &str) -> &str {
    label.trim_end().trim_end_matches([':', '：']).trim_end()
}

/// A row's label in one colour whether or not it is clickable (clickable labels would
/// otherwise be drawn brighter than plain ones).
fn row_label(ui: &Ui, label: &str) -> RichText {
    RichText::new(bare(label)).color(ui.visuals().widgets.inactive.fg_stroke.color)
}

/// A rounded card of rows, with an optional heading above it (pass "" for none).
pub fn section(ui: &mut Ui, title: &str, add_rows: impl FnOnce(&mut Section)) {
    if !title.is_empty() {
        ui.label(RichText::new(bare(title)).strong());
        ui.add_space(2.0);
    }
    card(ui, egui::Margin::symmetric(12, 2), |ui| add_rows(&mut Section { ui }));
    ui.add_space(12.0);
}

/// The rounded, faintly filled box behind a group of settings or a list, as wide as the space.
pub fn card<R>(ui: &mut Ui, margin: egui::Margin, add: impl FnOnce(&mut Ui) -> R) -> R {
    let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
    card_with_stroke(ui, margin, stroke, add)
}

/// A card with its own border, for example one that lights up while the editor inside it has
/// keyboard focus.
pub fn card_with_stroke<R>(ui: &mut Ui, margin: egui::Margin, stroke: egui::Stroke, add: impl FnOnce(&mut Ui) -> R) -> R {
    let frame = egui::Frame::new().fill(ui.visuals().faint_bg_color).stroke(stroke).corner_radius(8.0).inner_margin(margin);
    frame
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

pub struct Section<'a> {
    ui: &'a mut Ui,
}

impl Section<'_> {
    /// A row with `label` at the left and the control drawn by `add` at the right edge. A long
    /// label is shortened with "…" (full text on hover) rather than pushing the control away.
    pub fn row<R>(&mut self, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
        let size = egui::vec2(self.ui.available_width(), ROW_HEIGHT);
        let inner = self.ui.allocate_ui_with_layout(size, Layout::right_to_left(Align::Center), |ui| {
            ui.set_min_height(ROW_HEIGHT);
            let r = add(ui);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| ui.add(egui::Label::new(row_label(ui, label)).truncate()));
            r
        });
        inner.inner
    }

    /// A row with a switch; clicking anywhere else on the row flips it too.
    pub fn toggle(&mut self, label: &str, on: &mut bool) -> Response {
        self.toggle_with(label, on, |_| {})
    }

    /// Like `toggle`, with extra controls left of the switch that are enabled only while it is
    /// on (for example the auto-save interval).
    pub fn toggle_with(&mut self, label: &str, on: &mut bool, extra: impl FnOnce(&mut Ui)) -> Response {
        let size = egui::vec2(self.ui.available_width(), ROW_HEIGHT);
        let inner = self.ui.allocate_ui_with_layout(size, Layout::right_to_left(Align::Center), |ui| {
            ui.set_min_height(ROW_HEIGHT);
            // Reserved first so the hover background is painted under the row's widgets.
            let background = ui.painter().add(egui::Shape::Noop);
            // Registered before the row's widgets so they stay on top and keep their own clicks;
            // not focusable, so Tab still stops only at the switch.
            let row = ui.interact(ui.max_rect(), ui.unique_id().with("row"), Sense::CLICK);
            let mut response = switch(ui, on, bare(label));
            ui.add_enabled_ui(*on, extra);
            // Not selectable: a selectable label would take the click meant for the row.
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| ui.add(egui::Label::new(row_label(ui, label)).truncate().selectable(false)));
            if row.clicked() {
                *on = !*on;
                response.mark_changed();
            }
            // The whole row is a click target, so show its extent while the pointer is on it.
            // Widened into the card's side padding so the label does not touch the edge.
            if ui.is_enabled() && ui.rect_contains_pointer(ui.max_rect()) {
                let rect = ui.max_rect().expand2(egui::vec2(6.0, 0.0));
                ui.painter().set(background, egui::Shape::rect_filled(rect, 6.0, ui.visuals().widgets.hovered.weak_bg_fill));
            }
            response
        });
        inner.inner
    }

    /// Dimmed explanatory text under the previous row.
    pub fn note(&mut self, text: &str) {
        self.ui.add(egui::Label::new(RichText::new(text).weak()).wrap());
        self.ui.add_space(4.0);
    }

    /// Free-form content spanning the card's width, such as a row of text fields.
    pub fn body<R>(&mut self, add: impl FnOnce(&mut Ui) -> R) -> R {
        self.ui.add_space(4.0);
        let r = add(self.ui);
        self.ui.add_space(4.0);
        r
    }
}

/// An on/off switch, the modern stand-in for a checkbox in settings rows.
pub fn switch(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(egui::vec2(36.0, 20.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| WidgetInfo::selected(Role::Switch, ui.is_enabled(), *on, label));
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_responsive(response.id, *on);
        let off = if ui.visuals().dark_mode { Color32::from_gray(85) } else { Color32::from_gray(190) };
        // A disabled Ui already fades everything it paints.
        let mut fill = off.lerp_to_gamma(ACCENT, t);
        if response.hovered() {
            fill = fill.gamma_multiply(1.15);
        }
        let radius = rect.height() / 2.0;
        ui.painter().rect_filled(rect, radius, fill);
        let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
        ui.painter().circle_filled(egui::pos2(x, rect.center().y), radius - 3.0, Color32::WHITE);
        focus_ring(ui, &response, rect, radius, ACCENT);
    }
    response
}

/// Outline for a custom widget that has keyboard focus, so Tab never lands on an invisible stop.
fn focus_ring(ui: &Ui, response: &Response, rect: egui::Rect, radius: f32, color: Color32) {
    if response.has_focus() {
        ui.painter().rect_stroke(rect.expand(2.0), radius + 2.0, egui::Stroke::new(2.0, color), egui::StrokeKind::Outside);
    }
}

/// Mutually exclusive choices drawn as one joined pill, such as the theme or line endings.
/// Each choice reports itself as a radio button to assistive technology.
pub fn segmented<T: PartialEq + Clone>(ui: &mut Ui, value: &mut T, choices: &[(T, &str)]) {
    let visuals = ui.visuals().clone();
    egui::Frame::new().fill(visuals.extreme_bg_color).stroke(visuals.widgets.noninteractive.bg_stroke).corner_radius(7.0).inner_margin(2).show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        // Rows lay controls out right to left; keep the choices in reading order anyway.
        let rtl = ui.layout().prefer_right_to_left();
        ui.horizontal(|ui| {
            let ordered: Vec<_> = if rtl { choices.iter().rev().collect() } else { choices.iter().collect() };
            for (choice, text) in ordered {
                let selected = *value == *choice;
                let galley = WidgetText::from(*text).into_galley(ui, Some(TextWrapMode::Extend), f32::INFINITY, TextStyle::Button);
                let (rect, response) = ui.allocate_exact_size(galley.size() + egui::vec2(20.0, 8.0), Sense::click());
                if response.clicked() {
                    *value = choice.clone();
                }
                response.widget_info(|| WidgetInfo::selected(Role::RadioButton, ui.is_enabled(), selected, *text));
                let color = if selected {
                    ui.painter().rect_filled(rect, 5.0, ACCENT);
                    Color32::WHITE
                } else {
                    if response.hovered() {
                        ui.painter().rect_filled(rect, 5.0, visuals.widgets.hovered.weak_bg_fill);
                    }
                    visuals.widgets.inactive.fg_stroke.color
                };
                ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
                focus_ring(ui, &response, rect, 5.0, ACCENT);
            }
        });
    });
}

/// One icon in an [`icon_choices`] group.
pub struct IconChoice<'a> {
    pub image: egui::Image<'a>,
    /// Tooltip text, normally the command name with its shortcut.
    pub tip: String,
    /// Accessible name without the menu's "(&F)" mnemonic.
    pub name: String,
    pub selected: bool,
}

/// Mutually exclusive icon buttons drawn as one joined group, for a toolbar choice such as the
/// text, column and hex edit modes: an outlined strip holds every choice and the current one is
/// filled and outlined in the accent colour, so the mode in use is visible at a glance and the
/// group reads as "pick one of these three" instead of three unrelated buttons. Each choice is
/// announced as a radio button rather than a toggle. Returns the index of the picked choice.
pub fn icon_choices(ui: &mut Ui, choices: &[IconChoice<'_>]) -> Option<usize> {
    let visuals = ui.visuals().clone();
    let icon = egui::vec2(16.0, 16.0);
    let size = icon + ui.spacing().button_padding * 2.0;
    // Rows lay controls out right to left; keep the choices in reading order anyway.
    let rtl = ui.layout().prefer_right_to_left();
    let mut picked = None;
    egui::Frame::new()
        .fill(visuals.widgets.inactive.weak_bg_fill)
        .stroke(visuals.widgets.inactive.bg_stroke)
        .corner_radius(6.0)
        // A one-pixel vertical margin keeps the strip inside the toolbar row instead of making the
        // whole toolbar taller than the plain icon buttons next to it.
        .inner_margin(egui::Margin::symmetric(2, 1))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            ui.horizontal(|ui| {
                let order: Vec<usize> = if rtl { (0..choices.len()).rev().collect() } else { (0..choices.len()).collect() };
                for index in order {
                    let choice = &choices[index];
                    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                    // Space / Enter on a focused click-sensing widget counts as a click (egui).
                    if response.clicked() {
                        picked = Some(index);
                    }
                    response.widget_info(|| WidgetInfo::selected(Role::RadioButton, ui.is_enabled(), choice.selected, &choice.name));
                    let response = response.on_hover_text(choice.tip.as_str());
                    if ui.is_rect_visible(rect) {
                        let radius = 4.0;
                        if choice.selected {
                            // A soft accent fill keeps a colourful icon legible; the stroke marks
                            // the current choice even when the pointer is elsewhere.
                            let tint = if visuals.dark_mode { 0.45 } else { 0.22 };
                            ui.painter().rect_filled(rect, radius, ACCENT.gamma_multiply(tint));
                            ui.painter().rect_stroke(rect, radius, egui::Stroke::new(1.5, ACCENT), egui::StrokeKind::Inside);
                        } else if response.hovered() {
                            ui.painter().rect_filled(rect, radius, visuals.widgets.hovered.weak_bg_fill);
                        }
                        choice.image.paint_at(ui, egui::Rect::from_center_size(rect.center(), icon));
                        focus_ring(ui, &response, rect, radius, ACCENT);
                    }
                }
            });
        });
    picked
}

/// A page entry in a settings sidebar.
pub fn nav_item(ui: &mut Ui, selected: bool, text: &str) -> Response {
    list_row(ui, Role::Tab, selected, text, "")
}

/// A full-width selectable row: `text` at the left, `detail` (such as shortcut keys) dimmed at
/// the right. Both are shortened with "…" when the row is narrow.
pub fn list_row(ui: &mut Ui, role: Role, selected: bool, text: &str, detail: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), Sense::click());
    response.widget_info(|| WidgetInfo::selected(role, ui.is_enabled(), selected, text));
    if ui.is_rect_visible(rect) {
        let visuals = ui.visuals();
        if selected {
            ui.painter().rect_filled(rect, 6.0, ACCENT);
        } else if response.hovered() {
            ui.painter().rect_filled(rect, 6.0, visuals.widgets.hovered.weak_bg_fill);
        }
        let color = if selected { Color32::WHITE } else { visuals.widgets.inactive.fg_stroke.color };
        let inner = rect.shrink2(egui::vec2(10.0, 0.0));
        let mut text_width = inner.width();
        if !detail.is_empty() {
            let detail = WidgetText::from(RichText::new(detail).monospace()).into_galley(ui, Some(TextWrapMode::Truncate), inner.width() * 0.45, TextStyle::Monospace);
            text_width -= detail.size().x + 8.0;
            let dim = if selected { Color32::WHITE.gamma_multiply(0.8) } else { visuals.weak_text_color() };
            ui.painter().galley(egui::pos2(inner.right() - detail.size().x, rect.center().y - detail.size().y / 2.0), detail, dim);
        }
        let galley = WidgetText::from(text).into_galley(ui, Some(TextWrapMode::Truncate), text_width.max(0.0), TextStyle::Body);
        ui.painter().galley(egui::pos2(inner.left(), rect.center().y - galley.size().y / 2.0), galley, color);
        // Inset, as the list's scroll area would clip a ring drawn outside the row; white on
        // the accent fill of the selected row.
        focus_ring(ui, &response, rect.shrink(4.0), 3.0, color);
    }
    response
}

/// Height of one flat tab; the strip reserves [`TAB_STRIP_HEIGHT`] including its top padding.
pub const TAB_HEIGHT: f32 = 28.0;

/// Height of a tab strip: the tab plus the 2 px gap above it.
pub const TAB_STRIP_HEIGHT: f32 = TAB_HEIGHT + 2.0;

/// Horizontal padding inside a tab, on each side of its label.
const TAB_PAD: f32 = 12.0;
const TAB_MIN_WIDTH: f32 = 72.0;
const TAB_MAX_TEXT: f32 = 220.0;
const TAB_CLOSE_SIZE: f32 = 16.0;
const TAB_TOP_RADIUS: u8 = 6;

/// Background of a tab strip. Dark mode unifies it with the panes (the selected tab still reads
/// as part of the content via its outline and the strip's baseline gap); light mode keeps a
/// slightly darker strip (derived from the panel fill) so the selected tab stands out.
pub fn tab_strip_bg(ui: &Ui) -> Color32 {
    let fill = ui.visuals().panel_fill;
    if ui.visuals().dark_mode { fill } else { fill.gamma_multiply(0.95) }
}

/// A flat tab attached to the pane below it, in the style of Windows 11 Explorer / UltraEdit:
/// rounded top corners only, no bottom border, and the selected tab is filled with the pane's
/// own background so it merges with the content.
pub struct FlatTab {
    pub rect: egui::Rect,
    /// The clickable body of the tab (its close button is separate).
    pub body: Response,
    /// The close button, when `close_name` is given.
    pub close: Option<Response>,
}

/// Draws one flat tab. `id_salt` must be unique among sibling tabs; `content_bg` is the fill of
/// the pane below (the selected tab uses it); `close_name` both enables the close button and
/// names it for screen readers.
pub fn flat_tab(
    ui: &mut Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    label: &str,
    selected: bool,
    content_bg: Color32,
    close_name: Option<&str>,
) -> FlatTab {
    let visuals = ui.visuals().clone();
    let font = TextStyle::Button.resolve(ui.style());
    let color = if selected { visuals.strong_text_color() } else { visuals.text_color() };
    let measured = ui.painter().layout_no_wrap(label.to_string(), font.clone(), color);
    let close_space = if close_name.is_some() { TAB_CLOSE_SIZE + 6.0 } else { 0.0 };
    let width = (measured.size().x.min(TAB_MAX_TEXT) + TAB_PAD * 2.0 + close_space).max(TAB_MIN_WIDTH);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, TAB_HEIGHT), Sense::hover());
    let close_rect = close_name.map(|_| {
        egui::Rect::from_center_size(
            egui::pos2(rect.right() - TAB_CLOSE_SIZE / 2.0 - 6.0, rect.center().y),
            egui::vec2(TAB_CLOSE_SIZE, TAB_CLOSE_SIZE),
        )
    });
    let body_rect = close_rect.map_or(rect, |close| egui::Rect::from_min_max(rect.min, egui::pos2(close.left() - 2.0, rect.bottom())));
    let tab_id = ui.scope_id().with(&id_salt);
    // click_and_drag：編輯器分頁列用 drag 事件實現拖動排序，一般點擊、雙擊行為不變
    let body = ui.interact(body_rect, tab_id, Sense::click_and_drag());
    body.widget_info(|| WidgetInfo::selected(Role::Tab, ui.is_enabled(), selected, label));
    let close = close_rect.map(|close| {
        let response = ui.interact(close, tab_id.with("close"), Sense::click());
        if let Some(name) = close_name {
            response.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), name));
        }
        response
    });

    let radius = egui::CornerRadius { nw: TAB_TOP_RADIUS, ne: TAB_TOP_RADIUS, sw: 0, se: 0 };
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, radius, content_bg);
        painter.rect_stroke(rect, radius, visuals.widgets.noninteractive.bg_stroke, egui::StrokeKind::Inside);
        // Erase the bottom edge of the outline, so the tab continues into the pane.
        painter.rect_filled(egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - 1.5), rect.max), 0, content_bg);
    } else if body.hovered() || body.has_focus() {
        painter.rect_filled(rect, radius, visuals.widgets.hovered.weak_bg_fill);
    }

    let text_right = close_rect.map_or(rect.right() - TAB_PAD, |close| close.left() - 2.0);
    let text_width = (text_right - rect.left() - TAB_PAD).max(0.0);
    let mut job = egui::text::LayoutJob { wrap: egui::text::TextWrapping::truncate_at_width(text_width), ..Default::default() };
    job.append(label, 0.0, egui::TextFormat::simple(font, color));
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(egui::pos2(rect.left() + TAB_PAD, rect.center().y - galley.size().y / 2.0), galley, color);

    if let Some(close) = &close {
        let color = if close.hovered() {
            visuals.strong_text_color()
        } else if selected {
            color.gamma_multiply(0.7)
        } else {
            visuals.weak_text_color()
        };
        if close.hovered() {
            ui.painter().rect_filled(close.rect, 4.0, visuals.widgets.hovered.weak_bg_fill);
        }
        ui.painter().text(close.rect.center(), egui::Align2::CENTER_CENTER, "×", egui::FontId::proportional(14.0), color);
    }
    FlatTab { rect, body, close }
}

/// Draws the thin line under a tab strip, leaving a gap under the selected tab so it merges
/// with the pane. `strip` is the strip's full-width rect; `active` is that tab's rect.
pub fn tab_strip_baseline(ui: &Ui, strip: egui::Rect, active: Option<egui::Rect>) {
    let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
    let y = strip.bottom() - 0.5;
    let painter = ui.painter();
    match active.map(|active| active.intersect(strip)).filter(|active| active.width() > 0.0) {
        Some(active) => {
            if active.left() > strip.left() {
                painter.hline(strip.left()..=active.left(), y, stroke);
            }
            if active.right() < strip.right() {
                painter.hline(active.right()..=strip.right(), y, stroke);
            }
        }
        None => {
            painter.hline(strip.x_range(), y, stroke);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::bare;

    #[test]
    fn test_bare_drops_only_the_trailing_colon() {
        assert_eq!(bare("預設編碼："), "預設編碼");
        assert_eq!(bare("Default encoding: "), "Default encoding");
        assert_eq!(bare("行距（%）："), "行距（%）");
        assert_eq!(bare("Tab"), "Tab");
    }

    #[test]
    fn test_ime_events_start_and_end_a_composition_like_the_edit() {
        use super::composition_change;
        use egui::{Event, ImeEvent};
        let preedit = |text: &str| Event::Ime(ImeEvent::Preedit { text: text.into(), active_range_chars: None });
        assert_eq!(composition_change(&[]), None);
        assert_eq!(composition_change(&[preedit("ni")]), Some(true));
        assert_eq!(composition_change(&[preedit("")]), Some(false), "an emptied preedit dismisses the IME");
        assert_eq!(composition_change(&[Event::Ime(ImeEvent::Commit("你".into()))]), Some(false));
        assert_eq!(composition_change(&[Event::Ime(ImeEvent::Commit("\n".into()))]), None, "a committed line break is ignored");
        assert_eq!(composition_change(&[Event::Text("x".into())]), None);
        assert_eq!(composition_change(&[preedit("ni"), Event::Ime(ImeEvent::Commit("你".into()))]), Some(false), "the last event wins");
    }
}
