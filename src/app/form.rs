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
}
