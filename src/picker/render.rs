use crate::config::compile_pattern_specs;
use crate::hints::{assign_hints, HintAssignments};
use crate::model::{
    MatchSpan, PickerAction, PickerSnapshot, RenderLine, RenderSpan, RenderStyle, VisibleViewport,
};
use crate::patterns::{find_matches, find_openable_targets};
use crate::renderer::{overlay_status_line, render_hints, SelectedHints};
use crate::viewport::map_visible_viewport;
use unicode_width::UnicodeWidthChar;

/// Visible text and hint assignments that the picker shows for one snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerView {
    pub viewport: VisibleViewport,
    pub assignments: HintAssignments,
}

/// Live input state that changes the picker rendering after a key press.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SelectionState {
    pub multi: bool,
    pub selected: Vec<String>,
    pub typed: String,
}

/// Builds the picker view for the real width of the picker terminal.
///
/// The picker pane has the same width as the source pane, because the picker tab copies the
/// source layout. Thus this width tells which rows were soft-wrapped.
pub fn build_picker_view(snapshot: &PickerSnapshot, width: u16) -> PickerView {
    let viewport = map_visible_viewport(
        snapshot.source.rows.clone(),
        width,
        snapshot.source.content_rows,
    );
    let assignments = assign_hints(find_matches_in(snapshot, &viewport.logical_lines));
    PickerView {
        viewport,
        assignments,
    }
}

/// Renders the picker for the current terminal size and input state.
pub fn render_picker_lines(
    action: PickerAction,
    view: &PickerView,
    selection: &SelectionState,
    width: u16,
    height: u16,
) -> Vec<RenderLine> {
    let mut lines = if view.assignments.is_empty() {
        no_matches_view(action, width, height)
    } else {
        render_hints(
            &view.viewport,
            &view.assignments,
            width,
            height,
            &SelectedHints::from_hints(&selection.selected),
            &selection.typed,
        )
    };

    if selection.multi {
        overlay_status_line(&mut lines, width, &multi_select_status(selection));
    }

    lines
}

fn multi_select_status(selection: &SelectionState) -> String {
    let count = selection.selected.len();
    let typed = if selection.typed.is_empty() {
        String::new()
    } else {
        format!("  typed {}", selection.typed)
    };
    format!(" MULTI  {count} selected{typed}  tab/enter copy  esc cancel")
}

/// Message shown when an action finds nothing to act on.
pub fn no_match_message(action: PickerAction) -> &'static str {
    match action {
        PickerAction::Copy => "Herdr Yank: no copyable matches found",
        PickerAction::OpenUrl => "Herdr Yank: no openable URLs or files found",
    }
}

/// Whether the launcher must open the picker tab, decided before the tab exists.
///
/// The launcher knows only an estimated width, so its soft-wrap joins can differ from the
/// picker. The picker searches again with its real width.
pub fn has_matches(snapshot: &PickerSnapshot, estimated_width: u16) -> bool {
    let viewport = map_visible_viewport(
        snapshot.source.rows.clone(),
        estimated_width,
        snapshot.source.content_rows,
    );
    !find_matches_in(snapshot, &viewport.logical_lines).is_empty()
}

fn find_matches_in(snapshot: &PickerSnapshot, lines: &[String]) -> Vec<MatchSpan> {
    match snapshot.action {
        PickerAction::Copy => find_matches(
            lines,
            &compile_pattern_specs(&snapshot.custom_patterns),
            snapshot.settings.enabled_builtin_patterns.as_deref(),
        ),
        PickerAction::OpenUrl => find_openable_targets(lines),
    }
}

fn no_matches_view(action: PickerAction, width: u16, height: u16) -> Vec<RenderLine> {
    if width == 0 || height == 0 {
        return Vec::new();
    }

    let width = width as usize;
    let height = height as usize;
    let mut lines = Vec::with_capacity(height);
    let message = no_match_message(action);
    let hint = "Press any non-Enter key to close";

    for row in 0..height {
        let text = match row {
            0 => fit_to_width(message, width),
            2 if height > 2 => fit_to_width(hint, width),
            _ => " ".repeat(width),
        };
        lines.push(RenderLine {
            spans: vec![RenderSpan {
                text,
                style: RenderStyle::Unmatched,
            }],
        });
    }

    lines
}

/// Lines that show a failure message and how to close the picker pane.
///
/// The message wraps at the pane width. When the pane is too short, the last message rows
/// give their place to the close instruction.
pub fn failure_view(message: &str, width: u16, height: u16) -> Vec<RenderLine> {
    if width == 0 || height == 0 {
        return Vec::new();
    }

    let width = width as usize;
    let height = height as usize;
    let mut rows = wrap_text(message, width);
    rows.truncate(height.saturating_sub(2).max(1));
    let message_rows = rows.len();
    if message_rows + 2 <= height {
        rows.push(String::new());
        rows.push("Press any key to close".to_string());
    }
    rows.resize(height, String::new());

    rows.into_iter()
        .enumerate()
        .map(|(index, row)| RenderLine {
            spans: vec![RenderSpan {
                text: fit_to_width(&row, width),
                style: if index < message_rows {
                    RenderStyle::Match
                } else {
                    RenderStyle::Unmatched
                },
            }],
        })
        .collect()
}

/// Breaks text into rows of at most `width` terminal columns.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let mut rows = vec![String::new()];
    let mut row_width = 0;
    for ch in text.chars() {
        let char_width = ch.width().unwrap_or(0);
        if row_width + char_width > width && row_width > 0 {
            rows.push(String::new());
            row_width = 0;
        }
        if let Some(row) = rows.last_mut() {
            row.push(ch);
        }
        row_width += char_width;
    }
    rows
}

/// Cuts or pads text to exactly `width` terminal columns.
fn fit_to_width(text: &str, width: usize) -> String {
    let mut output = String::new();
    let mut output_width = 0;
    for ch in text.chars() {
        let char_width = ch.width().unwrap_or(0);
        if output_width + char_width > width {
            break;
        }
        output.push(ch);
        output_width += char_width;
    }
    output.push_str(&" ".repeat(width - output_width));
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PickerReturnContext, PickerSettings, SourcePaneSnapshot};
    use unicode_width::UnicodeWidthStr;

    fn snapshot(rows: Vec<&str>, content_rows: u16) -> PickerSnapshot {
        PickerSnapshot {
            source: SourcePaneSnapshot {
                rows: rows.into_iter().map(str::to_string).collect(),
                content_rows,
            },
            session: PickerReturnContext {
                return_tab_id: "t1".to_string(),
                zoom_picker: false,
            },
            action: PickerAction::Copy,
            custom_patterns: Vec::new(),
            settings: PickerSettings::default(),
        }
    }

    fn texts(lines: &[RenderLine]) -> Vec<String> {
        lines
            .iter()
            .map(|line| line.spans.iter().map(|span| span.text.as_str()).collect())
            .collect()
    }

    #[test]
    fn selected_hints_render_with_selected_styles_and_a_status_line() {
        let snapshot = snapshot(vec!["https://a.test and https://b.test", ""], 2);
        let view = build_picker_view(&snapshot, 40);
        let selection = SelectionState {
            multi: true,
            selected: vec!["a".to_string()],
            typed: String::new(),
        };

        let lines = render_picker_lines(snapshot.action, &view, &selection, 40, 2);

        assert!(lines[0]
            .spans
            .iter()
            .any(|span| span.style == RenderStyle::SelectedHint && span.text == "a"));
        assert!(lines[0]
            .spans
            .iter()
            .any(|span| span.style == RenderStyle::SelectedMatch));
        assert!(lines[0]
            .spans
            .iter()
            .any(|span| span.style == RenderStyle::Hint && span.text == "s"));
        assert_eq!(lines[1].spans[0].style, RenderStyle::Status);
        assert!(lines[1].spans[0].text.contains("1 selected"));
    }

    #[test]
    fn a_one_row_pane_keeps_its_content_instead_of_a_status_line() {
        let snapshot = snapshot(vec!["https://a.test"], 1);
        let view = build_picker_view(&snapshot, 40);
        let selection = SelectionState {
            multi: true,
            selected: vec!["a".to_string()],
            typed: String::new(),
        };

        let lines = render_picker_lines(snapshot.action, &view, &selection, 40, 1);

        assert_eq!(lines.len(), 1);
        assert!(lines[0]
            .spans
            .iter()
            .all(|span| span.style != RenderStyle::Status));
        assert!(lines[0]
            .spans
            .iter()
            .any(|span| span.style == RenderStyle::SelectedHint));
    }

    #[test]
    fn the_picker_width_decides_the_soft_wrap_joins() {
        let snapshot = snapshot(vec!["https://exa", "mple.com"], 2);

        let wrapped = build_picker_view(&snapshot, 11);
        let unwrapped = build_picker_view(&snapshot, 20);

        assert_eq!(
            wrapped.assignments.copied_text_for_hint("a"),
            Some("https://example.com")
        );
        assert_eq!(
            unwrapped.assignments.copied_text_for_hint("a"),
            Some("https://exa")
        );
    }

    #[test]
    fn the_launch_pre_check_uses_the_estimated_width() {
        let split_hex = snapshot(vec!["0x", "1f"], 2);

        assert!(has_matches(&split_hex, 2));
        assert!(!has_matches(&split_hex, 10));
        assert!(!has_matches(&snapshot(vec!["plain text only"], 1), 20));
    }

    #[test]
    fn disabled_builtin_patterns_reach_the_matcher() {
        let mut snapshot = snapshot(vec!["12345 https://example.com"], 1);
        snapshot.settings.enabled_builtin_patterns = Some(vec!["url".to_string()]);

        let view = build_picker_view(&snapshot, 40);

        assert_eq!(view.assignments.len(), 1);
        assert_eq!(
            view.assignments.copied_text_for_hint("a"),
            Some("https://example.com")
        );
    }

    #[test]
    fn open_url_view_assigns_hints_only_to_openable_urls() {
        let mut snapshot = snapshot(
            vec!["https://example.com ftp://host/repo file:///tmp/file"],
            1,
        );
        snapshot.action = PickerAction::OpenUrl;

        let view = build_picker_view(&snapshot, 60);

        assert_eq!(view.assignments.len(), 1);
        assert_eq!(
            view.assignments.copied_text_for_hint("a"),
            Some("https://example.com")
        );
    }

    #[test]
    fn open_url_no_match_view_uses_action_specific_message() {
        let mut snapshot = snapshot(vec!["ftp://host/repo"], 3);
        snapshot.action = PickerAction::OpenUrl;
        let view = build_picker_view(&snapshot, 40);

        let lines = render_picker_lines(snapshot.action, &view, &SelectionState::default(), 40, 3);

        assert!(lines[0].spans[0].text.contains("no openable URLs or files"));
        assert!(lines[2].spans[0].text.starts_with("Press"));
    }

    #[test]
    fn the_failure_view_wraps_the_message_and_fits_the_pane() {
        let lines = failure_view("Herdr Yank: failed to copy (12 bytes)", 16, 6);

        assert_eq!(
            texts(&lines),
            [
                "Herdr Yank: fail",
                "ed to copy (12 b",
                "ytes)           ",
                "                ",
                "Press any key to",
                "                ",
            ]
        );
        assert_eq!(lines[0].spans[0].style, RenderStyle::Match);
        assert_eq!(lines[4].spans[0].style, RenderStyle::Unmatched);
        assert!(lines
            .iter()
            .all(|line| UnicodeWidthStr::width(line.spans[0].text.as_str()) == 16));
    }

    #[test]
    fn a_short_pane_keeps_the_start_of_the_failure_message() {
        let lines = failure_view("a long failure message", 10, 2);

        assert_eq!(texts(&lines), ["a long fai", "          "]);
        assert!(failure_view("x", 0, 5).is_empty());
    }
}
