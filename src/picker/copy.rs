use crate::clipboard::{Clipboard, ClipboardError};
use anyhow::{Context, Result};

/// Copies a selected picker match and attaches user-facing context on failure.
pub(crate) fn copy_selected_text(clipboard: &impl Clipboard, text: &str) -> Result<()> {
    clipboard
        .copy(text)
        .map_err(|error| copy_error(text, error))
}

/// Reports a copy failure by size instead of by content.
///
/// The message goes to a Herdr notification and to stderr, and the selection is usually the
/// secret the user meant to keep, so the text itself never enters the message.
fn copy_error(text: &str, error: ClipboardError) -> anyhow::Error {
    let bytes = text.len();
    Err::<(), ClipboardError>(error)
        .with_context(|| format!("failed to copy selected text ({bytes} bytes) to clipboard"))
        .unwrap_err()
}
