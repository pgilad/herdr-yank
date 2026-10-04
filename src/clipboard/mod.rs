mod detect;
mod error;
mod osc52;
mod runner;
mod system;
mod tool;

pub use error::ClipboardError;

use crate::model::ClipboardMode;
use detect::select_tool;
use osc52::write_osc52;
use runner::ClipboardCommandRunner;
use system::SystemCommandRunner;
use tool::ClipboardEnvironment;

/// Clipboard abstraction used by picker code and tests.
pub trait Clipboard {
    fn copy(&self, text: &str) -> Result<(), ClipboardError>;
}

/// System clipboard implementation that uses OSC 52 or a platform clipboard command.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClipboard {
    mode: ClipboardMode,
}

impl SystemClipboard {
    pub fn new(mode: ClipboardMode) -> Self {
        Self { mode }
    }
}

impl Clipboard for SystemClipboard {
    fn copy(&self, text: &str) -> Result<(), ClipboardError> {
        copy_with_runner(
            text,
            &SystemCommandRunner,
            ClipboardEnvironment::current(),
            self.mode,
            write_osc52_to_terminal,
        )
    }
}

/// Writes the OSC 52 escape straight to the controlling terminal.
///
/// The picker owns stdout for its own rendering, so the escape goes to `/dev/tty` to avoid
/// interleaving with queued render output.
fn write_osc52_to_terminal(text: &str) -> Result<(), ClipboardError> {
    let mut terminal = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/tty")
        .map_err(|error| ClipboardError::Osc52Unavailable {
            message: error.to_string(),
        })?;
    write_osc52(&mut terminal, text)
}

/// Copies text with the method that `mode` selects.
///
/// `auto` prefers OSC 52: Herdr sends it to the clipboard of the attached client, which is on
/// the computer of the user also when the Herdr server is remote. A clipboard command runs on
/// the server computer, so it is only the fallback.
fn copy_with_runner(
    text: &str,
    runner: &impl ClipboardCommandRunner,
    env: ClipboardEnvironment,
    mode: ClipboardMode,
    osc52: impl Fn(&str) -> Result<(), ClipboardError>,
) -> Result<(), ClipboardError> {
    match mode {
        ClipboardMode::Osc52 => osc52(text),
        ClipboardMode::Tool => {
            let (selected, candidates) = select_tool(runner, env);
            let tool = selected.ok_or_else(|| ClipboardError::no_tool_found(&candidates))?;
            runner.run_with_stdin(tool, text)
        }
        ClipboardMode::Auto => {
            let Err(osc52_error) = osc52(text) else {
                return Ok(());
            };
            let (selected, candidates) = select_tool(runner, env);
            let Some(tool) = selected else {
                return Err(ClipboardError::no_clipboard(&osc52_error, &candidates));
            };
            runner.run_with_stdin(tool, text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::tool::{ClipboardOs, ClipboardTool};
    use std::cell::RefCell;
    use std::collections::HashSet;

    #[derive(Default)]
    struct FakeRunner {
        available: HashSet<&'static str>,
        runs: RefCell<Vec<(&'static str, Vec<&'static str>, String)>>,
        failure: Option<ClipboardError>,
    }

    impl ClipboardCommandRunner for FakeRunner {
        fn command_exists(&self, command: &str) -> bool {
            self.available.contains(command)
        }

        fn run_with_stdin(&self, tool: ClipboardTool, stdin: &str) -> Result<(), ClipboardError> {
            self.runs
                .borrow_mut()
                .push((tool.name, tool.args.to_vec(), stdin.to_string()));
            if let Some(error) = &self.failure {
                Err(error.clone())
            } else {
                Ok(())
            }
        }
    }

    fn env(os: ClipboardOs, wayland: bool, x11: bool) -> ClipboardEnvironment {
        ClipboardEnvironment { os, wayland, x11 }
    }

    /// Records OSC 52 writes so tests can assert which path handled the copy.
    #[derive(Default)]
    struct FakeOsc52 {
        written: RefCell<Vec<String>>,
        error: Option<ClipboardError>,
    }

    impl FakeOsc52 {
        fn unavailable() -> Self {
            Self {
                error: Some(ClipboardError::Osc52Unavailable {
                    message: "no tty".to_string(),
                }),
                ..Self::default()
            }
        }

        fn writer(&self) -> impl Fn(&str) -> Result<(), ClipboardError> + '_ {
            move |text: &str| {
                self.written.borrow_mut().push(text.to_string());
                self.error.clone().map_or(Ok(()), Err)
            }
        }
    }

    /// Copies in `tool` mode, where OSC 52 must never run.
    fn copy_with_tools(
        text: &str,
        runner: &impl ClipboardCommandRunner,
        env: ClipboardEnvironment,
    ) -> Result<(), ClipboardError> {
        copy_with_runner(text, runner, env, ClipboardMode::Tool, |_| {
            panic!("unexpected OSC 52 write")
        })
    }

    #[test]
    fn auto_mode_uses_osc52_first_and_runs_no_tool() {
        let runner = FakeRunner {
            available: HashSet::from(["pbcopy"]),
            ..FakeRunner::default()
        };
        let osc52 = FakeOsc52::default();

        copy_with_runner(
            "token",
            &runner,
            env(ClipboardOs::Macos, false, false),
            ClipboardMode::Auto,
            osc52.writer(),
        )
        .unwrap();

        assert_eq!(osc52.written.borrow().as_slice(), &["token".to_string()]);
        assert!(runner.runs.borrow().is_empty());
    }

    #[test]
    fn auto_mode_falls_back_to_a_tool_when_osc52_fails() {
        let runner = FakeRunner {
            available: HashSet::from(["pbcopy"]),
            ..FakeRunner::default()
        };
        let osc52 = FakeOsc52 {
            error: Some(ClipboardError::Osc52TooLarge {
                bytes: 300_000,
                limit: osc52::MAX_OSC52_TEXT_BYTES,
            }),
            ..FakeOsc52::default()
        };

        copy_with_runner(
            "token",
            &runner,
            env(ClipboardOs::Macos, false, false),
            ClipboardMode::Auto,
            osc52.writer(),
        )
        .unwrap();

        assert_eq!(osc52.written.borrow().len(), 1);
        assert_eq!(
            runner.runs.borrow().as_slice(),
            &[("pbcopy", Vec::new(), "token".to_string())]
        );
    }

    #[test]
    fn auto_mode_reports_both_failures_when_nothing_can_copy() {
        let runner = FakeRunner::default();
        let osc52 = FakeOsc52::unavailable();

        let error = copy_with_runner(
            "token",
            &runner,
            env(ClipboardOs::Other, false, false),
            ClipboardMode::Auto,
            osc52.writer(),
        )
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "OSC 52 failed (the terminal is unavailable for OSC 52: no tty) and no clipboard tool was found (tried pbcopy, wl-copy, xclip, xsel)"
        );
    }

    #[test]
    fn copies_with_pbcopy_on_macos() {
        let runner = FakeRunner {
            available: HashSet::from(["pbcopy"]),
            ..FakeRunner::default()
        };

        copy_with_tools(
            "https://example.com",
            &runner,
            env(ClipboardOs::Macos, false, false),
        )
        .unwrap();

        assert_eq!(
            runner.runs.borrow().as_slice(),
            &[("pbcopy", Vec::new(), "https://example.com".to_string())]
        );
    }

    #[test]
    fn copies_with_wayland_tool_when_available() {
        let runner = FakeRunner {
            available: HashSet::from(["wl-copy", "xclip"]),
            ..FakeRunner::default()
        };

        copy_with_tools("token", &runner, env(ClipboardOs::Other, true, true)).unwrap();

        assert_eq!(runner.runs.borrow()[0].0, "wl-copy");
    }

    #[test]
    fn copies_with_xclip_arguments_on_x11() {
        let runner = FakeRunner {
            available: HashSet::from(["xclip"]),
            ..FakeRunner::default()
        };

        copy_with_tools("/tmp/file", &runner, env(ClipboardOs::Other, false, true)).unwrap();

        assert_eq!(
            runner.runs.borrow().as_slice(),
            &[(
                "xclip",
                vec!["-selection", "clipboard"],
                "/tmp/file".to_string()
            )]
        );
    }

    #[test]
    fn copies_with_xsel_arguments_when_xclip_missing() {
        let runner = FakeRunner {
            available: HashSet::from(["xsel"]),
            ..FakeRunner::default()
        };

        copy_with_tools("abcdef1", &runner, env(ClipboardOs::Other, false, true)).unwrap();

        assert_eq!(
            runner.runs.borrow().as_slice(),
            &[(
                "xsel",
                vec!["--clipboard", "--input"],
                "abcdef1".to_string()
            )]
        );
    }

    #[test]
    fn reports_no_supported_tool_with_tried_list_in_tool_mode() {
        let runner = FakeRunner::default();

        let error =
            copy_with_tools("unused", &runner, env(ClipboardOs::Other, false, false)).unwrap_err();

        assert_eq!(
            error,
            ClipboardError::NoToolFound {
                tried: "pbcopy, wl-copy, xclip, xsel".to_string()
            }
        );
        assert!(runner.runs.borrow().is_empty());
    }

    #[test]
    fn osc52_mode_skips_tool_detection_entirely() {
        let runner = FakeRunner {
            available: HashSet::from(["pbcopy"]),
            ..FakeRunner::default()
        };
        let osc52 = FakeOsc52::unavailable();

        let error = copy_with_runner(
            "token",
            &runner,
            env(ClipboardOs::Macos, false, false),
            ClipboardMode::Osc52,
            osc52.writer(),
        )
        .unwrap_err();

        assert!(matches!(error, ClipboardError::Osc52Unavailable { .. }));
        assert!(runner.runs.borrow().is_empty());
    }

    #[test]
    fn surfaces_command_execution_failure() {
        let runner = FakeRunner {
            available: HashSet::from(["wl-copy"]),
            failure: Some(ClipboardError::CommandFailed {
                tool: "wl-copy".to_string(),
                status: "exit status: 1".to_string(),
            }),
            ..FakeRunner::default()
        };

        let error =
            copy_with_tools("token", &runner, env(ClipboardOs::Other, true, false)).unwrap_err();

        assert_eq!(
            error,
            ClipboardError::CommandFailed {
                tool: "wl-copy".to_string(),
                status: "exit status: 1".to_string(),
            }
        );
        assert_eq!(runner.runs.borrow()[0].2, "token");
    }
}
