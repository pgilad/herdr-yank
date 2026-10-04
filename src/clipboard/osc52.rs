use super::error::ClipboardError;
use base64::Engine;
use std::io::Write;

/// Largest selection that Herdr forwards as one OSC 52 clipboard write.
///
/// Herdr 0.7 drops an OSC 52 body above 256 KiB, and Herdr 0.8 or later drops decoded text
/// above 192 KiB. With the `52;c;` prefix and the base64 padding, 196,602 bytes of text is the
/// largest selection that both accept. A longer selection is an error, not a partial copy.
pub(crate) const MAX_OSC52_TEXT_BYTES: usize = 196_602;

/// Writes an OSC 52 clipboard escape that Herdr forwards to the foreground client.
pub(crate) fn write_osc52(writer: &mut impl Write, text: &str) -> Result<(), ClipboardError> {
    if text.len() > MAX_OSC52_TEXT_BYTES {
        return Err(ClipboardError::Osc52TooLarge {
            bytes: text.len(),
            limit: MAX_OSC52_TEXT_BYTES,
        });
    }

    let encoded = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    write!(writer, "\x1b]52;c;{encoded}\x07").map_err(|error| ClipboardError::WriteFailed {
        tool: "osc52".to_string(),
        message: error.to_string(),
    })?;
    writer.flush().map_err(|error| ClipboardError::WriteFailed {
        tool: "osc52".to_string(),
        message: error.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HERDR_07_BODY_LIMIT: usize = 256 * 1024;
    const HERDR_08_TEXT_LIMIT: usize = 192 * 1024;

    fn body_len(text_bytes: usize) -> usize {
        "52;c;".len() + text_bytes.div_ceil(3) * 4
    }

    #[test]
    fn writes_a_bel_terminated_clipboard_sequence() {
        let mut output = Vec::new();

        write_osc52(&mut output, "hello").unwrap();

        assert_eq!(output, b"\x1b]52;c;aGVsbG8=\x07");
    }

    #[test]
    fn the_limit_is_the_largest_text_that_every_herdr_version_accepts() {
        const { assert!(MAX_OSC52_TEXT_BYTES <= HERDR_08_TEXT_LIMIT) };
        assert!(body_len(MAX_OSC52_TEXT_BYTES) <= HERDR_07_BODY_LIMIT);
        assert!(body_len(MAX_OSC52_TEXT_BYTES + 1) > HERDR_07_BODY_LIMIT);
    }

    #[test]
    fn accepts_the_limit_and_rejects_one_byte_more() {
        let mut output = Vec::new();
        write_osc52(&mut output, &"a".repeat(MAX_OSC52_TEXT_BYTES)).unwrap();
        assert_eq!(output.len(), 2 + body_len(MAX_OSC52_TEXT_BYTES) + 1);

        let mut output = Vec::new();
        let error = write_osc52(&mut output, &"a".repeat(MAX_OSC52_TEXT_BYTES + 1)).unwrap_err();

        assert_eq!(
            error,
            ClipboardError::Osc52TooLarge {
                bytes: MAX_OSC52_TEXT_BYTES + 1,
                limit: MAX_OSC52_TEXT_BYTES,
            }
        );
        assert!(output.is_empty());
    }
}
