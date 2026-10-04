use crate::clipboard::{Clipboard, SystemClipboard};
use crate::hints::HintAssignments;
use crate::model::{PickerAction, PickerOutcome, PickerSnapshot};
use crate::picker::copy::copy_selected_text;
use crate::picker::input::{
    CrosstermInputSource, CursorGuard, InputDecision, InputSource, InputState, PickerInputEvent,
    RawModeGuard,
};
use crate::picker::open_url::open_selected_url;
use crate::picker::render::{
    build_picker_view, failure_view, render_picker_lines, PickerView, SelectionState,
};
use crate::renderer::terminal;
use crate::url_opener::{SystemUrlOpener, UrlOpener};
use anyhow::{anyhow, Context, Result};
use std::io::{self, Write};

/// Runs the production picker input/copy flow for a captured source snapshot.
pub fn run_picker(snapshot: &PickerSnapshot) -> Result<PickerOutcome> {
    let mut stdout = io::stdout();
    let mut input = CrosstermInputSource;
    let clipboard = SystemClipboard::new(snapshot.settings.clipboard);
    let url_opener = SystemUrlOpener;
    let _raw_mode = RawModeGuard::enable()?;
    let _cursor = CursorGuard::hide()?;
    let size = crossterm::terminal::size().context("failed to read picker terminal size")?;
    run_picker_with(
        snapshot,
        size,
        &mut input,
        &clipboard,
        &url_opener,
        &mut stdout,
    )
}

/// Runs the picker in a terminal of `size` (columns, rows).
pub(crate) fn run_picker_with<I, C, O, W>(
    snapshot: &PickerSnapshot,
    size: (u16, u16),
    input: &mut I,
    clipboard: &C,
    url_opener: &O,
    output: &mut W,
) -> Result<PickerOutcome>
where
    I: InputSource,
    C: Clipboard,
    O: UrlOpener,
    W: Write,
{
    let view = build_picker_view(snapshot, size.0);
    let valid_hints = view.assignments.valid_hints().collect::<Vec<_>>();
    // Opening several browser tabs from one key press is a surprise, so multi-select is
    // limited to the copy action.
    let multi_select_enabled = snapshot.action == PickerAction::Copy;
    let mut input_state =
        InputState::new(view.assignments.width().unwrap_or(0), multi_select_enabled);
    let mut size = size;
    draw(output, snapshot, &view, &input_state, size)?;

    loop {
        let event = input.read_event()?;
        if let PickerInputEvent::Resize(columns, rows) = event {
            // The hints stay; only the clip and the padding follow the new terminal size.
            size = (columns, rows);
            draw(output, snapshot, &view, &input_state, size)?;
            continue;
        }

        if view.assignments.is_empty() {
            match event {
                PickerInputEvent::Escape | PickerInputEvent::CtrlC => {
                    return Ok(PickerOutcome::Cancelled)
                }
                PickerInputEvent::Char(_) => return Ok(PickerOutcome::NoMatches),
                _ => continue,
            }
        }

        match input_state.push(event, &valid_hints) {
            InputDecision::Continue => continue,
            InputDecision::Cancel => return Ok(PickerOutcome::Cancelled),
            // An invalid hint clears the typed prefix, so the hidden hints come back.
            InputDecision::Redraw | InputDecision::InvalidHint => {
                draw(output, snapshot, &view, &input_state, size)?;
            }
            InputDecision::CommitSelection(hints) => {
                let text = joined_selection(&view.assignments, &hints)?;
                return finish(clipboard, url_opener, snapshot, &text);
            }
            InputDecision::CopyHint(hint) => {
                let text = view
                    .assignments
                    .copied_text_for_hint(&hint)
                    .ok_or_else(|| anyhow!("accepted unknown picker hint {hint}"))?
                    .to_string();
                return finish(clipboard, url_opener, snapshot, &text);
            }
        }
    }
}

fn draw(
    output: &mut impl Write,
    snapshot: &PickerSnapshot,
    view: &PickerView,
    input_state: &InputState,
    (columns, rows): (u16, u16),
) -> Result<()> {
    let selection = SelectionState {
        multi: input_state.is_multi(),
        selected: input_state.selected().to_vec(),
        typed: input_state.typed().to_string(),
    };
    let lines = render_picker_lines(snapshot.action, view, &selection, columns, rows);
    terminal::emit_render_lines_themed(output, &lines, &snapshot.settings.theme)?;
    output.flush()?;
    Ok(())
}

/// Joins the copied text of every selected hint, one per line, in selection order.
fn joined_selection(assignments: &HintAssignments, hints: &[String]) -> Result<String> {
    let mut texts = Vec::with_capacity(hints.len());
    for hint in hints {
        let text = assignments
            .copied_text_for_hint(hint)
            .ok_or_else(|| anyhow!("accepted unknown picker hint {hint}"))?;
        texts.push(text);
    }
    Ok(texts.join("\n"))
}

/// Performs the picker action on the selected text.
///
/// A failure returns an error without the text, and the caller shows it to the user.
fn finish<C, O>(
    clipboard: &C,
    url_opener: &O,
    snapshot: &PickerSnapshot,
    text: &str,
) -> Result<PickerOutcome>
where
    C: Clipboard,
    O: UrlOpener,
{
    match snapshot.action {
        PickerAction::Copy => copy_selected_text(clipboard, text).map(|()| PickerOutcome::Copied {
            text: text.to_string(),
        }),
        PickerAction::OpenUrl => {
            open_selected_url(url_opener, text).map(|()| PickerOutcome::OpenedUrl {
                url: text.to_string(),
            })
        }
    }
}

/// Shows a failure message on the picker pane and waits for one key press.
pub fn show_failure_and_wait(message: &str) -> Result<()> {
    let _raw_mode = RawModeGuard::enable()?;
    let _cursor = CursorGuard::hide()?;
    let size = crossterm::terminal::size().context("failed to read picker terminal size")?;
    show_failure_with(message, size, &mut CrosstermInputSource, &mut io::stdout())
}

pub(crate) fn show_failure_with(
    message: &str,
    size: (u16, u16),
    input: &mut impl InputSource,
    output: &mut impl Write,
) -> Result<()> {
    draw_failure(output, message, size)?;

    loop {
        match input.read_event()? {
            PickerInputEvent::Resize(columns, rows) => {
                draw_failure(output, message, (columns, rows))?;
            }
            PickerInputEvent::Other => {}
            _ => return Ok(()),
        }
    }
}

fn draw_failure(output: &mut impl Write, message: &str, (columns, rows): (u16, u16)) -> Result<()> {
    terminal::emit_render_lines(output, &failure_view(message, columns, rows))?;
    output.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::ClipboardError;
    use crate::model::{PickerReturnContext, PickerSettings, SourcePaneSnapshot};
    use crate::url_opener::UrlOpenError;
    use std::cell::RefCell;

    struct FakeInput {
        events: Vec<PickerInputEvent>,
    }

    impl FakeInput {
        fn new(events: Vec<PickerInputEvent>) -> Self {
            Self {
                events: events.into_iter().rev().collect(),
            }
        }
    }

    impl InputSource for FakeInput {
        fn read_event(&mut self) -> Result<PickerInputEvent> {
            self.events
                .pop()
                .ok_or_else(|| anyhow!("fake input exhausted"))
        }
    }

    #[derive(Default)]
    struct FakeClipboard {
        copied: RefCell<Vec<String>>,
        error: Option<ClipboardError>,
    }

    impl Clipboard for FakeClipboard {
        fn copy(&self, text: &str) -> std::result::Result<(), ClipboardError> {
            self.copied.borrow_mut().push(text.to_string());
            self.error.clone().map_or(Ok(()), Err)
        }
    }

    #[derive(Default)]
    struct FakeUrlOpener {
        opened: RefCell<Vec<String>>,
        error: Option<UrlOpenError>,
    }

    impl UrlOpener for FakeUrlOpener {
        fn open(&self, url: &str) -> std::result::Result<(), UrlOpenError> {
            self.opened.borrow_mut().push(url.to_string());
            self.error.clone().map_or(Ok(()), Err)
        }
    }

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

    /// Runs the picker with fakes and returns the outcome plus the terminal output.
    fn run(
        snapshot: &PickerSnapshot,
        size: (u16, u16),
        events: Vec<PickerInputEvent>,
        clipboard: &FakeClipboard,
        opener: &FakeUrlOpener,
    ) -> (Result<PickerOutcome>, String) {
        let mut output = Vec::new();
        let outcome = run_picker_with(
            snapshot,
            size,
            &mut FakeInput::new(events),
            clipboard,
            opener,
            &mut output,
        );
        (outcome, String::from_utf8(output).unwrap())
    }

    fn frames(output: &str) -> usize {
        output.matches("\u{1b}[?2026h").count()
    }

    #[test]
    fn multi_select_copies_every_selection_joined_by_newlines() {
        let clipboard = FakeClipboard::default();

        let (outcome, _) = run(
            &snapshot(vec!["https://a.test /tmp/second"], 1),
            (40, 1),
            vec![
                PickerInputEvent::Tab,
                PickerInputEvent::Char('s'),
                PickerInputEvent::Char('a'),
                PickerInputEvent::Tab,
            ],
            &clipboard,
            &FakeUrlOpener::default(),
        );

        assert_eq!(
            outcome.unwrap(),
            PickerOutcome::Copied {
                text: "/tmp/second\nhttps://a.test".to_string()
            }
        );
        assert_eq!(clipboard.copied.borrow().len(), 1);
    }

    #[test]
    fn deselecting_everything_keeps_the_picker_open() {
        let clipboard = FakeClipboard::default();

        let (outcome, _) = run(
            &snapshot(vec!["https://a.test /tmp/second"], 1),
            (40, 1),
            vec![
                PickerInputEvent::Tab,
                PickerInputEvent::Char('a'),
                PickerInputEvent::Char('a'),
                PickerInputEvent::Tab,
                PickerInputEvent::Escape,
            ],
            &clipboard,
            &FakeUrlOpener::default(),
        );

        assert_eq!(outcome.unwrap(), PickerOutcome::Cancelled);
        assert!(clipboard.copied.borrow().is_empty());
    }

    #[test]
    fn open_url_action_ignores_multi_select() {
        let mut snapshot = snapshot(vec!["https://a.test https://b.test"], 1);
        snapshot.action = PickerAction::OpenUrl;
        let opener = FakeUrlOpener::default();

        let (outcome, _) = run(
            &snapshot,
            (40, 1),
            vec![PickerInputEvent::Tab, PickerInputEvent::Char('a')],
            &FakeClipboard::default(),
            &opener,
        );

        assert_eq!(
            outcome.unwrap(),
            PickerOutcome::OpenedUrl {
                url: "https://a.test".to_string()
            }
        );
        assert_eq!(opener.opened.borrow().len(), 1);
    }

    #[test]
    fn exact_hint_copies_matched_text_not_hint() {
        let clipboard = FakeClipboard::default();

        let (outcome, _) = run(
            &snapshot(vec!["open https://example.com/path"], 1),
            (40, 1),
            vec![PickerInputEvent::Char('a')],
            &clipboard,
            &FakeUrlOpener::default(),
        );

        assert_eq!(
            outcome.unwrap(),
            PickerOutcome::Copied {
                text: "https://example.com/path".to_string()
            }
        );
        assert_eq!(
            clipboard.copied.borrow().as_slice(),
            &["https://example.com/path".to_string()]
        );
    }

    #[test]
    fn open_url_hint_uses_opener_without_copying() {
        let mut snapshot = snapshot(vec!["open https://example.com/path"], 1);
        snapshot.action = PickerAction::OpenUrl;
        let clipboard = FakeClipboard::default();
        let opener = FakeUrlOpener::default();

        let (outcome, _) = run(
            &snapshot,
            (40, 1),
            vec![PickerInputEvent::Char('a')],
            &clipboard,
            &opener,
        );

        assert_eq!(
            outcome.unwrap(),
            PickerOutcome::OpenedUrl {
                url: "https://example.com/path".to_string()
            }
        );
        assert!(clipboard.copied.borrow().is_empty());
        assert_eq!(
            opener.opened.borrow().as_slice(),
            &["https://example.com/path"]
        );
    }

    #[test]
    fn opener_failure_returns_an_error_without_the_url() {
        let mut snapshot = snapshot(vec!["https://example.com/?token=hunter2"], 1);
        snapshot.action = PickerAction::OpenUrl;
        let opener = FakeUrlOpener {
            error: Some(UrlOpenError::NoToolFound { tool: "fake-open" }),
            ..FakeUrlOpener::default()
        };

        let (outcome, _) = run(
            &snapshot,
            (40, 1),
            vec![PickerInputEvent::Char('a')],
            &FakeClipboard::default(),
            &opener,
        );

        let message = format!("{:#}", outcome.unwrap_err());
        assert!(message.contains("failed to open selected URL or file"));
        assert!(!message.contains("hunter2"));
    }

    #[test]
    fn clipboard_failure_returns_an_error_without_the_text() {
        let clipboard = FakeClipboard {
            error: Some(ClipboardError::NoToolFound {
                tried: "fake-copy".to_string(),
            }),
            ..FakeClipboard::default()
        };

        let (outcome, _) = run(
            &snapshot(vec!["open https://example.com/secret-path"], 1),
            (40, 1),
            vec![PickerInputEvent::Char('a')],
            &clipboard,
            &FakeUrlOpener::default(),
        );

        let message = format!("{:#}", outcome.unwrap_err());
        assert!(message.contains("failed to copy selected text (31 bytes)"));
        assert!(!message.contains("secret-path"));
    }

    #[test]
    fn duplicate_hint_copies_shared_text_once() {
        let clipboard = FakeClipboard::default();

        let (outcome, _) = run(
            &snapshot(
                vec!["https://example.com first", "again https://example.com"],
                2,
            ),
            (40, 2),
            vec![PickerInputEvent::Char('a')],
            &clipboard,
            &FakeUrlOpener::default(),
        );

        assert_eq!(
            outcome.unwrap(),
            PickerOutcome::Copied {
                text: "https://example.com".to_string()
            }
        );
        assert_eq!(clipboard.copied.borrow().len(), 1);
    }

    #[test]
    fn an_invalid_hint_redraws_and_keeps_the_picker_active() {
        let clipboard = FakeClipboard::default();

        let (outcome, output) = run(
            &snapshot(vec!["open https://example.com/path"], 1),
            (40, 1),
            vec![PickerInputEvent::Char('x'), PickerInputEvent::Char('a')],
            &clipboard,
            &FakeUrlOpener::default(),
        );

        assert!(matches!(outcome.unwrap(), PickerOutcome::Copied { .. }));
        assert_eq!(frames(&output), 2, "the invalid hint must redraw");
        assert_eq!(clipboard.copied.borrow().len(), 1);
    }

    #[test]
    fn a_resize_redraws_with_the_new_size() {
        let (outcome, output) = run(
            &snapshot(vec!["open https://example.com/path"], 1),
            (40, 1),
            vec![PickerInputEvent::Resize(30, 2), PickerInputEvent::Escape],
            &FakeClipboard::default(),
            &FakeUrlOpener::default(),
        );

        assert_eq!(outcome.unwrap(), PickerOutcome::Cancelled);
        assert_eq!(frames(&output), 2);
        // The second frame draws two rows of 30 columns.
        let last_frame = output.rsplit("\u{1b}[?2026h").next().unwrap();
        assert!(last_frame.contains("\u{1b}[2;1H"));
    }

    #[test]
    fn escape_and_ctrl_c_cancel_without_copying() {
        for event in [PickerInputEvent::Escape, PickerInputEvent::CtrlC] {
            let clipboard = FakeClipboard::default();

            let (outcome, _) = run(
                &snapshot(vec!["open https://example.com/path"], 1),
                (40, 1),
                vec![event],
                &clipboard,
                &FakeUrlOpener::default(),
            );

            assert_eq!(outcome.unwrap(), PickerOutcome::Cancelled);
            assert!(clipboard.copied.borrow().is_empty());
        }
    }

    #[test]
    fn enter_is_ignored_before_valid_hint() {
        let (outcome, _) = run(
            &snapshot(vec!["open https://example.com/path"], 1),
            (40, 1),
            vec![PickerInputEvent::Enter, PickerInputEvent::Char('a')],
            &FakeClipboard::default(),
            &FakeUrlOpener::default(),
        );

        assert!(matches!(outcome.unwrap(), PickerOutcome::Copied { .. }));
    }

    #[test]
    fn no_matches_waits_for_close_key_and_returns_no_matches() {
        let clipboard = FakeClipboard::default();

        let (outcome, output) = run(
            &snapshot(vec!["plain text only"], 3),
            (30, 3),
            vec![
                PickerInputEvent::Enter,
                PickerInputEvent::Resize(20, 3),
                PickerInputEvent::Char('q'),
            ],
            &clipboard,
            &FakeUrlOpener::default(),
        );

        assert_eq!(outcome.unwrap(), PickerOutcome::NoMatches);
        assert_eq!(frames(&output), 2);
        assert!(clipboard.copied.borrow().is_empty());
    }

    #[test]
    fn the_failure_screen_waits_for_one_key() {
        let mut input = FakeInput::new(vec![
            PickerInputEvent::Other,
            PickerInputEvent::Resize(20, 4),
            PickerInputEvent::Enter,
        ]);
        let mut output = Vec::new();

        show_failure_with("Herdr Yank: copy failed", (30, 4), &mut input, &mut output).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert_eq!(frames(&output), 2);
        assert!(output.contains("Herdr Yank: copy failed"));
        assert!(input.events.is_empty());
    }
}
