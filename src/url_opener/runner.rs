use super::error::UrlOpenError;
use super::tool::UrlOpenTool;

/**
 * Injectable command seam for URL opener detection and execution.
 */
pub(crate) trait UrlOpenCommandRunner {
    fn command_exists(&self, command: &str) -> bool;
    /// Runs `tool` with `argument`, a checked web URL or canonical file path, as one argument.
    fn run(&self, tool: UrlOpenTool, argument: &str) -> Result<(), UrlOpenError>;
}
