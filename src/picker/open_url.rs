use crate::url_opener::{UrlOpenError, UrlOpener};
use anyhow::{Context, Result};

/**
 * Opens a selected picker URL or local file and attaches user-facing context on failure.
 */
pub(crate) fn open_selected_url(opener: &impl UrlOpener, target: &str) -> Result<()> {
    opener.open(target).map_err(open_error)
}

/**
 * Reports a launch failure without the address.
 *
 * The message goes to a Herdr notification and to stderr. A URL can carry a session token or
 * a signed query parameter, and a path can name a private file, so the address itself never
 * enters the message.
 */
fn open_error(error: UrlOpenError) -> anyhow::Error {
    Err::<(), UrlOpenError>(error)
        .context("failed to open selected URL or file")
        .unwrap_err()
}
