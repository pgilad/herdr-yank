use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

/// Keyboard input understood by the picker state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PickerInputEvent {
    Char(char),
    Backspace,
    Tab,
    Enter,
    Escape,
    CtrlC,
    /// The terminal changed to (columns, rows).
    Resize(u16, u16),
    Other,
}

/// Converts terminal events into picker-domain input events.
pub(crate) fn input_event_from_crossterm(event: Event) -> PickerInputEvent {
    match event {
        Event::Key(KeyEvent {
            code: KeyCode::Char('c'),
            modifiers,
            ..
        }) if modifiers.contains(KeyModifiers::CONTROL) => PickerInputEvent::CtrlC,
        // Terminals send Ctrl-I as a literal tab, but some emit it as a control char.
        Event::Key(KeyEvent {
            code: KeyCode::Char('i'),
            modifiers,
            ..
        }) if modifiers.contains(KeyModifiers::CONTROL) => PickerInputEvent::Tab,
        Event::Key(KeyEvent {
            code: KeyCode::Tab | KeyCode::BackTab,
            ..
        }) => PickerInputEvent::Tab,
        Event::Key(KeyEvent {
            code: KeyCode::Backspace,
            ..
        }) => PickerInputEvent::Backspace,
        Event::Key(KeyEvent {
            code: KeyCode::Char(ch),
            modifiers,
            ..
        }) if !modifiers.contains(KeyModifiers::CONTROL)
            && !modifiers.contains(KeyModifiers::ALT) =>
        {
            PickerInputEvent::Char(ch)
        }
        Event::Key(KeyEvent {
            code: KeyCode::Enter,
            ..
        }) => PickerInputEvent::Enter,
        Event::Key(KeyEvent {
            code: KeyCode::Esc, ..
        }) => PickerInputEvent::Escape,
        Event::Resize(columns, rows) => PickerInputEvent::Resize(columns, rows),
        _ => PickerInputEvent::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_crossterm_keys_to_picker_events() {
        assert_eq!(
            input_event_from_crossterm(Event::Key(KeyEvent::new(
                KeyCode::Char('a'),
                KeyModifiers::empty()
            ))),
            PickerInputEvent::Char('a')
        );
        assert_eq!(
            input_event_from_crossterm(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::empty()
            ))),
            PickerInputEvent::Enter
        );
        assert_eq!(
            input_event_from_crossterm(Event::Key(KeyEvent::new(
                KeyCode::Esc,
                KeyModifiers::empty()
            ))),
            PickerInputEvent::Escape
        );
        assert_eq!(
            input_event_from_crossterm(Event::Key(KeyEvent::new(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL
            ))),
            PickerInputEvent::CtrlC
        );
    }

    #[test]
    fn maps_resize_events_to_the_new_size() {
        assert_eq!(
            input_event_from_crossterm(Event::Resize(120, 40)),
            PickerInputEvent::Resize(120, 40)
        );
    }

    #[test]
    fn maps_tab_and_backspace_for_multi_select_and_editing() {
        for event in [
            Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty())),
            Event::Key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::empty())),
            Event::Key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::CONTROL)),
        ] {
            assert_eq!(input_event_from_crossterm(event), PickerInputEvent::Tab);
        }

        assert_eq!(
            input_event_from_crossterm(Event::Key(KeyEvent::new(
                KeyCode::Backspace,
                KeyModifiers::empty()
            ))),
            PickerInputEvent::Backspace
        );
    }
}
