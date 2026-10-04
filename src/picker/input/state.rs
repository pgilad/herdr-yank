use super::PickerInputEvent;

/// Result of feeding one key into the fixed-width hint state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InputDecision {
    Continue,
    Cancel,
    CopyHint(String),
    InvalidHint,
    /// Visible state changed; the caller must redraw before reading the next key.
    Redraw,
    /// Multi-select finished with at least one hint chosen, in selection order.
    CommitSelection(Vec<String>),
}

/// Pure fixed-width hint buffer for picker input, with optional multi-select.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InputState {
    width: usize,
    buffer: String,
    multi_select_enabled: bool,
    multi: bool,
    selected: Vec<String>,
}

impl InputState {
    pub(crate) fn new(width: usize, multi_select_enabled: bool) -> Self {
        Self {
            width,
            buffer: String::new(),
            multi_select_enabled,
            multi: false,
            selected: Vec::new(),
        }
    }

    /// Whether multi-select mode is currently active.
    pub(crate) fn is_multi(&self) -> bool {
        self.multi
    }

    /// Hints chosen so far, in selection order.
    pub(crate) fn selected(&self) -> &[String] {
        &self.selected
    }

    /// Hint characters typed but not yet resolved into a full-width hint.
    pub(crate) fn typed(&self) -> &str {
        &self.buffer
    }

    pub(crate) fn push(&mut self, event: PickerInputEvent, valid_hints: &[&str]) -> InputDecision {
        match event {
            PickerInputEvent::Escape | PickerInputEvent::CtrlC => InputDecision::Cancel,
            PickerInputEvent::Other | PickerInputEvent::Resize(..) => InputDecision::Continue,
            PickerInputEvent::Enter => self.commit_selection(),
            PickerInputEvent::Tab => self.toggle_multi_mode(),
            PickerInputEvent::Backspace => self.backspace(),
            PickerInputEvent::Char(ch) => self.push_char(ch, valid_hints),
        }
    }

    fn toggle_multi_mode(&mut self) -> InputDecision {
        if !self.multi_select_enabled {
            return InputDecision::Continue;
        }

        if !self.multi {
            self.multi = true;
            self.buffer.clear();
            return InputDecision::Redraw;
        }

        self.commit_selection()
    }

    /// Finishes multi-select, or does nothing when there is nothing to commit.
    fn commit_selection(&mut self) -> InputDecision {
        if !self.multi || self.selected.is_empty() {
            return InputDecision::Continue;
        }
        InputDecision::CommitSelection(std::mem::take(&mut self.selected))
    }

    fn backspace(&mut self) -> InputDecision {
        if self.buffer.pop().is_none() {
            return InputDecision::Continue;
        }
        InputDecision::Redraw
    }

    fn push_char(&mut self, ch: char, valid_hints: &[&str]) -> InputDecision {
        if self.width == 0 {
            return InputDecision::Continue;
        }

        self.buffer.push(ch);
        if self.buffer.chars().count() < self.width {
            return InputDecision::Redraw;
        }

        let entered = std::mem::take(&mut self.buffer);
        if !valid_hints.iter().any(|hint| *hint == entered) {
            return InputDecision::InvalidHint;
        }

        if !self.multi {
            return InputDecision::CopyHint(entered);
        }

        self.toggle_selected(entered);
        InputDecision::Redraw
    }

    fn toggle_selected(&mut self, hint: String) {
        if let Some(index) = self.selected.iter().position(|chosen| chosen == &hint) {
            self.selected.remove(index);
        } else {
            self.selected.push(hint);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn single(width: usize) -> InputState {
        InputState::new(width, true)
    }

    #[test]
    fn exact_one_character_hint_requests_copy() {
        let mut state = single(1);

        assert_eq!(
            state.push(PickerInputEvent::Char('a'), &["a"]),
            InputDecision::CopyHint("a".to_string())
        );
    }

    #[test]
    fn exact_two_character_hint_waits_for_full_width() {
        let mut state = single(2);

        assert_eq!(
            state.push(PickerInputEvent::Char('a'), &["as"]),
            InputDecision::Redraw
        );
        assert_eq!(
            state.push(PickerInputEvent::Char('s'), &["as"]),
            InputDecision::CopyHint("as".to_string())
        );
    }

    #[test]
    fn invalid_full_width_hint_clears_buffer() {
        let mut state = single(2);

        assert_eq!(
            state.push(PickerInputEvent::Char('a'), &["sd"]),
            InputDecision::Redraw
        );
        assert_eq!(
            state.push(PickerInputEvent::Char('x'), &["sd"]),
            InputDecision::InvalidHint
        );
        assert_eq!(
            state.push(PickerInputEvent::Char('s'), &["sd"]),
            InputDecision::Redraw
        );
        assert_eq!(
            state.push(PickerInputEvent::Char('d'), &["sd"]),
            InputDecision::CopyHint("sd".to_string())
        );
    }

    #[test]
    fn backspace_undoes_a_mistyped_hint_prefix() {
        let mut state = single(2);

        state.push(PickerInputEvent::Char('x'), &["as"]);
        assert_eq!(state.typed(), "x");
        assert_eq!(
            state.push(PickerInputEvent::Backspace, &["as"]),
            InputDecision::Redraw
        );
        assert_eq!(state.typed(), "");

        state.push(PickerInputEvent::Char('a'), &["as"]);
        assert_eq!(
            state.push(PickerInputEvent::Char('s'), &["as"]),
            InputDecision::CopyHint("as".to_string())
        );
    }

    #[test]
    fn backspace_on_an_empty_buffer_does_nothing() {
        let mut state = single(1);

        assert_eq!(
            state.push(PickerInputEvent::Backspace, &["a"]),
            InputDecision::Continue
        );
    }

    #[test]
    fn escape_and_ctrl_c_cancel() {
        let mut state = single(1);
        assert_eq!(
            state.push(PickerInputEvent::Escape, &[]),
            InputDecision::Cancel
        );
        assert_eq!(
            state.push(PickerInputEvent::CtrlC, &[]),
            InputDecision::Cancel
        );
    }

    #[test]
    fn enter_is_ignored_outside_multi_select() {
        let mut state = single(1);
        assert_eq!(
            state.push(PickerInputEvent::Enter, &["a"]),
            InputDecision::Continue
        );
    }

    #[test]
    fn tab_enters_multi_mode_and_hints_toggle_instead_of_copying() {
        let mut state = single(1);
        let hints = ["a", "s", "d"];

        assert_eq!(
            state.push(PickerInputEvent::Tab, &hints),
            InputDecision::Redraw
        );
        assert!(state.is_multi());

        assert_eq!(
            state.push(PickerInputEvent::Char('a'), &hints),
            InputDecision::Redraw
        );
        assert_eq!(
            state.push(PickerInputEvent::Char('d'), &hints),
            InputDecision::Redraw
        );
        assert_eq!(state.selected(), ["a".to_string(), "d".to_string()]);
    }

    #[test]
    fn selecting_the_same_hint_twice_removes_it() {
        let mut state = single(1);
        let hints = ["a", "s"];

        state.push(PickerInputEvent::Tab, &hints);
        state.push(PickerInputEvent::Char('a'), &hints);
        state.push(PickerInputEvent::Char('s'), &hints);
        state.push(PickerInputEvent::Char('a'), &hints);

        assert_eq!(state.selected(), ["s".to_string()]);
    }

    #[test]
    fn second_tab_commits_the_selection_in_order() {
        let mut state = single(1);
        let hints = ["a", "s"];

        state.push(PickerInputEvent::Tab, &hints);
        state.push(PickerInputEvent::Char('s'), &hints);
        state.push(PickerInputEvent::Char('a'), &hints);

        assert_eq!(
            state.push(PickerInputEvent::Tab, &hints),
            InputDecision::CommitSelection(vec!["s".to_string(), "a".to_string()])
        );
    }

    #[test]
    fn enter_commits_the_selection_in_multi_mode() {
        let mut state = single(1);
        let hints = ["a"];

        state.push(PickerInputEvent::Tab, &hints);
        state.push(PickerInputEvent::Char('a'), &hints);

        assert_eq!(
            state.push(PickerInputEvent::Enter, &hints),
            InputDecision::CommitSelection(vec!["a".to_string()])
        );
    }

    #[test]
    fn committing_an_empty_selection_keeps_the_picker_open() {
        let mut state = single(1);
        let hints = ["a"];

        state.push(PickerInputEvent::Tab, &hints);

        assert_eq!(
            state.push(PickerInputEvent::Tab, &hints),
            InputDecision::Continue
        );
        assert!(state.is_multi());
    }

    #[test]
    fn tab_is_inert_when_multi_select_is_disabled() {
        let mut state = InputState::new(1, false);

        assert_eq!(
            state.push(PickerInputEvent::Tab, &["a"]),
            InputDecision::Continue
        );
        assert!(!state.is_multi());
        assert_eq!(
            state.push(PickerInputEvent::Char('a'), &["a"]),
            InputDecision::CopyHint("a".to_string())
        );
    }
}
