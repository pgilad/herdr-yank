mod error;
mod runner;
mod system;
mod tool;

pub use error::UrlOpenError;

use runner::UrlOpenCommandRunner;
use std::path::{Path, PathBuf};
use system::SystemCommandRunner;
use tool::{UrlOpenEnvironment, UrlOpenTool};

/**
 * Schemes the picker is allowed to hand to the system launcher.
 *
 * `open` and `xdg-open` give a `file://` URL to the default application of the file type,
 * which can start a program or a script. A `file://` URL would skip the local file checks
 * below, thus only web URLs are allowed. The match layer already filters to these, but the
 * launcher re-checks so that no future caller can pass a `file:` or `javascript:` URL, or an
 * argument that begins with `-`, to `open`.
 */
const ALLOWED_SCHEMES: [&str; 2] = ["http://", "https://"];

/**
 * Lowercase extensions of the local files the picker is allowed to hand to the system launcher.
 *
 * A local file goes to the default application of its type. For a script, an app bundle, or an
 * installer, that application runs it. These types open in a browser or a viewer instead.
 */
const ALLOWED_FILE_EXTENSIONS: [&str; 9] = [
    "html", "htm", "pdf", "svg", "png", "jpg", "jpeg", "gif", "webp",
];

/**
 * Opens web URLs and local files through the system's default handler.
 */
pub trait UrlOpener {
    fn open(&self, target: &str) -> Result<(), UrlOpenError>;
}

/**
 * Reports whether the launcher accepts a picker target.
 *
 * A local path costs a file system lookup, because only an existing file qualifies.
 */
pub fn is_openable_target(target: &str) -> bool {
    launch_argument(target, home_dir().as_deref()).is_ok()
}

/**
 * Checks a picker target and returns the argument for the system launcher.
 *
 * A target is a web URL, or an absolute or `~/` path to an existing file of an allowed type. A
 * relative path does not qualify, as the launcher does not know the working directory of the
 * pane.
 */
fn launch_argument(target: &str, home: Option<&Path>) -> Result<String, UrlOpenError> {
    if target.starts_with('/') || target.starts_with("~/") {
        return openable_file(target, home);
    }
    if is_web_url(target) {
        return Ok(target.to_string());
    }
    Err(UrlOpenError::UnsupportedScheme {
        scheme: scheme_of(target),
    })
}

fn is_web_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    ALLOWED_SCHEMES
        .iter()
        .any(|scheme| lower.starts_with(scheme))
}

/**
 * Resolves a local path to the canonical path of the file the launcher may open.
 *
 * The type check uses the canonical path, so a `report.html` symlink to a script is refused.
 * The launcher also gets the canonical path, which is the file that passed the check and
 * always begins with `/`, so `open` cannot read it as a flag.
 */
fn openable_file(target: &str, home: Option<&Path>) -> Result<String, UrlOpenError> {
    let path = match target.strip_prefix("~/") {
        Some(relative) => home.ok_or(UrlOpenError::NotAFile)?.join(relative),
        None => PathBuf::from(target),
    };
    let canonical = std::fs::canonicalize(path).map_err(|_| UrlOpenError::NotAFile)?;
    if !canonical.is_file() {
        return Err(UrlOpenError::NotAFile);
    }
    let allowed = canonical
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ALLOWED_FILE_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
        });
    if !allowed {
        return Err(UrlOpenError::UnsupportedFileType);
    }
    canonical
        .into_os_string()
        .into_string()
        .map_err(|_| UrlOpenError::NotAFile)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

/**
 * Extracts the leading scheme so a rejection can name it without repeating the URL.
 *
 * A rejected URL can be a `postgres://user:password@host` style string from the pane, thus
 * only this prefix is safe to put in an error that reaches a notification. Anything that is
 * not a plain scheme, such as an `open` flag or a `git@host:path` remote, becomes `unknown`.
 */
fn scheme_of(url: &str) -> String {
    url.split_once("://")
        .or_else(|| url.split_once(':'))
        .map(|(scheme, _)| scheme)
        .filter(|scheme| {
            !scheme.is_empty()
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        })
        .unwrap_or("unknown")
        .to_ascii_lowercase()
}

/**
 * Production URL opener using the platform's default launcher.
 */
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemUrlOpener;

impl UrlOpener for SystemUrlOpener {
    fn open(&self, target: &str) -> Result<(), UrlOpenError> {
        open_with_runner(
            target,
            &SystemCommandRunner,
            UrlOpenEnvironment::current(),
            home_dir().as_deref(),
        )
    }
}

fn open_with_runner(
    target: &str,
    runner: &impl UrlOpenCommandRunner,
    env: UrlOpenEnvironment,
    home: Option<&Path>,
) -> Result<(), UrlOpenError> {
    let argument = launch_argument(target, home)?;
    let tool = UrlOpenTool::for_environment(env)?;
    if !runner.command_exists(tool.name) {
        return Err(UrlOpenError::NoToolFound { tool: tool.name });
    }
    runner.run(tool, &argument)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    use std::cell::RefCell;

    #[derive(Default)]
    struct FakeRunner {
        available: bool,
        runs: RefCell<Vec<(&'static str, String)>>,
        failure: Option<UrlOpenError>,
    }

    impl UrlOpenCommandRunner for FakeRunner {
        fn command_exists(&self, _command: &str) -> bool {
            self.available
        }

        fn run(&self, tool: UrlOpenTool, argument: &str) -> Result<(), UrlOpenError> {
            self.runs
                .borrow_mut()
                .push((tool.name, argument.to_string()));
            self.failure.clone().map_or(Ok(()), Err)
        }
    }

    #[test]
    fn macos_uses_open_with_the_url_as_one_argument() {
        let runner = FakeRunner {
            available: true,
            ..FakeRunner::default()
        };
        open_with_runner(
            "https://example.com/a b",
            &runner,
            UrlOpenEnvironment::Macos,
            None,
        )
        .unwrap();

        assert_eq!(
            runner.runs.borrow()[0],
            ("open", "https://example.com/a b".into())
        );
    }

    #[test]
    fn linux_uses_xdg_open() {
        let runner = FakeRunner {
            available: true,
            ..FakeRunner::default()
        };
        open_with_runner(
            "https://example.com",
            &runner,
            UrlOpenEnvironment::Linux,
            None,
        )
        .unwrap();

        assert_eq!(runner.runs.borrow()[0].0, "xdg-open");
    }

    #[test]
    fn refuses_schemes_outside_the_allow_list() {
        let runner = FakeRunner {
            available: true,
            ..FakeRunner::default()
        };

        for (url, scheme) in [
            ("javascript:alert(1)", "javascript"),
            ("-a/Applications/Calculator.app", "unknown"),
            ("ssh://host/path", "ssh"),
            ("data:text/html,x", "data"),
            ("file:///Applications/Calculator.app", "file"),
            ("FILE:///tmp/run.command", "file"),
        ] {
            let error =
                open_with_runner(url, &runner, UrlOpenEnvironment::Macos, None).unwrap_err();
            assert_eq!(
                error,
                UrlOpenError::UnsupportedScheme {
                    scheme: scheme.to_string()
                }
            );
        }

        assert!(runner.runs.borrow().is_empty());
    }

    #[test]
    fn rejection_names_the_scheme_without_repeating_the_credential() {
        let error = open_with_runner(
            "postgres://admin:hunter2@db.internal:5432/app",
            &FakeRunner::default(),
            UrlOpenEnvironment::Macos,
            None,
        )
        .unwrap_err();

        let message = error.to_string();
        assert_eq!(
            message,
            "refusing to open a postgres URL: only http and https URLs and local files are allowed"
        );
        assert!(!message.contains("hunter2"));
    }

    #[test]
    fn scheme_extraction_falls_back_when_there_is_no_plain_scheme() {
        assert_eq!(scheme_of("HTTPS://example.com"), "https");
        assert_eq!(scheme_of("git@github.com:org/repo.git"), "unknown");
        assert_eq!(scheme_of("plain-text"), "unknown");
        assert_eq!(scheme_of(":leading-colon"), "unknown");
    }

    #[test]
    fn reports_missing_and_unsupported_launchers() {
        let error = open_with_runner(
            "https://example.com",
            &FakeRunner::default(),
            UrlOpenEnvironment::Linux,
            None,
        )
        .unwrap_err();
        assert_eq!(error, UrlOpenError::NoToolFound { tool: "xdg-open" });

        let error = open_with_runner(
            "https://example.com",
            &FakeRunner::default(),
            UrlOpenEnvironment::Unsupported,
            None,
        )
        .unwrap_err();
        assert_eq!(error, UrlOpenError::UnsupportedPlatform);
    }

    fn available_runner() -> FakeRunner {
        FakeRunner {
            available: true,
            ..FakeRunner::default()
        }
    }

    fn canonical(path: &Path) -> String {
        std::fs::canonicalize(path)
            .unwrap()
            .into_os_string()
            .into_string()
            .unwrap()
    }

    #[test]
    fn opens_an_allowed_local_file_by_its_canonical_path() {
        let dir = TempDir::new("open-file");
        let report = dir.join("report.html");
        let chart = dir.join("Chart.PNG");
        std::fs::write(&report, "<html></html>").unwrap();
        std::fs::write(&chart, "png").unwrap();
        let runner = available_runner();

        for path in [&report, &chart] {
            open_with_runner(
                path.to_str().unwrap(),
                &runner,
                UrlOpenEnvironment::Macos,
                None,
            )
            .unwrap();
        }

        assert_eq!(
            *runner.runs.borrow(),
            [("open", canonical(&report)), ("open", canonical(&chart))]
        );
    }

    #[test]
    fn expands_a_home_relative_path() {
        let home = TempDir::new("open-home");
        std::fs::create_dir(home.join("repos")).unwrap();
        let page = home.join("repos").join("page.htm");
        std::fs::write(&page, "<html></html>").unwrap();
        let runner = available_runner();

        open_with_runner(
            "~/repos/page.htm",
            &runner,
            UrlOpenEnvironment::Linux,
            Some(&home),
        )
        .unwrap();

        assert_eq!(runner.runs.borrow()[0], ("xdg-open", canonical(&page)));
        assert_eq!(
            open_with_runner("~/repos/page.htm", &runner, UrlOpenEnvironment::Linux, None),
            Err(UrlOpenError::NotAFile)
        );
    }

    #[test]
    fn refuses_local_files_that_can_run_or_do_not_exist() {
        let dir = TempDir::new("open-refuse");
        for name in ["run.command", "install.sh", "Makefile"] {
            std::fs::write(dir.join(name), "#!/bin/sh").unwrap();
        }
        std::fs::create_dir(dir.join("Tool.app")).unwrap();
        std::fs::create_dir(dir.join("folder.html")).unwrap();
        std::os::unix::fs::symlink(dir.join("run.command"), dir.join("link.html")).unwrap();
        let runner = available_runner();
        let path = |name: &str| dir.join(name).to_str().unwrap().to_string();

        for (target, expected) in [
            (path("run.command"), UrlOpenError::UnsupportedFileType),
            (path("install.sh"), UrlOpenError::UnsupportedFileType),
            (path("Makefile"), UrlOpenError::UnsupportedFileType),
            (path("link.html"), UrlOpenError::UnsupportedFileType),
            (path("Tool.app"), UrlOpenError::NotAFile),
            (path("folder.html"), UrlOpenError::NotAFile),
            (path("missing.html"), UrlOpenError::NotAFile),
            (
                "report.html".to_string(),
                UrlOpenError::UnsupportedScheme {
                    scheme: "unknown".to_string(),
                },
            ),
        ] {
            let error =
                open_with_runner(&target, &runner, UrlOpenEnvironment::Macos, None).unwrap_err();
            assert_eq!(error, expected, "{target}");
        }

        assert!(runner.runs.borrow().is_empty());
    }
}
