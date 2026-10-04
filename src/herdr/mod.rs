pub mod client;
pub mod context;
pub mod executor;
pub mod layout;
mod protocol;
pub mod snapshot;
mod socket;

use crate::config::load_config;
use crate::herdr::client::{HerdrClient, SocketHerdrClient};
use crate::herdr::context::HerdrContext;
use crate::herdr::executor::{
    cleanup_session, launch_layout_tab_picker, run_snapshot_picker, zoom_picker, LaunchRequest,
};
use crate::herdr::snapshot::{take_snapshot_file, wait_for_ready, PickerLaunchFiles};
use crate::model::{PaneId, PickerAction};
use anyhow::{Context, Result};
use crossterm::{cursor, execute, terminal};
use std::io::{stdout, Write};
use std::path::Path;
use std::time::{Duration, Instant};

/// Narrow production adapter for Herdr layout launch and picker cleanup.
#[derive(Debug, Clone)]
pub struct HerdrAdapter {
    context: HerdrContext,
}

impl HerdrAdapter {
    pub fn from_env() -> Self {
        Self {
            context: HerdrContext::from_env(),
        }
    }

    pub fn target_pane_from_context(&self) -> Option<PaneId> {
        self.context.target_pane()
    }

    /**
     * Opens the picker that copies any supported visible token.
     */
    pub fn open_copy_picker(&self, target: &PaneId) -> Result<()> {
        self.open_picker(target, PickerAction::Copy)
    }

    /**
     * Opens the picker that launches visible web URLs and local files.
     */
    pub fn open_url_picker(&self, target: &PaneId) -> Result<()> {
        self.open_picker(target, PickerAction::OpenUrl)
    }

    fn open_picker(&self, target: &PaneId, action: PickerAction) -> Result<()> {
        let binary = std::env::current_exe().context("failed to locate herdr-yank binary")?;
        let mut client = SocketHerdrClient::from_context(&self.context)?;
        let details = client.pane_details(target)?;
        // Project patterns apply only to the copy action.
        let cwd = match action {
            PickerAction::Copy => details.cwd.as_deref(),
            PickerAction::OpenUrl => None,
        };
        let config = load_config(cwd);
        report_config_problems(&mut client, &config.problems);

        launch_layout_tab_picker(
            &mut client,
            LaunchRequest {
                target,
                viewport_rows: details.viewport_rows,
                binary_path: &binary,
                action,
                custom_patterns: match action {
                    PickerAction::Copy => config.patterns,
                    PickerAction::OpenUrl => Vec::new(),
                },
                settings: config.settings,
            },
        )
    }

    /// Waits for layout completion, runs the picker, and always cleans up explicit resources.
    ///
    /// Closing the picker tab can end this process, because the picker runs in that tab.
    /// Thus every other step, such as the file cleanup and the failure report, runs before it.
    pub fn run_picker_from_snapshot(&self, snapshot_path: &Path, ready_path: &Path) -> Result<()> {
        let files = PickerLaunchFiles::for_paths(snapshot_path, ready_path);
        let temp_tab = self
            .context
            .tab_id
            .clone()
            .context("picker process is missing HERDR_TAB_ID")?;
        let pane = self
            .context
            .pane_id
            .clone()
            .map(PaneId::new)
            .context("picker process is missing HERDR_PANE_ID")?;
        let mut client = SocketHerdrClient::from_context(&self.context)?;

        let snapshot = match take_snapshot_file(snapshot_path) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                // Without the snapshot the source tab is unknown, so only the picker tab closes.
                report_picker_failure(&mut client, &error, show_failure_on_pane);
                let _ = files.cleanup();
                if let Err(close) = client.close_tab(&temp_tab) {
                    eprintln!("closing the picker tab also failed: {close:#}");
                }
                return Err(error);
            }
        };

        let primary = wait_for_ready(ready_path, Duration::from_secs(10))
            .and_then(|_| zoom_picker(&mut client, &snapshot, &pane))
            .and_then(|_| {
                if snapshot.session.zoom_picker {
                    wait_for_terminal_rows(snapshot.source.content_rows, Duration::from_secs(2))
                } else {
                    Ok(())
                }
            })
            .and_then(|_| run_snapshot_picker(&snapshot));
        if let Err(error) = &primary {
            report_picker_failure(&mut client, error, show_failure_on_pane);
        }
        let files_cleanup = files.cleanup();
        let cleanup = cleanup_session(&mut client, &snapshot.session, &temp_tab);
        match primary {
            Err(e) => {
                if let Err(c) = cleanup {
                    eprintln!("cleanup also failed: {c:#}");
                }
                if let Err(c) = files_cleanup {
                    eprintln!("file cleanup also failed: {c:#}");
                }
                Err(e)
            }
            Ok(()) => {
                cleanup?;
                files_cleanup?;
                Ok(())
            }
        }
    }
}

/// Shows a picker failure to the user before the picker tab closes.
///
/// Herdr does not log the output of layout panes, and the picker tab closes right after a
/// failure. Thus a notification is the main channel. When Herdr does not show it, the message
/// stays on the picker pane until the user pushes a key.
fn report_picker_failure<C: HerdrClient>(
    client: &mut C,
    error: &anyhow::Error,
    show_on_pane: impl FnOnce(&str),
) {
    eprintln!("Herdr Yank: {error:#}");
    let title = format!("Herdr Yank: {error}");
    let cause = error
        .chain()
        .skip(1)
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(": ");
    let body = (!cause.is_empty()).then_some(cause.as_str());
    let shown = client
        .show_notification(&title, body)
        .is_ok_and(|outcome| outcome.shown);
    if !shown {
        show_on_pane(&format!("Herdr Yank: {error:#}"));
    }
}

fn show_failure_on_pane(message: &str) {
    if let Err(error) = crate::picker::show_failure_and_wait(message) {
        eprintln!("Herdr Yank: could not show the failure on the picker pane: {error:#}");
    }
}

/// Shows config problems, because each one makes Herdr Yank ignore a key or an entry.
///
/// The notification body holds the first problem. The plugin log holds all of them.
fn report_config_problems<C: HerdrClient>(client: &mut C, problems: &[String]) {
    let Some(first) = problems.first() else {
        return;
    };
    for problem in problems {
        eprintln!("Herdr Yank: config: {problem}");
    }

    let (title, body) = match problems.len() {
        1 => ("Herdr Yank: config problem".to_string(), first.clone()),
        count => (
            format!("Herdr Yank: {count} config problems"),
            format!("{first} (all problems are in herdr plugin log)"),
        ),
    };
    match client.show_notification(&title, Some(&body)) {
        Ok(outcome) if !outcome.shown => {
            eprintln!(
                "Herdr Yank: config notification suppressed ({})",
                outcome.reason
            );
        }
        Ok(_) => {}
        Err(error) => eprintln!("Herdr Yank: config notification failed: {error:#}"),
    }
}

/**
 * Waits for Herdr to apply the asynchronous zoom resize to the picker PTY.
 *
 * A different final size is not an error, because the picker fits its output to the real
 * terminal size. For example, a tab bar that appears for the picker tab removes one row.
 */
fn wait_for_terminal_rows(rows: u16, timeout: Duration) -> Result<()> {
    let started = Instant::now();

    loop {
        let (_, current_rows) = terminal::size().context("failed to read picker terminal size")?;

        if current_rows == rows || started.elapsed() >= timeout {
            return Ok(());
        }

        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Clears an inert pane and remains alive until Herdr closes its tab.
pub fn run_idle() -> Result<()> {
    let mut out = stdout();
    execute!(
        out,
        terminal::Clear(terminal::ClearType::All),
        cursor::MoveTo(0, 0)
    )?;
    out.flush()?;
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::herdr::client::{AppliedLayout, LaunchLayoutNode, NotificationOutcome, PaneDetails};
    use crate::herdr::layout::{ExportedLayout, LayoutSnapshot};
    use anyhow::anyhow;
    use std::cell::RefCell;

    /// Records notifications; the picker flow under test calls no other Herdr method.
    #[derive(Default)]
    struct NotifyingClient {
        shown: bool,
        fail: bool,
        notifications: Vec<(String, Option<String>)>,
    }

    impl HerdrClient for NotifyingClient {
        fn pane_layout(&mut self, _pane: &PaneId) -> Result<LayoutSnapshot> {
            unreachable!("not used by failure reporting")
        }

        fn layout_export(&mut self, _pane: &PaneId) -> Result<ExportedLayout> {
            unreachable!("not used by failure reporting")
        }

        fn pane_details(&mut self, _pane: &PaneId) -> Result<PaneDetails> {
            unreachable!("not used by failure reporting")
        }

        fn pane_read_visible(&mut self, _pane: &PaneId, _lines: u16) -> Result<String> {
            unreachable!("not used by failure reporting")
        }

        fn apply_layout(
            &mut self,
            _workspace_id: &str,
            _tab_label: &str,
            _root: &LaunchLayoutNode,
        ) -> Result<AppliedLayout> {
            unreachable!("not used by failure reporting")
        }

        fn focus_pane(&mut self, _pane: &PaneId) -> Result<()> {
            unreachable!("not used by failure reporting")
        }

        fn zoom_pane(&mut self, _pane: &PaneId) -> Result<()> {
            unreachable!("not used by failure reporting")
        }

        fn focus_tab(&mut self, _tab_id: &str) -> Result<()> {
            unreachable!("not used by failure reporting")
        }

        fn close_tab(&mut self, _tab_id: &str) -> Result<()> {
            unreachable!("not used by failure reporting")
        }

        fn show_notification(
            &mut self,
            title: &str,
            body: Option<&str>,
        ) -> Result<NotificationOutcome> {
            self.notifications
                .push((title.to_string(), body.map(str::to_string)));
            if self.fail {
                return Err(anyhow!("socket closed"));
            }
            Ok(NotificationOutcome {
                shown: self.shown,
                reason: "disabled".into(),
            })
        }
    }

    fn copy_failure() -> anyhow::Error {
        anyhow!("no supported clipboard tool found")
            .context("failed to copy selected text (12 bytes) to clipboard")
    }

    #[test]
    fn a_shown_notification_carries_the_failure_and_its_cause() {
        let mut client = NotifyingClient {
            shown: true,
            ..NotifyingClient::default()
        };
        let on_pane = RefCell::new(None);

        report_picker_failure(&mut client, &copy_failure(), |message| {
            *on_pane.borrow_mut() = Some(message.to_string());
        });

        assert_eq!(
            client.notifications,
            [(
                "Herdr Yank: failed to copy selected text (12 bytes) to clipboard".to_string(),
                Some("no supported clipboard tool found".to_string())
            )]
        );
        assert_eq!(*on_pane.borrow(), None);
    }

    #[test]
    fn an_unseen_notification_puts_the_failure_on_the_pane() {
        for fail in [false, true] {
            let mut client = NotifyingClient {
                shown: false,
                fail,
                ..NotifyingClient::default()
            };
            let on_pane = RefCell::new(None);

            report_picker_failure(&mut client, &copy_failure(), |message| {
                *on_pane.borrow_mut() = Some(message.to_string());
            });

            assert_eq!(
                on_pane.borrow().as_deref(),
                Some("Herdr Yank: failed to copy selected text (12 bytes) to clipboard: no supported clipboard tool found")
            );
        }
    }

    #[test]
    fn a_failure_without_a_cause_sends_no_body() {
        let mut client = NotifyingClient {
            shown: true,
            ..NotifyingClient::default()
        };

        report_picker_failure(&mut client, &anyhow!("timed out"), |_| {});

        assert_eq!(
            client.notifications,
            [("Herdr Yank: timed out".to_string(), None)]
        );
    }

    #[test]
    fn config_problems_are_summarized_in_one_notification() {
        let mut client = NotifyingClient::default();

        report_config_problems(&mut client, &[]);
        assert!(client.notifications.is_empty());

        report_config_problems(
            &mut client,
            &["config.toml: unknown key `clipbaord`".into()],
        );
        report_config_problems(
            &mut client,
            &[
                "config.toml: unknown key `clipbaord`".into(),
                "config.toml: `style.hint_bg`: \"pink\" is not a known color".into(),
            ],
        );

        assert_eq!(
            client.notifications,
            [
                (
                    "Herdr Yank: config problem".to_string(),
                    Some("config.toml: unknown key `clipbaord`".to_string())
                ),
                (
                    "Herdr Yank: 2 config problems".to_string(),
                    Some(
                        "config.toml: unknown key `clipbaord` (all problems are in herdr plugin log)"
                            .to_string()
                    )
                ),
            ]
        );
    }
}
