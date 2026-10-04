use crate::model::PaneId;
use std::env;
use std::path::PathBuf;

/// Herdr runtime values that locate the socket and the invoking pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HerdrContext {
    pub pane_id: Option<String>,
    pub tab_id: Option<String>,
    pub socket_path: Option<PathBuf>,
}

impl HerdrContext {
    /// Reads the context from the environment that Herdr gives to a command.
    ///
    /// Herdr sets `HERDR_PANE_ID` for plugin commands and panes from the focused pane of the
    /// invocation, and `HERDR_ACTIVE_PANE_ID` for custom key commands.
    pub fn from_env() -> Self {
        Self {
            pane_id: env::var("HERDR_PANE_ID")
                .or_else(|_| env::var("HERDR_ACTIVE_PANE_ID"))
                .ok(),
            tab_id: env::var("HERDR_TAB_ID").ok(),
            socket_path: env::var_os("HERDR_SOCKET_PATH").map(PathBuf::from),
        }
    }

    pub fn target_pane(&self) -> Option<PaneId> {
        self.pane_id.clone().map(PaneId::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_pane_comes_from_the_pane_id() {
        let context = HerdrContext {
            pane_id: Some("w1:p3".to_string()),
            tab_id: None,
            socket_path: None,
        };

        assert_eq!(context.target_pane(), Some(PaneId::new("w1:p3")));
        assert_eq!(
            HerdrContext {
                pane_id: None,
                ..context
            }
            .target_pane(),
            None
        );
    }
}
