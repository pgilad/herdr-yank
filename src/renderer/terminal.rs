use crate::model::{RenderLine, RenderStyle, Theme, ThemeColor};
use anyhow::Result;
use crossterm::{
    cursor::MoveTo,
    queue,
    style::{
        Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
    },
    terminal::{BeginSynchronizedUpdate, Clear, ClearType, EndSynchronizedUpdate},
};
use std::io::Write;

/// Emits abstract picker render lines to a terminal writer using the default theme.
pub fn emit_render_lines(writer: &mut impl Write, lines: &[RenderLine]) -> Result<()> {
    emit_render_lines_themed(writer, lines, &Theme::default())
}

/// Emits abstract picker render lines to a terminal writer using the configured theme.
///
/// A synchronized update wraps each frame, so a terminal that supports it shows the clear
/// and the redraw as one frame. Other terminals ignore the two sequences.
pub fn emit_render_lines_themed(
    writer: &mut impl Write,
    lines: &[RenderLine],
    theme: &Theme,
) -> Result<()> {
    queue!(writer, BeginSynchronizedUpdate, Clear(ClearType::All))?;

    for (line_index, line) in lines.iter().enumerate() {
        queue!(writer, MoveTo(0, line_index as u16))?;
        for span in &line.spans {
            queue_style(writer, span.style, theme)?;
            queue!(writer, Print(&span.text))?;
        }
    }

    // Cancel any wrap-pending state left by a full-width final row.
    queue!(
        writer,
        MoveTo(0, 0),
        ResetColor,
        SetAttribute(Attribute::Reset),
        EndSynchronizedUpdate
    )?;
    Ok(())
}

impl From<ThemeColor> for Color {
    fn from(color: ThemeColor) -> Self {
        match color {
            ThemeColor::Black => Color::Black,
            ThemeColor::DarkGrey => Color::DarkGrey,
            ThemeColor::Grey => Color::Grey,
            ThemeColor::White => Color::White,
            ThemeColor::Red => Color::Red,
            ThemeColor::DarkRed => Color::DarkRed,
            ThemeColor::Green => Color::Green,
            ThemeColor::DarkGreen => Color::DarkGreen,
            ThemeColor::Yellow => Color::Yellow,
            ThemeColor::DarkYellow => Color::DarkYellow,
            ThemeColor::Blue => Color::Blue,
            ThemeColor::DarkBlue => Color::DarkBlue,
            ThemeColor::Magenta => Color::Magenta,
            ThemeColor::DarkMagenta => Color::DarkMagenta,
            ThemeColor::Cyan => Color::Cyan,
            ThemeColor::DarkCyan => Color::DarkCyan,
            ThemeColor::Ansi(value) => Color::AnsiValue(value),
            ThemeColor::Rgb { r, g, b } => Color::Rgb { r, g, b },
        }
    }
}

fn queue_style(writer: &mut impl Write, style: RenderStyle, theme: &Theme) -> Result<()> {
    match style {
        RenderStyle::Unmatched => queue!(
            writer,
            SetAttribute(Attribute::Reset),
            SetForegroundColor(theme.unmatched_fg.into()),
            SetAttribute(Attribute::Dim)
        )?,
        RenderStyle::Match => queue_text_style(writer, theme.match_fg, theme.match_bg, false)?,
        RenderStyle::Hint => {
            queue_text_style(writer, theme.hint_fg, Some(theme.hint_bg), true)?;
        }
        RenderStyle::SelectedMatch => {
            queue_text_style(
                writer,
                theme.selected_match_fg,
                theme.selected_match_bg,
                false,
            )?;
        }
        RenderStyle::SelectedHint => {
            queue_text_style(
                writer,
                theme.selected_hint_fg,
                Some(theme.selected_hint_bg),
                true,
            )?;
        }
        RenderStyle::Status => {
            queue_text_style(writer, theme.status_fg, Some(theme.status_bg), false)?;
        }
    }
    Ok(())
}

fn queue_text_style(
    writer: &mut impl Write,
    foreground: ThemeColor,
    background: Option<ThemeColor>,
    bold: bool,
) -> Result<()> {
    queue!(
        writer,
        SetAttribute(Attribute::Reset),
        SetForegroundColor(foreground.into())
    )?;
    if let Some(background) = background {
        queue!(writer, SetBackgroundColor(background.into()))?;
    }
    if bold {
        queue!(writer, SetAttribute(Attribute::Bold))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{RenderSpan, RenderStyle};

    #[test]
    fn terminal_emission_clears_screen_and_writes_all_spans() {
        let lines = vec![RenderLine {
            spans: vec![
                RenderSpan {
                    text: "open ".to_string(),
                    style: RenderStyle::Unmatched,
                },
                RenderSpan {
                    text: "a".to_string(),
                    style: RenderStyle::Hint,
                },
                RenderSpan {
                    text: "ttps://example.com".to_string(),
                    style: RenderStyle::Match,
                },
            ],
        }];
        let mut output = Vec::new();

        emit_render_lines(&mut output, &lines).unwrap();
        let output = String::from_utf8(output).unwrap();

        assert!(output.starts_with("\u{1b}[?2026h\u{1b}[2J\u{1b}[1;1H"));
        assert!(output.contains("open "));
        assert!(output.contains("a"));
        assert!(output.contains("ttps://example.com"));
        assert!(output.contains("\u{1b}[38;5;0m"));
        assert!(output.contains("\u{1b}[48;5;14m"));
        assert!(output.contains("\u{1b}[38;5;11m"));
    }

    #[test]
    fn terminal_emission_positions_lines_without_newlines() {
        let lines = vec![
            RenderLine {
                spans: vec![RenderSpan {
                    text: "one".to_string(),
                    style: RenderStyle::Unmatched,
                }],
            },
            RenderLine {
                spans: vec![RenderSpan {
                    text: "two".to_string(),
                    style: RenderStyle::Match,
                }],
            },
        ];
        let mut output = Vec::new();

        emit_render_lines(&mut output, &lines).unwrap();
        let output = String::from_utf8(output).unwrap();

        assert!(output.contains("\u{1b}[1;1H"));
        assert!(output.contains("\u{1b}[2;1H"));
        assert!(!output.contains("\r\n"));
        assert!(output.ends_with("\u{1b}[1;1H\u{1b}[0m\u{1b}[0m\u{1b}[?2026l"));
    }
}
