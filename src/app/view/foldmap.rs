//! Display-row mapping for collapsed code.
//!
//! The editor's screen rows are normally document lines (or wrapped visual rows
//! of those lines). Folding removes the rows of the hidden lines, so every
//! row <-> position conversion must go through this map. The map is rebuilt
//! from the active folds on each frame (O(number of folds)); an empty fold list
//! produces no map and the view keeps its direct indexing path.

use crate::core::document::Document;
use crate::core::editor::EditorState;
use crate::core::fold::{FoldMarker, FoldModel};
use crate::core::settings::Settings;

use super::ViewState;

/// One collapsed range expressed in wrap-row coordinates.
#[derive(Debug, Clone, Copy)]
struct FoldSpan {
    /// First hidden wrap row (the row right after the header line's last row).
    start_row: usize,
    /// How many wrap rows this fold hides.
    hidden_rows: usize,
    /// Display row at which the hidden region starts (header rows stay before it).
    display_start: usize,
    /// Display row of the header line's first row.
    header_display: usize,
}

/// Row mapping between the document/wrap row space and what is on screen.
#[derive(Debug, Default)]
pub(crate) struct DisplayMap {
    spans: Vec<FoldSpan>,
    total_rows: usize,
    hidden_rows: usize,
}

impl DisplayMap {
    /// Build the mapping for the active folds.
    ///
    /// `line_span(line)` returns the first wrap row of a document line and how
    /// many wrap rows it occupies; for a line past the end it returns
    /// `(total_rows, 0)`. Returns `None` when nothing is folded.
    pub fn build(folds: &FoldModel, total_rows: usize, line_span: impl Fn(usize) -> (usize, usize)) -> Option<Self> {
        if folds.active().is_empty() {
            return None;
        }
        let mut spans = Vec::with_capacity(folds.active().len());
        let mut hidden_rows = 0usize;
        let mut previous_end = 0usize;
        for range in folds.active() {
            let (header_row, header_rows) = line_span(range.header);
            let start_row = header_row + header_rows;
            let (after_row, _) = line_span(range.end + 1);
            if after_row <= start_row || start_row < previous_end {
                continue; // overlapping or empty fold: ignore, the model sorts them out
            }
            let hidden = after_row - start_row;
            let display_start = start_row - hidden_rows;
            spans.push(FoldSpan {
                start_row,
                hidden_rows: hidden,
                display_start,
                header_display: header_row - hidden_rows,
            });
            hidden_rows += hidden;
            previous_end = after_row;
        }
        if spans.is_empty() {
            return None;
        }
        Some(Self { spans, total_rows, hidden_rows })
    }

    /// Number of rows actually on screen.
    pub fn row_count(&self) -> usize {
        self.total_rows.saturating_sub(self.hidden_rows)
    }

    /// Display row -> underlying wrap row. Every visible row maps past the rows
    /// that folding removed; the header row itself stays where it is.
    pub fn to_wrap_row(&self, row: usize) -> usize {
        let mut offset = 0usize;
        for span in &self.spans {
            if row < span.display_start {
                break;
            }
            offset += span.hidden_rows;
        }
        (row + offset).min(self.total_rows.saturating_sub(1))
    }

    /// Underlying wrap row -> display row. Hidden rows map to the header's row.
    pub fn to_display_row(&self, wrap_row: usize) -> usize {
        let mut offset = 0usize;
        for span in &self.spans {
            if wrap_row < span.start_row {
                return wrap_row - offset;
            }
            if wrap_row < span.start_row + span.hidden_rows {
                return span.header_display;
            }
            offset += span.hidden_rows;
        }
        wrap_row - offset
    }
}

/// View-side folding API. Every call keeps the display map in sync.
impl ViewState {
    /// Refresh detection and the display map for this frame; unfolds the caret's
    /// block so the caret can never live inside a hidden row.
    pub fn refresh_folds(&mut self, doc: &Document, settings: &Settings) {
        if !settings.code_folding || doc.hex_only || doc.mode != crate::core::model::EditMode::Text {
            if self.folds.is_empty() {
                self.fold_map = None;
                return;
            }
            self.folds.clear();
            self.fold_map = None;
            return;
        }
        self.folds.refresh(&doc.editor.lines, &doc.syntax, doc.version);
        self.folds.clamp(doc.editor.lines.len());
        if let Some(range) = self.folds.header_of(doc.editor.selection.head.line) {
            self.folds.unfold_at(range.header);
        }
        self.rebuild_fold_map(&doc.editor);
    }

    /// Drop every fold (file reloaded, folding disabled).
    pub fn clear_folds(&mut self) {
        self.folds.clear();
        self.fold_map = None;
    }

    /// Gutter state for `line`: `None` means the line cannot be folded.
    pub fn fold_marker(&self, line: usize) -> Option<FoldMarker> {
        self.folds.marker(line)
    }

    /// Is any block currently collapsed?
    pub fn has_folds(&self) -> bool {
        !self.folds.is_empty()
    }

    /// Toggle the block starting at `line`; `None` when the line has no block.
    pub fn toggle_fold_at(&mut self, line: usize) -> Option<bool> {
        self.folds.toggle(line)
    }

    /// Innermost foldable block that contains `line` (caret inside a body).
    pub fn enclosing_fold_header(&self, line: usize) -> Option<usize> {
        self.folds.enclosing_header(line)
    }

    /// Header line of the collapsed block that hides `line`, if any.
    pub fn fold_header_for_line(&self, line: usize) -> Option<usize> {
        self.folds.header_of(line).map(|range| range.header)
    }

    /// Make sure candidates exist for a document that has never been inspected
    /// (menu command issued before the view painted a frame).
    pub fn ensure_fold_candidates(&mut self, doc: &Document) {
        self.folds.refresh(&doc.editor.lines, &doc.syntax, doc.version);
    }

    pub fn fold_all(&mut self) {
        self.folds.fold_all();
    }

    pub fn unfold_all(&mut self) {
        self.folds.unfold_all();
    }

    fn rebuild_fold_map(&mut self, ed: &EditorState) {
        if self.folds.is_empty() {
            self.fold_map = None;
            return;
        }
        let wrapped = self.wrapped();
        let total_rows = if wrapped { self.wrap.rows.len() } else { ed.lines.len() };
        let map = {
            let wrap = &self.wrap;
            let line_span = move |line: usize| -> (usize, usize) {
                if !wrapped {
                    return (line.min(ed.lines.len()), if line < ed.lines.len() { 1 } else { 0 });
                }
                let first = wrap.line_first_row.get(line).map_or(wrap.rows.len(), |&v| v as usize);
                let next = wrap.line_first_row.get(line + 1).map_or(wrap.rows.len(), |&v| v as usize);
                (first, next.saturating_sub(first))
            };
            DisplayMap::build(&self.folds, total_rows, line_span)
        };
        self.fold_map = map;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(ranges: &[(usize, usize)]) -> FoldModel {
        let mut model = FoldModel::default();
        // Build candidates through detection so the model stays consistent.
        let mut lines = vec![String::new(); 32];
        for (header, end) in ranges {
            lines[*header] = "a {".into();
            lines[*end] = "}".into();
        }
        model.refresh(&lines, "C", 1);
        for (header, _) in ranges {
            model.toggle(*header);
        }
        assert_eq!(model.active().len(), ranges.len());
        model
    }

    fn line_span_unwrapped(line: usize) -> (usize, usize) {
        (line, 1)
    }

    #[test]
    fn test_no_folds_means_no_map() {
        let model = FoldModel::default();
        assert!(DisplayMap::build(&model, 10, line_span_unwrapped).is_none());
    }

    #[test]
    fn test_single_fold_hides_its_rows() {
        let model = model(&[(2, 5)]);
        let map = DisplayMap::build(&model, 10, line_span_unwrapped).expect("map");
        assert_eq!(map.row_count(), 7);
        assert_eq!(map.to_wrap_row(0), 0);
        assert_eq!(map.to_wrap_row(2), 2, "header row stays");
        assert_eq!(map.to_wrap_row(3), 6, "first row after the hidden block");
        assert_eq!(map.to_wrap_row(6), 9);
        assert_eq!(map.to_wrap_row(9), 9, "rows past the end clamp to the last row");
        assert_eq!(map.to_display_row(2), 2, "header line");
        assert_eq!(map.to_display_row(4), 2, "hidden line maps to the header");
        assert_eq!(map.to_display_row(6), 3);
        assert_eq!(map.to_display_row(9), 6);
    }

    #[test]
    fn test_two_folds_accumulate_offsets() {
        let model = model(&[(1, 2), (5, 7)]);
        let map = DisplayMap::build(&model, 10, line_span_unwrapped).expect("map");
        assert_eq!(map.row_count(), 7);
        assert_eq!(map.to_wrap_row(1), 1);
        assert_eq!(map.to_wrap_row(2), 3);
        assert_eq!(map.to_wrap_row(3), 4);
        assert_eq!(map.to_wrap_row(4), 5, "second header");
        assert_eq!(map.to_wrap_row(5), 8);
        assert_eq!(map.to_display_row(3), 2);
        assert_eq!(map.to_display_row(6), 4, "hidden row of the second fold");
        assert_eq!(map.to_display_row(8), 5);
    }

    #[test]
    fn test_wrapped_lines_use_their_row_counts() {
        // Line 2 wraps into three rows; line 4 wraps into two.
        let span = |line: usize| -> (usize, usize) {
            let first = match line {
                0 => 0,
                1 => 1,
                2 => 2,
                3 => 5,
                4 => 6,
                5 => 8,
                6 => 9,
                7 => 10,
                8 => 11,
                _ => 12,
            };
            let rows = match line {
                2 => 3,
                4 => 2,
                _ => 1,
            };
            (first, rows)
        };
        let model = model(&[(2, 4)]);
        let map = DisplayMap::build(&model, 12, span).expect("map");
        // Hidden: line 3 (1 row) + line 4 (2 rows) = 3 rows.
        assert_eq!(map.row_count(), 9);
        assert_eq!(map.to_wrap_row(2), 2, "header line, first wrapped row");
        assert_eq!(map.to_wrap_row(3), 3, "header keeps all its wrapped rows");
        assert_eq!(map.to_wrap_row(5), 8, "first row after the block");
        assert_eq!(map.to_display_row(6), 2, "a wrapped row of a hidden line maps to the header");
        assert_eq!(map.to_display_row(8), 5);
    }

    #[test]
    fn test_fold_at_the_end_of_the_document() {
        let model = model(&[(7, 8)]);
        let map = DisplayMap::build(&model, 9, line_span_unwrapped).expect("map");
        assert_eq!(map.row_count(), 8);
        assert_eq!(map.to_wrap_row(7), 7);
        assert_eq!(map.to_display_row(8), 7);
    }

    #[test]
    fn test_range_boundaries_never_lose_rows() {
        let model = model(&[(2, 5)]);
        let map = DisplayMap::build(&model, 10, |line| (line, 1)).expect("map");
        // Display -> wrap -> display round-trips for every visible row.
        for row in 0..map.row_count() {
            let wrap = map.to_wrap_row(row);
            assert_eq!(map.to_display_row(wrap), row, "row {row}");
        }
    }
}
