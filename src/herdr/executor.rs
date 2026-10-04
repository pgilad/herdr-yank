use crate::herdr::client::{HerdrClient, LaunchLayoutNode};
use crate::herdr::layout::{target_content_size, target_is_zoomed, ExportedNode};
use crate::herdr::snapshot::PickerLaunchFiles;
use crate::model::{
    PaneId, PatternSpec, PickerAction, PickerReturnContext, PickerSettings, PickerSnapshot,
    SourcePaneSnapshot,
};
use anyhow::{bail, Result};
use std::path::Path;

/// Target pane and launch options that the picker tab is built from.
pub struct LaunchRequest<'a> {
    pub target: &'a PaneId,
    /// Content rows that Herdr reports for the target pane, if it has scroll metrics.
    pub viewport_rows: Option<u16>,
    pub binary_path: &'a Path,
    pub action: PickerAction,
    pub custom_patterns: Vec<PatternSpec>,
    pub settings: PickerSettings,
}

/// Captures source state and atomically applies the temporary picker layout.
pub fn launch_layout_tab_picker<C: HerdrClient>(
    client: &mut C,
    request: LaunchRequest<'_>,
) -> Result<()> {
    let target = request.target;
    let layout = client.pane_layout(target)?;
    let size = target_content_size(&layout, target, request.viewport_rows)?;

    if size.rows == 0 {
        bail!("target pane {target} has zero visible content height");
    }

    let exported = client.layout_export(target)?;
    if !exported.root.contains_pane(target) {
        bail!("target pane {target} was not present in the exported layout");
    }

    let visible_text = client.pane_read_visible(target, size.rows)?;
    let return_context = PickerReturnContext {
        return_tab_id: exported.tab_id.clone(),
        zoom_picker: target_is_zoomed(&layout, target),
    };
    let snapshot = PickerSnapshot {
        source: SourcePaneSnapshot {
            rows: visible_text.lines().map(str::to_string).collect(),
            content_rows: size.rows,
        },
        session: return_context.clone(),
        action: request.action,
        custom_patterns: request.custom_patterns,
        settings: request.settings,
    };

    // Nothing to hint. Report it without building a temporary tab, so an accidental key
    // press does not leave a dead-end pane waiting to be dismissed.
    if !crate::picker::has_matches(&snapshot, size.columns)
        && report_no_matches(client, request.action)
    {
        return Ok(());
    }

    let files = PickerLaunchFiles::create(&snapshot)?;
    let commands = PaneCommands::new(request.binary_path, &files);
    let root = convert_layout(&exported.root, target, &commands);
    let tab_label = match request.action {
        PickerAction::Copy => "Herdr Yank",
        PickerAction::OpenUrl => "Herdr Yank: Open URL",
    };
    let applied = match client.apply_layout(&exported.workspace_id, tab_label, &root) {
        Ok(value) => value,
        Err(error) => {
            let _ = files.cleanup();
            return Err(error);
        }
    };

    let focus_result = client.focus_pane(&applied.picker_pane_id);

    if let Err(error) = focus_result.and_then(|_| files.signal_ready()) {
        if let Err(cleanup) = cleanup_session(client, &return_context, &applied.tab_id) {
            eprintln!("launch cleanup also failed: {cleanup:#}");
        }

        if let Err(cleanup) = files.cleanup() {
            eprintln!("launch file cleanup also failed: {cleanup:#}");
        }

        return Err(error);
    }

    Ok(())
}

/// Notifies that an action found nothing, reporting whether the user will actually see it.
///
/// Herdr honours `[ui.toast] delivery`, so the notification can be suppressed. A `false`
/// return means the caller must fall back to the on-screen picker rather than leave the key
/// press with no feedback at all.
fn report_no_matches<C: HerdrClient>(client: &mut C, action: PickerAction) -> bool {
    match client.show_notification(crate::picker::no_match_message(action), None) {
        Ok(outcome) => {
            if !outcome.shown {
                eprintln!(
                    "Herdr Yank: notification suppressed ({}); showing the picker instead",
                    outcome.reason
                );
            }
            outcome.shown
        }
        Err(error) => {
            eprintln!("Herdr Yank: notification failed ({error:#}); showing the picker instead");
            false
        }
    }
}

/// Argv of the two kinds of pane in the temporary picker tab.
struct PaneCommands {
    picker: Vec<String>,
    idle: Vec<String>,
}

impl PaneCommands {
    fn new(binary: &Path, files: &PickerLaunchFiles) -> Self {
        let binary = binary.to_string_lossy().into_owned();
        Self {
            picker: vec![
                binary.clone(),
                "pick".into(),
                "--snapshot".into(),
                files.snapshot_path.to_string_lossy().into_owned(),
                "--ready".into(),
                files.ready_path.to_string_lossy().into_owned(),
            ],
            idle: vec![binary, "idle".into()],
        }
    }
}

/// Replaces each pane of the exported tree with the picker or an idle command.
fn convert_layout(
    node: &ExportedNode,
    target: &PaneId,
    commands: &PaneCommands,
) -> LaunchLayoutNode {
    match node {
        ExportedNode::Pane { pane_id } => LaunchLayoutNode::Pane {
            command: if pane_id.as_deref() == Some(target.0.as_str()) {
                commands.picker.clone()
            } else {
                commands.idle.clone()
            },
        },
        ExportedNode::Split {
            direction,
            ratio,
            first,
            second,
        } => LaunchLayoutNode::Split {
            direction: *direction,
            ratio: *ratio,
            first: Box::new(convert_layout(first, target, commands)),
            second: Box::new(convert_layout(second, target, commands)),
        },
    }
}

/// Restores the source tab and closes only the explicit temporary tab.
pub fn cleanup_session<C: HerdrClient>(
    client: &mut C,
    session: &PickerReturnContext,
    temporary_tab_id: &str,
) -> Result<()> {
    if temporary_tab_id.is_empty() {
        bail!("temporary picker tab id is missing");
    }

    if temporary_tab_id == session.return_tab_id {
        bail!(
            "refusing to close source tab {} as temporary picker tab",
            temporary_tab_id
        );
    }

    let mut first = None;

    if let Err(e) = client.focus_tab(&session.return_tab_id) {
        first = Some(e);
    }

    if let Err(e) = client.close_tab(temporary_tab_id) {
        if first.is_none() {
            first = Some(e);
        }
    }

    first.map_or(Ok(()), Err)
}

pub fn zoom_picker<C: HerdrClient>(
    client: &mut C,
    snapshot: &PickerSnapshot,
    pane_id: &PaneId,
) -> Result<()> {
    if snapshot.session.zoom_picker {
        client.zoom_pane(pane_id)?;
    }

    Ok(())
}

pub fn run_snapshot_picker(snapshot: &PickerSnapshot) -> Result<()> {
    crate::picker::run_picker(snapshot).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::herdr::client::{AppliedLayout, NotificationOutcome, PaneDetails};
    use crate::herdr::layout::{ExportedLayout, LayoutPane, LayoutSnapshot};
    use crate::model::{Rect, SplitDirection};
    use anyhow::{anyhow, Context};

    #[derive(Default)]
    struct FakeClient {
        layout: Option<LayoutSnapshot>,
        exported: Option<ExportedLayout>,
        calls: Vec<String>,
        launch_paths: Option<(std::path::PathBuf, std::path::PathBuf)>,
        fail_focus_pane: bool,
        fail_focus_tab: bool,
        pane_text: Option<String>,
        notification_shown: bool,
        fail_notification: bool,
    }

    impl HerdrClient for FakeClient {
        fn pane_layout(&mut self, _pane: &PaneId) -> Result<LayoutSnapshot> {
            self.calls.push("pane_layout".into());
            self.layout.take().context("missing fake layout")
        }

        fn layout_export(&mut self, _pane: &PaneId) -> Result<ExportedLayout> {
            self.calls.push("layout_export".into());
            self.exported.take().context("missing fake exported layout")
        }

        fn pane_details(&mut self, _pane: &PaneId) -> Result<PaneDetails> {
            self.calls.push("pane_details".into());
            Ok(PaneDetails::default())
        }

        fn pane_read_visible(&mut self, _pane: &PaneId, lines: u16) -> Result<String> {
            self.calls.push(format!("pane_read:{lines}"));
            Ok(self
                .pane_text
                .clone()
                .unwrap_or_else(|| "https://example.com".into()))
        }

        fn show_notification(
            &mut self,
            title: &str,
            _body: Option<&str>,
        ) -> Result<NotificationOutcome> {
            self.calls.push(format!("notify:{title}"));
            if self.fail_notification {
                return Err(anyhow!("notification failed"));
            }
            Ok(NotificationOutcome {
                shown: self.notification_shown,
                reason: if self.notification_shown {
                    "shown".into()
                } else {
                    "disabled".into()
                },
            })
        }

        fn apply_layout(
            &mut self,
            workspace_id: &str,
            _tab_label: &str,
            root: &LaunchLayoutNode,
        ) -> Result<AppliedLayout> {
            self.calls.push(format!("apply:{workspace_id}"));
            self.launch_paths = picker_paths(root);
            Ok(AppliedLayout {
                tab_id: "w1:t2".into(),
                picker_pane_id: PaneId::new("w1:p2"),
            })
        }

        fn focus_pane(&mut self, pane: &PaneId) -> Result<()> {
            self.calls.push(format!("focus_pane:{pane}"));
            let (_, ready) = self.launch_paths.as_ref().context("missing launch paths")?;
            assert!(!ready.exists(), "barrier released before picker focus");
            if self.fail_focus_pane {
                Err(anyhow!("focus failed"))
            } else {
                Ok(())
            }
        }

        fn zoom_pane(&mut self, pane: &PaneId) -> Result<()> {
            self.calls.push(format!("zoom:{pane}"));
            Ok(())
        }

        fn focus_tab(&mut self, tab_id: &str) -> Result<()> {
            self.calls.push(format!("focus_tab:{tab_id}"));
            if self.fail_focus_tab {
                Err(anyhow!("tab focus failed"))
            } else {
                Ok(())
            }
        }

        fn close_tab(&mut self, tab_id: &str) -> Result<()> {
            self.calls.push(format!("close_tab:{tab_id}"));
            Ok(())
        }
    }

    impl FakeClient {
        fn with_source(zoomed: bool) -> Self {
            Self {
                layout: Some(source_layout(zoomed)),
                exported: Some(exported_layout()),
                ..Self::default()
            }
        }

        fn remove_launch_files(&self) {
            let (snapshot, ready) = self.launch_paths.clone().unwrap();
            PickerLaunchFiles::for_paths(&snapshot, &ready)
                .cleanup()
                .unwrap();
        }
    }

    fn picker_paths(node: &LaunchLayoutNode) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
        match node {
            LaunchLayoutNode::Pane { command }
                if command.get(1).is_some_and(|argument| argument == "pick") =>
            {
                Some((command.get(3)?.into(), command.get(5)?.into()))
            }
            LaunchLayoutNode::Split { first, second, .. } => {
                picker_paths(first).or_else(|| picker_paths(second))
            }
            _ => None,
        }
    }

    fn source_layout(zoomed: bool) -> LayoutSnapshot {
        LayoutSnapshot {
            area: Rect::new(0, 0, 80, 24),
            focused_pane_id: Some("w1:p1".into()),
            panes: vec![LayoutPane {
                focused: true,
                pane_id: "w1:p1".into(),
                rect: Rect::new(0, 0, 80, 24),
            }],
            zoomed,
        }
    }

    fn exported_layout() -> ExportedLayout {
        ExportedLayout {
            workspace_id: "w1".into(),
            tab_id: "w1:t1".into(),
            root: ExportedNode::Pane {
                pane_id: Some("w1:p1".into()),
            },
        }
    }

    fn request(
        target: &PaneId,
        action: PickerAction,
        viewport_rows: Option<u16>,
    ) -> LaunchRequest<'_> {
        LaunchRequest {
            target,
            viewport_rows,
            binary_path: Path::new("/tmp/herdr yank"),
            action,
            custom_patterns: Vec::new(),
            settings: PickerSettings::default(),
        }
    }

    fn picker_snapshot(zoom_picker: bool) -> PickerSnapshot {
        PickerSnapshot {
            source: SourcePaneSnapshot {
                rows: Vec::new(),
                content_rows: 24,
            },
            session: PickerReturnContext {
                return_tab_id: "w1:t1".into(),
                zoom_picker,
            },
            action: PickerAction::Copy,
            custom_patterns: Vec::new(),
            settings: PickerSettings::default(),
        }
    }

    #[test]
    fn conversion_preserves_argv_and_split() {
        let tree = ExportedNode::Split {
            direction: SplitDirection::Right,
            ratio: 0.37,
            first: Box::new(ExportedNode::Pane {
                pane_id: Some("a".into()),
            }),
            second: Box::new(ExportedNode::Pane {
                pane_id: Some("b".into()),
            }),
        };
        let files = PickerLaunchFiles::for_paths(Path::new("/s p"), Path::new("/r p"));
        let commands = PaneCommands::new(Path::new("/a b/π'"), &files);
        let converted = convert_layout(&tree, &PaneId::new("b"), &commands);
        let LaunchLayoutNode::Split {
            direction,
            ratio,
            first,
            second,
        } = converted
        else {
            panic!("expected split layout");
        };
        assert_eq!(direction, SplitDirection::Right);
        assert_eq!(ratio, 0.37);
        assert!(matches!(
            first.as_ref(),
            LaunchLayoutNode::Pane { command }
                if command == &vec!["/a b/π'".to_string(), "idle".to_string()]
        ));
        assert!(matches!(
            second.as_ref(),
            LaunchLayoutNode::Pane { command }
                if command == &vec![
                    "/a b/π'".to_string(),
                    "pick".to_string(),
                    "--snapshot".to_string(),
                    "/s p".to_string(),
                    "--ready".to_string(),
                    "/r p".to_string(),
                ]
        ));
    }

    #[test]
    fn launch_focuses_picker_before_releasing_barrier() {
        let target = PaneId::new("w1:p1");
        let mut client = FakeClient::with_source(false);

        launch_layout_tab_picker(&mut client, request(&target, PickerAction::Copy, Some(22)))
            .unwrap();

        assert_eq!(
            client.calls,
            [
                "pane_layout",
                "layout_export",
                "pane_read:22",
                "apply:w1",
                "focus_pane:w1:p2"
            ]
        );
        let (_, ready) = client.launch_paths.clone().unwrap();
        assert!(ready.exists());
        client.remove_launch_files();
    }

    #[test]
    fn missing_scroll_metrics_read_the_estimated_rows() {
        let target = PaneId::new("w1:p1");
        let mut client = FakeClient::with_source(false);

        launch_layout_tab_picker(&mut client, request(&target, PickerAction::Copy, None)).unwrap();

        assert!(client.calls.contains(&"pane_read:24".to_string()));
        client.remove_launch_files();
    }

    #[test]
    fn the_snapshot_keeps_rows_and_the_return_tab() {
        let target = PaneId::new("w1:p1");
        let mut client = FakeClient {
            pane_text: Some("first https://a.test\nsecond".into()),
            ..FakeClient::with_source(true)
        };

        launch_layout_tab_picker(&mut client, request(&target, PickerAction::Copy, Some(20)))
            .unwrap();

        let (snapshot_path, _) = client.launch_paths.clone().unwrap();
        let snapshot: PickerSnapshot =
            serde_json::from_slice(&std::fs::read(&snapshot_path).unwrap()).unwrap();
        assert_eq!(snapshot.source.rows, ["first https://a.test", "second"]);
        assert_eq!(snapshot.source.content_rows, 20);
        assert_eq!(snapshot.session.return_tab_id, "w1:t1");
        assert!(snapshot.session.zoom_picker);
        client.remove_launch_files();
    }

    #[test]
    fn a_target_missing_from_the_exported_tree_stops_the_launch() {
        let target = PaneId::new("w1:p1");
        let mut client = FakeClient {
            exported: Some(ExportedLayout {
                root: ExportedNode::Pane {
                    pane_id: Some("w1:p9".into()),
                },
                ..exported_layout()
            }),
            ..FakeClient::with_source(false)
        };

        let error =
            launch_layout_tab_picker(&mut client, request(&target, PickerAction::Copy, None))
                .unwrap_err();

        assert!(error.to_string().contains("exported layout"));
        assert!(client.launch_paths.is_none());
    }

    #[test]
    fn no_matches_notifies_instead_of_opening_a_dead_end_tab() {
        let target = PaneId::new("w1:p1");
        let mut client = FakeClient {
            pane_text: Some("nothing hintable here".into()),
            notification_shown: true,
            ..FakeClient::with_source(false)
        };

        launch_layout_tab_picker(&mut client, request(&target, PickerAction::OpenUrl, None))
            .unwrap();

        assert_eq!(
            client.calls,
            [
                "pane_layout",
                "layout_export",
                "pane_read:24",
                "notify:Herdr Yank: no openable URLs or files found"
            ]
        );
        assert!(client.launch_paths.is_none(), "must not write a snapshot");
    }

    #[test]
    fn suppressed_notification_falls_back_to_the_picker() {
        let target = PaneId::new("w1:p1");
        for fail_notification in [false, true] {
            let mut client = FakeClient {
                pane_text: Some("nothing hintable here".into()),
                notification_shown: false,
                fail_notification,
                ..FakeClient::with_source(false)
            };

            launch_layout_tab_picker(&mut client, request(&target, PickerAction::Copy, None))
                .unwrap();

            assert!(
                client.calls.iter().any(|call| call.starts_with("apply:")),
                "a user who cannot see the toast must still get the picker"
            );
            client.remove_launch_files();
        }
    }

    #[test]
    fn matches_present_skips_the_notification_entirely() {
        let target = PaneId::new("w1:p1");
        let mut client = FakeClient::with_source(false);

        launch_layout_tab_picker(&mut client, request(&target, PickerAction::OpenUrl, None))
            .unwrap();

        assert!(!client.calls.iter().any(|call| call.starts_with("notify:")));
        client.remove_launch_files();
    }

    #[test]
    fn failed_focus_compensates_with_returned_tab_id_and_preserves_primary_error() {
        let target = PaneId::new("w1:p1");
        let mut client = FakeClient {
            fail_focus_pane: true,
            fail_focus_tab: true,
            ..FakeClient::with_source(false)
        };

        let error =
            launch_layout_tab_picker(&mut client, request(&target, PickerAction::Copy, None))
                .unwrap_err();

        assert_eq!(error.to_string(), "focus failed");
        assert!(client.calls.contains(&"focus_tab:w1:t1".into()));
        assert!(client.calls.contains(&"close_tab:w1:t2".into()));
        let (snapshot, ready) = client.launch_paths.unwrap();
        assert!(!snapshot.exists() && !ready.exists());
    }

    #[test]
    fn cleanup_attempts_close_after_focus_failure_and_rejects_source_tab() {
        let session = picker_snapshot(false).session;
        let mut client = FakeClient {
            fail_focus_tab: true,
            ..FakeClient::default()
        };

        let error = cleanup_session(&mut client, &session, "w1:t2").unwrap_err();

        assert_eq!(error.to_string(), "tab focus failed");
        assert_eq!(client.calls, ["focus_tab:w1:t1", "close_tab:w1:t2"]);
        assert!(cleanup_session(&mut client, &session, "w1:t1")
            .unwrap_err()
            .to_string()
            .contains("refusing"));
    }

    #[test]
    fn zooms_only_when_snapshot_requests_it() {
        let mut client = FakeClient::default();
        zoom_picker(&mut client, &picker_snapshot(false), &PaneId::new("w1:p2")).unwrap();
        zoom_picker(&mut client, &picker_snapshot(true), &PaneId::new("w1:p2")).unwrap();
        assert_eq!(client.calls, ["zoom:w1:p2"]);
    }
}
