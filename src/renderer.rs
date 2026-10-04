pub mod terminal;

use crate::hints::HintAssignments;
use crate::model::{
    LogicalLineVisualSegment, MatchSpan, RenderLine, RenderSpan, RenderStyle, VisibleViewport,
};
use std::collections::HashSet;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Hints the user has toggled on in multi-select mode.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SelectedHints(HashSet<String>);

impl SelectedHints {
    pub fn from_hints(hints: &[String]) -> Self {
        Self(hints.iter().cloned().collect())
    }

    fn contains(&self, hint: &str) -> bool {
        self.0.contains(hint)
    }
}

/// Renders the visible rows with inline hints at their source row and column.
///
/// The output is clipped or padded to `width` × `height`, so it fits the real picker
/// terminal. While the user types a hint, only hints that start with `typed` stay visible.
pub fn render_hints(
    viewport: &VisibleViewport,
    assignments: &HintAssignments,
    width: u16,
    height: u16,
    selected: &SelectedHints,
    typed: &str,
) -> Vec<RenderLine> {
    if width == 0 || height == 0 {
        return Vec::new();
    }

    let width = width as usize;
    let height = height as usize;
    let mut cells = viewport
        .rows
        .iter()
        .take(height)
        .map(|row| row_cells(row, width))
        .collect::<Vec<_>>();
    while cells.len() < height {
        cells.push(row_cells("", width));
    }

    for assignment in assignments.assignments() {
        let is_selected = selected.contains(&assignment.hint);
        let shows_hint = assignment.hint.starts_with(typed);
        if !shows_hint && !is_selected {
            continue;
        }

        let styles = OccurrenceStyles::for_selection(is_selected);
        let hint = if shows_hint {
            assignment.hint.as_str()
        } else {
            ""
        };
        for occurrence in &assignment.occurrences {
            apply_visible_occurrence(
                &mut cells,
                &viewport.rows,
                &viewport.segments,
                occurrence,
                hint,
                styles,
            );
        }
    }

    for row in &mut cells {
        repair_wide_cells(row);
    }
    cells.into_iter().map(cells_to_render_line).collect()
}

/// Style pair used for one match, depending on whether its hint is selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OccurrenceStyles {
    match_style: RenderStyle,
    hint_style: RenderStyle,
}

impl OccurrenceStyles {
    fn for_selection(selected: bool) -> Self {
        if selected {
            Self {
                match_style: RenderStyle::SelectedMatch,
                hint_style: RenderStyle::SelectedHint,
            }
        } else {
            Self {
                match_style: RenderStyle::Match,
                hint_style: RenderStyle::Hint,
            }
        }
    }
}

/// Replaces one rendered row with a picker status line.
///
/// The status line uses the lowest row that shows no match, so it hides no hint. When every
/// row shows a match, it uses the last row. A one-row pane keeps its content instead, because
/// covering the only row would hide every hint the status line is describing.
pub fn overlay_status_line(lines: &mut [RenderLine], width: u16, status: &str) {
    if lines.len() < 2 {
        return;
    }
    let index = lines
        .iter()
        .rposition(|line| line.spans.iter().all(|span| !span.style.is_match()))
        .unwrap_or(lines.len() - 1);

    let width = width as usize;
    let mut text: String = status.chars().collect();
    let mut text_width = UnicodeWidthStr::width(text.as_str());
    while text_width > width {
        text.pop();
        text_width = UnicodeWidthStr::width(text.as_str());
    }
    text.push_str(&" ".repeat(width - text_width));

    lines[index] = RenderLine {
        spans: vec![RenderSpan {
            text,
            style: RenderStyle::Status,
        }],
    };
}

/// One terminal column. A wide character fills its cell and an empty continuation cell.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Cell {
    text: String,
    style: RenderStyle,
}

fn row_cells(row: &str, width: usize) -> Vec<Cell> {
    let mut cells = Vec::with_capacity(width);
    for ch in row.chars() {
        let char_width = ch.width().unwrap_or(0);
        if char_width == 0 {
            continue;
        }
        if cells.len() + char_width > width {
            break;
        }

        cells.push(Cell {
            text: ch.to_string(),
            style: RenderStyle::Unmatched,
        });
        for _ in 1..char_width {
            cells.push(Cell {
                text: String::new(),
                style: RenderStyle::Unmatched,
            });
        }
    }

    while cells.len() < width {
        cells.push(Cell {
            text: " ".to_string(),
            style: RenderStyle::Unmatched,
        });
    }
    cells
}

fn apply_visible_occurrence(
    cells: &mut [Vec<Cell>],
    rows: &[String],
    segments: &[LogicalLineVisualSegment],
    occurrence: &MatchSpan,
    hint: &str,
    styles: OccurrenceStyles,
) {
    let mut positions = Vec::new();
    for segment in segments
        .iter()
        .filter(|segment| segment.logical_line == occurrence.line)
    {
        let start = occurrence.start.max(segment.logical_start);
        let end = occurrence.end.min(segment.logical_end);
        if start >= end {
            continue;
        }

        let Some(segment_text) = rows.get(segment.row) else {
            continue;
        };
        let start_in_segment = start.saturating_sub(segment.logical_start);
        let end_in_segment = end.saturating_sub(segment.logical_start);
        let Some(start_cols) = display_width_until_byte(segment_text, start_in_segment) else {
            continue;
        };
        let Some(end_cols) = display_width_until_byte(segment_text, end_in_segment) else {
            continue;
        };
        let col_start = segment.col_start + start_cols;
        let col_end = segment.col_start + end_cols;
        for col in col_start..col_end.min(segment.col_end) {
            positions.push((segment.row, col));
        }
    }

    if positions.is_empty() {
        return;
    }

    for &(row, col) in &positions {
        if let Some(cell) = cells.get_mut(row).and_then(|row| row.get_mut(col)) {
            cell.style = styles.match_style;
        }
    }

    // A match that is shorter than its hint still shows the full hint in the next cells.
    let mut hint_cells = positions;
    if let Some(&(row, last_col)) = hint_cells.last() {
        let row_width = cells.get(row).map_or(0, Vec::len);
        let missing = hint.chars().count().saturating_sub(hint_cells.len());
        hint_cells.extend(
            (last_col + 1..row_width)
                .take(missing)
                .map(|col| (row, col)),
        );
    }

    for ((row, col), hint_ch) in hint_cells.into_iter().zip(hint.chars()) {
        if let Some(cell) = cells.get_mut(row).and_then(|row| row.get_mut(col)) {
            cell.text = hint_ch.to_string();
            cell.style = styles.hint_style;
        }
    }
}

/// Keeps a row at its full width after hint letters replace half of a wide character.
///
/// A wide character needs its own cell and an empty continuation cell. When a hint letter
/// replaces one of the two, the other one becomes a space.
fn repair_wide_cells(row: &mut [Cell]) {
    for index in 0..row.len() {
        let is_wide = UnicodeWidthStr::width(row[index].text.as_str()) == 2;
        let has_continuation = row.get(index + 1).is_some_and(|next| next.text.is_empty());
        if is_wide && !has_continuation {
            row[index].text = " ".to_string();
        }

        let is_orphan = row[index].text.is_empty()
            && (index == 0 || UnicodeWidthStr::width(row[index - 1].text.as_str()) != 2);
        if is_orphan {
            row[index].text = " ".to_string();
        }
    }
}

fn cells_to_render_line(cells: Vec<Cell>) -> RenderLine {
    let mut spans = Vec::new();
    for cell in cells {
        push_span(&mut spans, &cell.text, cell.style);
    }
    RenderLine { spans }
}

fn display_width_until_byte(text: &str, byte_offset: usize) -> Option<usize> {
    if byte_offset > text.len() || !text.is_char_boundary(byte_offset) {
        return None;
    }
    Some(UnicodeWidthStr::width(&text[..byte_offset]))
}

/// Pushes a text span into the line, merging with the previous span if styles match.
fn push_span(spans: &mut Vec<RenderSpan>, text: &str, style: RenderStyle) {
    if text.is_empty() {
        return;
    }

    if let Some(last) = spans.last_mut() {
        if last.style == style {
            last.text.push_str(text);
            return;
        }
    }

    spans.push(RenderSpan {
        text: text.to_string(),
        style,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hints::assign_hints;
    use crate::model::HintAssignment;
    use crate::viewport::map_visible_viewport;

    fn span(text: impl Into<String>, line: usize, start: usize) -> MatchSpan {
        let text = text.into();
        MatchSpan {
            line,
            start,
            end: start + text.len(),
            text,
            pattern: "test".to_string(),
            priority: 10,
        }
    }

    fn assignment(hint: &str, text: &str, occurrences: Vec<MatchSpan>) -> HintAssignment {
        HintAssignment {
            hint: hint.to_string(),
            text: text.to_string(),
            occurrences,
        }
    }

    fn viewport(rows: &[&str], width: u16, height: u16) -> VisibleViewport {
        map_visible_viewport(
            rows.iter().map(|row| row.to_string()).collect(),
            width,
            height,
        )
    }

    fn render(
        viewport: &VisibleViewport,
        assignments: &HintAssignments,
        width: u16,
        height: u16,
    ) -> Vec<RenderLine> {
        render_hints(
            viewport,
            assignments,
            width,
            height,
            &SelectedHints::default(),
            "",
        )
    }

    fn assert_rows(lines: &[RenderLine], expected: &[&[(RenderStyle, &str)]]) {
        let actual: Vec<Vec<(RenderStyle, &str)>> = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| (span.style, span.text.as_str()))
                    .collect()
            })
            .collect();
        let expected: Vec<Vec<(RenderStyle, &str)>> =
            expected.iter().map(|row| row.to_vec()).collect();

        assert_eq!(actual, expected);
    }

    fn line_width(line: &RenderLine) -> usize {
        line.spans
            .iter()
            .map(|span| UnicodeWidthStr::width(span.text.as_str()))
            .sum()
    }

    #[test]
    fn zero_dimensions_return_no_rows() {
        let assignments = HintAssignments::new(Vec::new());
        let viewport = viewport(&["abc"], 3, 1);

        assert!(render(&viewport, &assignments, 0, 3).is_empty());
        assert!(render(&viewport, &assignments, 3, 0).is_empty());
    }

    #[test]
    fn visible_render_preserves_top_row_position() {
        let viewport = viewport(&["https://example.com", "", ""], 30, 3);
        let assignments = assign_hints(vec![span("https://example.com", 0, 0)]);

        let lines = render(&viewport, &assignments, 30, 3);

        assert_rows(
            &lines,
            &[
                &[
                    (RenderStyle::Hint, "a"),
                    (RenderStyle::Match, "ttps://example.com"),
                    (RenderStyle::Unmatched, "           "),
                ],
                &[(RenderStyle::Unmatched, "                              ")],
                &[(RenderStyle::Unmatched, "                              ")],
            ],
        );
    }

    #[test]
    fn visible_render_maps_match_after_wide_utf8_prefix() {
        let row = "界 https://example.com";
        let viewport = viewport(&[row], 30, 1);
        let assignments = assign_hints(vec![MatchSpan {
            line: 0,
            start: "界 ".len(),
            end: row.len(),
            text: "https://example.com".to_string(),
            pattern: "url".to_string(),
            priority: 10,
        }]);

        let lines = render(&viewport, &assignments, 30, 1);

        assert_rows(
            &lines,
            &[&[
                (RenderStyle::Unmatched, "界 "),
                (RenderStyle::Hint, "a"),
                (RenderStyle::Match, "ttps://example.com"),
                (RenderStyle::Unmatched, "        "),
            ]],
        );
    }

    #[test]
    fn visible_render_highlights_wrapped_match_across_rows() {
        let viewport = viewport(&["https://exa", "mple.com"], 11, 2);
        let assignments = assign_hints(vec![span("https://example.com", 0, 0)]);

        let lines = render(&viewport, &assignments, 11, 2);

        assert_rows(
            &lines,
            &[
                &[(RenderStyle::Hint, "a"), (RenderStyle::Match, "ttps://exa")],
                &[
                    (RenderStyle::Match, "mple.com"),
                    (RenderStyle::Unmatched, "   "),
                ],
            ],
        );
    }

    #[test]
    fn output_is_clipped_and_padded_to_the_terminal_size() {
        let viewport = viewport(&["abcdef", "second", "third"], 6, 3);
        let assignments = HintAssignments::new(Vec::new());

        assert_rows(
            &render(&viewport, &assignments, 4, 2),
            &[
                &[(RenderStyle::Unmatched, "abcd")],
                &[(RenderStyle::Unmatched, "seco")],
            ],
        );
        assert_rows(
            &render(&viewport, &assignments, 7, 4),
            &[
                &[(RenderStyle::Unmatched, "abcdef ")],
                &[(RenderStyle::Unmatched, "second ")],
                &[(RenderStyle::Unmatched, "third  ")],
                &[(RenderStyle::Unmatched, "       ")],
            ],
        );
    }

    #[test]
    fn duplicate_text_occurrences_render_same_hint() {
        let viewport = viewport(&["foo then foo"], 12, 1);
        let assignments = assign_hints(vec![span("foo", 0, 0), span("foo", 0, 9)]);

        assert_rows(
            &render(&viewport, &assignments, 12, 1),
            &[&[
                (RenderStyle::Hint, "a"),
                (RenderStyle::Match, "oo"),
                (RenderStyle::Unmatched, " then "),
                (RenderStyle::Hint, "a"),
                (RenderStyle::Match, "oo"),
            ]],
        );
    }

    #[test]
    fn duplicate_exact_occurrence_is_rendered_once() {
        let viewport = viewport(&["foo"], 3, 1);
        let assignments = HintAssignments::new(vec![assignment(
            "a",
            "foo",
            vec![span("foo", 0, 0), span("foo", 0, 0)],
        )]);

        assert_rows(
            &render(&viewport, &assignments, 3, 1),
            &[&[(RenderStyle::Hint, "a"), (RenderStyle::Match, "oo")]],
        );
    }

    #[test]
    fn invalid_occurrences_are_ignored() {
        let viewport = viewport(&["abc", "éx"], 4, 2);
        let assignments = HintAssignments::new(vec![assignment(
            "a",
            "bad",
            vec![
                MatchSpan {
                    line: 9,
                    ..span("abc", 0, 0)
                },
                // Byte 1 is inside `é`, so it is not a character boundary.
                MatchSpan {
                    line: 1,
                    start: 1,
                    end: 3,
                    ..span("x", 1, 0)
                },
            ],
        )]);

        assert_rows(
            &render(&viewport, &assignments, 4, 2),
            &[
                &[(RenderStyle::Unmatched, "abc ")],
                &[(RenderStyle::Unmatched, "éx  ")],
            ],
        );
    }

    #[test]
    fn a_short_match_still_shows_the_full_hint() {
        let viewport = viewport(&["x a y"], 5, 1);
        let assignments = HintAssignments::new(vec![assignment("as", "a", vec![span("a", 0, 2)])]);

        assert_rows(
            &render(&viewport, &assignments, 5, 1),
            &[&[
                (RenderStyle::Unmatched, "x "),
                (RenderStyle::Hint, "as"),
                (RenderStyle::Unmatched, "y"),
            ]],
        );
    }

    #[test]
    fn a_hint_over_a_wide_character_keeps_the_row_width() {
        let row = "日本/foo bar";
        let viewport = viewport(&[row], 20, 1);
        for hint in ["a", "as", "asd"] {
            let assignments = HintAssignments::new(vec![assignment(
                hint,
                "日本/foo",
                vec![span("日本/foo", 0, 0)],
            )]);

            let lines = render(&viewport, &assignments, 20, 1);

            assert_eq!(line_width(&lines[0]), 20, "hint {hint}: {lines:?}");
            assert!(lines[0]
                .spans
                .iter()
                .any(|span| span.style == RenderStyle::Hint && span.text == hint));
        }
    }

    #[test]
    fn a_typed_prefix_hides_the_hints_that_do_not_start_with_it() {
        let viewport = viewport(&["one two"], 7, 1);
        let assignments = HintAssignments::new(vec![
            assignment("as", "one", vec![span("one", 0, 0)]),
            assignment("sa", "two", vec![span("two", 0, 4)]),
        ]);

        let lines = render_hints(
            &viewport,
            &assignments,
            7,
            1,
            &SelectedHints::default(),
            "s",
        );

        assert_rows(
            &lines,
            &[&[
                (RenderStyle::Unmatched, "one "),
                (RenderStyle::Hint, "sa"),
                (RenderStyle::Match, "o"),
            ]],
        );
    }

    #[test]
    fn selected_hints_use_selected_styles_and_stay_marked_while_typing() {
        let viewport = viewport(&["one two"], 7, 1);
        let assignments = HintAssignments::new(vec![
            assignment("as", "one", vec![span("one", 0, 0)]),
            assignment("sa", "two", vec![span("two", 0, 4)]),
        ]);
        let selected = SelectedHints::from_hints(&["as".to_string()]);

        assert_rows(
            &render_hints(&viewport, &assignments, 7, 1, &selected, ""),
            &[&[
                (RenderStyle::SelectedHint, "as"),
                (RenderStyle::SelectedMatch, "e"),
                (RenderStyle::Unmatched, " "),
                (RenderStyle::Hint, "sa"),
                (RenderStyle::Match, "o"),
            ]],
        );
        assert_rows(
            &render_hints(&viewport, &assignments, 7, 1, &selected, "s"),
            &[&[
                (RenderStyle::SelectedMatch, "one"),
                (RenderStyle::Unmatched, " "),
                (RenderStyle::Hint, "sa"),
                (RenderStyle::Match, "o"),
            ]],
        );
    }

    fn plain(text: &str) -> RenderLine {
        RenderLine {
            spans: vec![RenderSpan {
                text: text.to_string(),
                style: RenderStyle::Unmatched,
            }],
        }
    }

    fn hinted(text: &str) -> RenderLine {
        RenderLine {
            spans: vec![RenderSpan {
                text: text.to_string(),
                style: RenderStyle::Hint,
            }],
        }
    }

    #[test]
    fn the_status_line_uses_the_lowest_row_without_a_match() {
        let mut lines = vec![plain("aaaa"), plain("bbbb"), hinted("cccc")];

        overlay_status_line(&mut lines, 4, "status");

        assert_eq!(lines[1].spans[0].style, RenderStyle::Status);
        assert_eq!(lines[1].spans[0].text, "stat");
        assert_eq!(lines[2], hinted("cccc"));
    }

    #[test]
    fn the_status_line_uses_the_last_row_when_every_row_has_a_match() {
        let mut lines = vec![hinted("aaaa"), hinted("bbbb")];

        overlay_status_line(&mut lines, 6, "st");

        assert_eq!(lines[0], hinted("aaaa"));
        assert_eq!(lines[1].spans[0].text, "st    ");
    }

    #[test]
    fn a_one_row_pane_keeps_its_content() {
        let mut lines = vec![hinted("aaaa")];

        overlay_status_line(&mut lines, 4, "status");

        assert_eq!(lines, [hinted("aaaa")]);
    }
}
