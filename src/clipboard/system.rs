use super::error::ClipboardError;
use super::runner::ClipboardCommandRunner;
use super::tool::ClipboardTool;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

/// `std::process`-backed clipboard command runner for normal operation.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct SystemCommandRunner;

impl ClipboardCommandRunner for SystemCommandRunner {
    fn command_exists(&self, command: &str) -> bool {
        which::which(command).is_ok()
    }

    /// Runs one clipboard command detached from the picker pane.
    ///
    /// `wl-copy`, `xclip`, and `xsel` leave a background process that serves the selection.
    /// A new process group keeps that process alive when Herdr closes the picker pane, and
    /// null output keeps it from holding the pane terminal open.
    fn run_with_stdin(&self, tool: ClipboardTool, stdin: &str) -> Result<(), ClipboardError> {
        let mut child = Command::new(tool.name)
            .args(tool.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .map_err(|err| ClipboardError::SpawnFailed {
                tool: tool.name.to_string(),
                message: err.to_string(),
            })?;

        if let Some(mut child_stdin) = child.stdin.take() {
            use std::io::Write;
            child_stdin
                .write_all(stdin.as_bytes())
                .map_err(|err| ClipboardError::WriteFailed {
                    tool: tool.name.to_string(),
                    message: err.to_string(),
                })?;
        }

        let status = child.wait().map_err(|err| ClipboardError::WaitFailed {
            tool: tool.name.to_string(),
            message: err.to_string(),
        })?;

        if status.success() {
            Ok(())
        } else {
            Err(ClipboardError::CommandFailed {
                tool: tool.name.to_string(),
                status: status.to_string(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs a real command with the production settings: stdin reaches it, and it gets its
    /// own process group. The first stdin line names the file that the script writes.
    #[test]
    fn the_command_reads_stdin_in_its_own_process_group() {
        let marker = std::env::temp_dir().join(format!("yank-clip-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let tool = ClipboardTool {
            name: "sh",
            args: &[
                "-c",
                r#"read -r path && cat > "$path" && ps -o pgid= -p $$ >> "$path" && ps -o pgid= -p $PPID >> "$path""#,
            ],
        };

        SystemCommandRunner
            .run_with_stdin(tool, &format!("{}\ncopied\n", marker.display()))
            .unwrap();

        let output = std::fs::read_to_string(&marker).unwrap();
        let mut lines = output.lines();
        assert_eq!(lines.next(), Some("copied"));
        let child_group = lines.next().unwrap().trim().to_string();
        let parent_group = lines.next().unwrap().trim().to_string();
        assert_ne!(
            child_group, parent_group,
            "the command must not share the picker group"
        );
        std::fs::remove_file(marker).unwrap();
    }

    #[test]
    fn a_failing_command_reports_its_status() {
        let tool = ClipboardTool {
            name: "sh",
            args: &["-c", "cat > /dev/null; exit 3"],
        };

        let error = SystemCommandRunner.run_with_stdin(tool, "x").unwrap_err();

        assert!(matches!(error, ClipboardError::CommandFailed { .. }));
    }
}
