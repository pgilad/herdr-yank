use thiserror::Error;

/// User-facing failure while copying text to the system clipboard.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ClipboardError {
    #[error("no supported clipboard tool found: tried {tried}")]
    NoToolFound { tried: String },
    #[error("failed to start {tool}: {message}")]
    SpawnFailed { tool: String, message: String },
    #[error("failed to write clipboard text to {tool}: {message}")]
    WriteFailed { tool: String, message: String },
    #[error("{tool} exited with status {status}")]
    CommandFailed { tool: String, status: String },
    #[error("failed waiting for {tool}: {message}")]
    WaitFailed { tool: String, message: String },
    #[error("the terminal is unavailable for OSC 52: {message}")]
    Osc52Unavailable { message: String },
    #[error("selection is {bytes} bytes, above the {limit}-byte OSC 52 limit of Herdr")]
    Osc52TooLarge { bytes: usize, limit: usize },
    /// `auto` mode found neither a usable OSC 52 path nor a clipboard tool.
    #[error("OSC 52 failed ({osc52}) and no clipboard tool was found (tried {tried})")]
    NoClipboard { osc52: String, tried: String },
}

impl ClipboardError {
    pub(crate) fn no_tool_found(tools: &[crate::clipboard::tool::ClipboardTool]) -> Self {
        Self::NoToolFound {
            tried: tool_names(tools),
        }
    }

    pub(crate) fn no_clipboard(
        osc52: &ClipboardError,
        tools: &[crate::clipboard::tool::ClipboardTool],
    ) -> Self {
        Self::NoClipboard {
            osc52: osc52.to_string(),
            tried: tool_names(tools),
        }
    }
}

fn tool_names(tools: &[crate::clipboard::tool::ClipboardTool]) -> String {
    tools
        .iter()
        .map(|tool| tool.name)
        .collect::<Vec<_>>()
        .join(", ")
}
