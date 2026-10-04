use thiserror::Error;

/**
 * User-facing failure while opening a URL or a local file with the system handler.
 */
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum UrlOpenError {
    #[error("opening URLs is unsupported on this platform")]
    UnsupportedPlatform,
    #[error("system URL opener {tool} was not found")]
    NoToolFound { tool: &'static str },
    /// Carries the scheme only. The rejected URL can hold a credential, and this error
    /// reaches a Herdr notification.
    #[error(
        "refusing to open a {scheme} URL: only http and https URLs and local files are allowed"
    )]
    UnsupportedScheme { scheme: String },
    /// Names no path, as a path can name a private file.
    #[error("the selected path is not an existing file")]
    NotAFile,
    #[error("refusing to open the selected file: only web pages, PDFs, and images are allowed")]
    UnsupportedFileType,
    #[error("failed to start {tool}: {message}")]
    SpawnFailed { tool: String, message: String },
    #[error("failed waiting for {tool}: {message}")]
    WaitFailed { tool: String, message: String },
    #[error("{tool} exited with status {status}")]
    CommandFailed { tool: String, status: String },
}
