use crate::model::{PaneId, Rect, SplitDirection};
use anyhow::{Context, Result};
use serde::Deserialize;

/// Herdr tab layout snapshot returned by `pane.layout` in Herdr-global coordinates.
#[derive(Debug, Clone, Deserialize)]
pub struct LayoutSnapshot {
    pub area: Rect,
    pub focused_pane_id: Option<String>,
    pub panes: Vec<LayoutPane>,
    #[serde(default)]
    pub zoomed: bool,
}

/// Pane entry within a Herdr layout snapshot.
#[derive(Debug, Clone, Deserialize)]
pub struct LayoutPane {
    #[serde(default)]
    pub focused: bool,
    pub pane_id: String,
    pub rect: Rect,
}

/// Portable tab layout tree returned by `layout.export`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ExportedLayout {
    pub workspace_id: String,
    pub tab_id: String,
    pub root: ExportedNode,
}

/// One node of an exported Herdr layout tree.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExportedNode {
    Pane {
        #[serde(default)]
        pane_id: Option<String>,
    },
    Split {
        direction: SplitDirection,
        ratio: f32,
        first: Box<ExportedNode>,
        second: Box<ExportedNode>,
    },
}

impl ExportedNode {
    /// Whether the tree has a pane with this id.
    pub fn contains_pane(&self, target: &PaneId) -> bool {
        match self {
            Self::Pane { pane_id } => pane_id.as_deref() == Some(target.0.as_str()),
            Self::Split { first, second, .. } => {
                first.contains_pane(target) || second.contains_pane(target)
            }
        }
    }
}

/// Content size of the target pane in terminal cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentSize {
    pub columns: u16,
    pub rows: u16,
}

/// Whether the target pane fills its tab because the tab is zoomed on it.
pub fn target_is_zoomed(layout: &LayoutSnapshot, target: &PaneId) -> bool {
    layout.zoomed
        && (layout.focused_pane_id.as_deref() == Some(target.0.as_str())
            || layout
                .panes
                .iter()
                .any(|pane| pane.pane_id == target.0 && pane.focused))
}

/// Derives the content size of the target pane for the launch step.
///
/// Herdr reports the content rows (`scroll.viewport_rows`) but not the content columns. The
/// columns are an estimate: the side borders are as wide as the top and bottom borders
/// together, and one scrollbar column follows. Only the launch pre-check uses the estimate,
/// because the picker reads its real terminal width.
pub fn target_content_size(
    layout: &LayoutSnapshot,
    target: &PaneId,
    viewport_rows: Option<u16>,
) -> Result<ContentSize> {
    let pane = layout
        .panes
        .iter()
        .find(|pane| pane.pane_id == target.0)
        .with_context(|| format!("target pane {target} was not present in layout"))?;
    let outer = if target_is_zoomed(layout, target) {
        layout.area
    } else {
        pane.rect
    };

    // Without scroll metrics, assume the default Herdr frame: a 1-cell border around split panes.
    let rows = viewport_rows.unwrap_or_else(|| {
        outer
            .height
            .saturating_sub(2 * u16::from(layout.panes.len() > 1))
    });
    let border = outer.height.saturating_sub(rows);
    let columns = outer.width.saturating_sub(border).saturating_sub(1);

    Ok(ContentSize { columns, rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(id: &str, rect: Rect, focused: bool) -> LayoutPane {
        LayoutPane {
            focused,
            pane_id: id.to_string(),
            rect,
        }
    }

    fn layout(panes: Vec<LayoutPane>, zoomed: bool) -> LayoutSnapshot {
        LayoutSnapshot {
            area: Rect::new(0, 0, 177, 46),
            focused_pane_id: Some("p1".to_string()),
            panes,
            zoomed,
        }
    }

    fn split_layout(zoomed: bool) -> LayoutSnapshot {
        layout(
            vec![
                pane("p1", Rect::new(0, 0, 89, 46), true),
                pane("p2", Rect::new(89, 0, 88, 14), false),
            ],
            zoomed,
        )
    }

    // The expected columns are the `stty size` values of live Herdr 0.9.1 panes.
    #[test]
    fn estimates_columns_that_match_live_herdr_panes() {
        let size = target_content_size(&split_layout(false), &PaneId::new("p1"), Some(44));
        assert_eq!(
            size.unwrap(),
            ContentSize {
                columns: 86,
                rows: 44
            }
        );

        let size = target_content_size(&split_layout(false), &PaneId::new("p2"), Some(12));
        assert_eq!(
            size.unwrap(),
            ContentSize {
                columns: 85,
                rows: 12
            }
        );
    }

    #[test]
    fn a_zoomed_target_uses_the_full_tab_area() {
        let mut layout = split_layout(true);
        layout.area = Rect::new(0, 0, 143, 46);

        let size = target_content_size(&layout, &PaneId::new("p1"), Some(44)).unwrap();

        assert!(target_is_zoomed(&layout, &PaneId::new("p1")));
        assert!(!target_is_zoomed(&layout, &PaneId::new("p2")));
        assert_eq!(
            size,
            ContentSize {
                columns: 140,
                rows: 44
            }
        );
    }

    #[test]
    fn borderless_panes_get_wider_estimates() {
        // With `ui.pane_borders = "off"` Herdr reports all rows of the pane as content rows.
        let size = target_content_size(&split_layout(false), &PaneId::new("p1"), Some(46));

        assert_eq!(
            size.unwrap(),
            ContentSize {
                columns: 88,
                rows: 46
            }
        );
    }

    #[test]
    fn missing_scroll_metrics_fall_back_to_the_default_frame() {
        let size = target_content_size(&split_layout(false), &PaneId::new("p1"), None).unwrap();
        let single = layout(vec![pane("p1", Rect::new(0, 0, 177, 46), true)], false);
        let single = target_content_size(&single, &PaneId::new("p1"), None).unwrap();

        assert_eq!(
            size,
            ContentSize {
                columns: 86,
                rows: 44
            }
        );
        assert_eq!(
            single,
            ContentSize {
                columns: 176,
                rows: 46
            }
        );
    }

    #[test]
    fn target_must_be_present() {
        let error =
            target_content_size(&split_layout(false), &PaneId::new("missing"), None).unwrap_err();

        assert!(error.to_string().contains("target pane missing"));
    }

    #[test]
    fn exported_tree_reports_its_panes() {
        let tree = ExportedNode::Split {
            direction: SplitDirection::Right,
            ratio: 0.5,
            first: Box::new(ExportedNode::Pane {
                pane_id: Some("p1".into()),
            }),
            second: Box::new(ExportedNode::Pane { pane_id: None }),
        };

        assert!(tree.contains_pane(&PaneId::new("p1")));
        assert!(!tree.contains_pane(&PaneId::new("p2")));
    }
}
