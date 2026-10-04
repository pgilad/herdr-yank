use serde::{Deserialize, Serialize};

/// Herdr pane identifier, such as `w1:p2`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PaneId(pub String);

impl PaneId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl std::fmt::Display for PaneId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Cell-space rectangle in Herdr layout coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

#[cfg(test)]
impl Rect {
    pub fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// Exact visible pane rows plus the logical lines reconstructed from soft wraps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibleViewport {
    pub rows: Vec<String>,
    pub logical_lines: Vec<String>,
    pub segments: Vec<LogicalLineVisualSegment>,
}

/// Maps a logical byte range onto a row/column range in the exact visible viewport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicalLineVisualSegment {
    pub logical_line: usize,
    pub logical_start: usize,
    pub logical_end: usize,
    pub row: usize,
    pub col_start: usize,
    pub col_end: usize,
}

/// Visible text of the target pane, captured before the picker tab opens.
///
/// The picker rebuilds soft-wrapped lines from `rows` with its own terminal width, because
/// Herdr reports the content rows of a pane but not its content columns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourcePaneSnapshot {
    /// Visible rows, top to bottom. Herdr omits trailing blank rows.
    pub rows: Vec<String>,
    /// Content rows of the target pane.
    pub content_rows: u16,
}

/// Source tab restored after picker completion, and whether the picker must zoom.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PickerReturnContext {
    pub return_tab_id: String,
    pub zoom_picker: bool,
}

/// Serializable regex pattern config resolved before the picker pane starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatternSpec {
    pub name: String,
    pub regex: String,
    pub priority: u16,
}

/// Operation performed when the user selects a hinted match.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PickerAction {
    #[default]
    Copy,
    OpenUrl,
}

/// How the picker delivers copied text to the clipboard.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClipboardMode {
    /// Prefer an OSC 52 escape, then fall back to a platform clipboard command.
    ///
    /// Herdr sends an OSC 52 write to the clipboard of the attached client, which is the
    /// computer of the user also when Herdr runs on a remote host.
    #[default]
    Auto,
    /// Only use a platform clipboard command on the computer that runs the Herdr server.
    Tool,
    /// Only use an OSC 52 escape.
    Osc52,
}

/// Terminal color for one picker style slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeColor {
    Black,
    DarkGrey,
    Grey,
    White,
    Red,
    DarkRed,
    Green,
    DarkGreen,
    Yellow,
    DarkYellow,
    Blue,
    DarkBlue,
    Magenta,
    DarkMagenta,
    Cyan,
    DarkCyan,
    Ansi(u8),
    Rgb { r: u8, g: u8, b: u8 },
}

/// Colors applied to each picker style slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Theme {
    pub unmatched_fg: ThemeColor,
    pub match_fg: ThemeColor,
    pub match_bg: Option<ThemeColor>,
    pub hint_fg: ThemeColor,
    pub hint_bg: ThemeColor,
    pub selected_match_fg: ThemeColor,
    pub selected_match_bg: Option<ThemeColor>,
    pub selected_hint_fg: ThemeColor,
    pub selected_hint_bg: ThemeColor,
    pub status_fg: ThemeColor,
    pub status_bg: ThemeColor,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            unmatched_fg: ThemeColor::DarkGrey,
            match_fg: ThemeColor::Yellow,
            match_bg: None,
            hint_fg: ThemeColor::Black,
            hint_bg: ThemeColor::Cyan,
            selected_match_fg: ThemeColor::White,
            selected_match_bg: Some(ThemeColor::DarkMagenta),
            selected_hint_fg: ThemeColor::White,
            selected_hint_bg: ThemeColor::Magenta,
            status_fg: ThemeColor::Black,
            status_bg: ThemeColor::Grey,
        }
    }
}

/// User-configurable picker behavior resolved before the picker pane starts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PickerSettings {
    /// Built-in pattern names to keep enabled. `None` keeps every built-in.
    #[serde(default)]
    pub enabled_builtin_patterns: Option<Vec<String>>,
    #[serde(default)]
    pub theme: Theme,
    #[serde(default)]
    pub clipboard: ClipboardMode,
}

/// Full picker launch payload passed from the action process to picker mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PickerSnapshot {
    pub source: SourcePaneSnapshot,
    pub session: PickerReturnContext,
    #[serde(default)]
    pub action: PickerAction,
    #[serde(default)]
    pub custom_patterns: Vec<PatternSpec>,
    #[serde(default)]
    pub settings: PickerSettings,
}

/// Direction of a Herdr binary pane split as exposed by layout snapshots and replay commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SplitDirection {
    Right,
    Down,
}

/// Copied/highlighted occurrence found on one unwrapped logical pane line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchSpan {
    /// Zero-based logical line index.
    pub line: usize,
    /// UTF-8 byte offset where the copied/highlighted substring starts.
    pub start: usize,
    /// UTF-8 byte offset immediately after the copied/highlighted substring.
    pub end: usize,
    /// Copied text; for regexes with named capture `match`, this is that capture.
    pub text: String,
    /// Built-in pattern name that produced this occurrence.
    pub pattern: String,
    /// Match precedence where lower numbers are higher priority.
    pub priority: u16,
}

impl MatchSpan {
    /// Returns the matched byte length used by matcher tie-breaking.
    pub fn len_bytes(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// Returns whether copied/highlighted byte ranges overlap on the same logical line.
    pub fn overlaps(&self, other: &Self) -> bool {
        self.line == other.line && self.start < other.end && other.start < self.end
    }
}

/// A unique matched text pattern and all its occurrences in the pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HintAssignment {
    pub hint: String,
    pub text: String,
    pub occurrences: Vec<MatchSpan>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderStyle {
    Unmatched,
    Match,
    Hint,
    SelectedMatch,
    SelectedHint,
    Status,
}

impl RenderStyle {
    /// Whether this style marks part of a hinted match.
    pub fn is_match(self) -> bool {
        matches!(
            self,
            Self::Match | Self::Hint | Self::SelectedMatch | Self::SelectedHint
        )
    }
}

/// A contiguous span of text to render in the picker, with a single style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderSpan {
    pub text: String,
    pub style: RenderStyle,
}

/// A single line of text to render in the picker,
/// with style spans for matched/highlighted regions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderLine {
    pub spans: Vec<RenderSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerOutcome {
    Copied { text: String },
    OpenedUrl { url: String },
    Cancelled,
    NoMatches,
}
