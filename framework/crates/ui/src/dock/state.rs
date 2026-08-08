use gpui::{App, AppContext, Axis, Bounds, Entity, Pixels, WeakEntity, Window, point, px, size};
use itertools::Itertools as _;
use serde::{Deserialize, Serialize};
use std::{error::Error, fmt};

use super::{Dock, DockArea, DockItem, DockPlacement, Panel, PanelRegistry};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DockLoadError {
    IncompatibleVersion {
        expected: Option<usize>,
        found: Option<usize>,
    },
    UnknownPanel {
        panel_name: String,
    },
    InvalidTabsPayload {
        panel_name: String,
        child_panel_name: String,
    },
}

impl fmt::Display for DockLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IncompatibleVersion { expected, found } => write!(
                f,
                "dock layout version mismatch: expected {:?}, found {:?}",
                expected, found
            ),
            Self::UnknownPanel { panel_name } => {
                write!(f, "dock layout references unknown panel `{panel_name}`")
            }
            Self::InvalidTabsPayload {
                panel_name,
                child_panel_name,
            } => write!(
                f,
                "dock tabs payload `{panel_name}` contains non-panel child `{child_panel_name}`",
            ),
        }
    }
}

impl Error for DockLoadError {}

/// Used to serialize and deserialize the DockArea
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct DockAreaState {
    /// The version is used to mark this persisted state is compatible with the current version
    /// For example, some times we many totally changed the structure of the Panel,
    /// then we can compare the version to decide whether we can use the state or ignore.
    #[serde(default)]
    pub version: Option<usize>,
    pub center: PanelState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left_dock: Option<DockState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right_dock: Option<DockState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bottom_dock: Option<DockState>,
}

/// Used to serialize and deserialize the Dock
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DockState {
    panel: PanelState,
    placement: DockPlacement,
    size: Pixels,
    open: bool,
}

impl DockState {
    pub fn new(dock: Entity<Dock>, cx: &App) -> Self {
        let dock = dock.read(cx);

        Self {
            placement: dock.placement,
            size: dock.size,
            open: dock.open,
            panel: dock.panel.view().dump(cx),
        }
    }

    /// Convert the DockState to Dock
    pub fn to_dock(
        &self,
        dock_area: WeakEntity<DockArea>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<Entity<Dock>, DockLoadError> {
        let item = self.panel.to_item(dock_area.clone(), window, cx)?;
        Ok(cx.new(|cx| {
            Dock::from_state(
                dock_area.clone(),
                self.placement,
                self.size,
                item,
                self.open,
                window,
                cx,
            )
        }))
    }
}

/// Used to serialize and deserialize the DockerItem
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PanelState {
    pub panel_name: String,
    pub children: Vec<PanelState>,
    pub info: PanelInfo,
}

#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
pub struct TileMeta {
    pub bounds: Bounds<Pixels>,
    pub z_index: usize,
}

impl Default for TileMeta {
    fn default() -> Self {
        Self {
            bounds: Bounds {
                origin: point(px(10.), px(10.)),
                size: size(px(200.), px(200.)),
            },
            z_index: 0,
        }
    }
}

impl From<Bounds<Pixels>> for TileMeta {
    fn from(bounds: Bounds<Pixels>) -> Self {
        Self { bounds, z_index: 0 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PanelInfo {
    #[serde(rename = "stack")]
    Stack {
        sizes: Vec<Pixels>,
        axis: usize, // 0 for horizontal, 1 for vertical
    },
    #[serde(rename = "tabs")]
    Tabs { active_index: usize },
    #[serde(rename = "panel")]
    Panel(serde_json::Value),
    #[serde(rename = "tiles")]
    Tiles { metas: Vec<TileMeta> },
}

impl PanelInfo {
    pub fn stack(sizes: Vec<Pixels>, axis: Axis) -> Self {
        Self::Stack {
            sizes,
            axis: if axis == Axis::Horizontal { 0 } else { 1 },
        }
    }

    pub fn tabs(active_index: usize) -> Self {
        Self::Tabs { active_index }
    }

    pub fn panel(info: serde_json::Value) -> Self {
        Self::Panel(info)
    }

    pub fn tiles(metas: Vec<TileMeta>) -> Self {
        Self::Tiles { metas }
    }

    pub fn axis(&self) -> Option<Axis> {
        match self {
            Self::Stack { axis, .. } => Some(if *axis == 0 {
                Axis::Horizontal
            } else {
                Axis::Vertical
            }),
            _ => None,
        }
    }

    pub fn sizes(&self) -> Option<&Vec<Pixels>> {
        match self {
            Self::Stack { sizes, .. } => Some(sizes),
            _ => None,
        }
    }

    pub fn active_index(&self) -> Option<usize> {
        match self {
            Self::Tabs { active_index } => Some(*active_index),
            _ => None,
        }
    }
}

impl Default for PanelState {
    fn default() -> Self {
        Self {
            panel_name: "".to_string(),
            children: Vec::new(),
            info: PanelInfo::Panel(serde_json::Value::Null),
        }
    }
}

impl PanelState {
    fn validate_with<F>(&self, panel_exists: F) -> Result<(), DockLoadError>
    where
        F: Fn(&str) -> bool + Copy,
    {
        for child in &self.children {
            child.validate_with(panel_exists)?;
        }

        if matches!(self.info, PanelInfo::Tabs { .. }) {
            for child in &self.children {
                if !matches!(child.info, PanelInfo::Panel(_)) {
                    return Err(DockLoadError::InvalidTabsPayload {
                        panel_name: self.panel_name.clone(),
                        child_panel_name: child.panel_name.clone(),
                    });
                }
            }
        }

        if matches!(self.info, PanelInfo::Panel(_)) && !panel_exists(&self.panel_name) {
            return Err(DockLoadError::UnknownPanel {
                panel_name: self.panel_name.clone(),
            });
        }

        Ok(())
    }

    pub fn new<P: Panel>(panel: &P) -> Self {
        Self {
            panel_name: panel.panel_name().to_string(),
            ..Default::default()
        }
    }

    pub fn add_child(&mut self, panel: PanelState) {
        self.children.push(panel);
    }

    pub fn to_item(
        &self,
        dock_area: WeakEntity<DockArea>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<DockItem, DockLoadError> {
        let info = self.info.clone();

        let items = self
            .children
            .iter()
            .map(|child| child.to_item(dock_area.clone(), window, cx))
            .collect::<Result<Vec<_>, _>>()?;

        match info {
            PanelInfo::Stack { sizes, axis } => {
                let axis = if axis == 0 {
                    Axis::Horizontal
                } else {
                    Axis::Vertical
                };
                let sizes = sizes.iter().map(|s| Some(*s)).collect_vec();
                Ok(DockItem::split_with_sizes(
                    axis, items, sizes, &dock_area, window, cx,
                ))
            }
            PanelInfo::Tabs { active_index } => {
                if items.len() == 1 {
                    return match &items[0] {
                        DockItem::Tabs { .. } => Ok(items[0].clone()),
                        _ => Err(DockLoadError::InvalidTabsPayload {
                            panel_name: self.panel_name.clone(),
                            child_panel_name: self.children[0].panel_name.clone(),
                        }),
                    };
                }

                let items = items
                    .into_iter()
                    .zip(self.children.iter())
                    .map(|(item, child)| match item {
                        DockItem::Tabs { items, .. } => Ok(items),
                        _ => Err(DockLoadError::InvalidTabsPayload {
                            panel_name: self.panel_name.clone(),
                            child_panel_name: child.panel_name.clone(),
                        }),
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .flatten()
                    .collect_vec();

                Ok(DockItem::tabs(items, &dock_area, window, cx).active_index(active_index, cx))
            }
            PanelInfo::Panel(_) => {
                let view = PanelRegistry::build_panel(
                    &self.panel_name,
                    dock_area.clone(),
                    self,
                    &info,
                    window,
                    cx,
                );
                Ok(DockItem::tabs(vec![view.into()], &dock_area, window, cx))
            }
            PanelInfo::Tiles { metas } => Ok(DockItem::tiles(items, metas, &dock_area, window, cx)),
        }
    }
}

impl DockAreaState {
    pub fn validate_for_load(
        &self,
        expected_version: Option<usize>,
        cx: &App,
    ) -> Result<(), DockLoadError> {
        self.validate_for_load_with(expected_version, |panel_name| {
            PanelRegistry::global(cx).items.contains_key(panel_name)
        })
    }

    fn validate_for_load_with<F>(
        &self,
        expected_version: Option<usize>,
        panel_exists: F,
    ) -> Result<(), DockLoadError>
    where
        F: Fn(&str) -> bool + Copy,
    {
        if self.version != expected_version {
            return Err(DockLoadError::IncompatibleVersion {
                expected: expected_version,
                found: self.version,
            });
        }

        self.center.validate_with(panel_exists)?;

        for dock in [
            self.left_dock.as_ref(),
            self.right_dock.as_ref(),
            self.bottom_dock.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            dock.panel.validate_with(panel_exists)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use gpui::px;

    use super::*;

    #[test]
    fn test_deserialize_item_state() {
        let json = include_str!("../../tests/fixtures/layout.json");
        let state: DockAreaState = serde_json::from_str(json).unwrap();
        assert_eq!(state.version, None);
        assert_eq!(state.center.panel_name, "StackPanel");
        assert_eq!(state.center.children.len(), 2);
        assert_eq!(state.center.children[0].panel_name, "TabPanel");
        assert_eq!(state.center.children[1].children.len(), 1);
        assert_eq!(
            state.center.children[1].children[0].panel_name,
            "StoryContainer"
        );
        assert_eq!(state.center.children[1].panel_name, "TabPanel");

        let left_dock = state.left_dock.unwrap();
        assert_eq!(left_dock.open, true);
        assert_eq!(left_dock.size, px(350.0));
        assert_eq!(left_dock.placement, DockPlacement::Left);
        assert_eq!(left_dock.panel.panel_name, "TabPanel");
        assert_eq!(left_dock.panel.children.len(), 1);
        assert_eq!(left_dock.panel.children[0].panel_name, "StoryContainer");

        let bottom_dock = state.bottom_dock.unwrap();
        assert_eq!(bottom_dock.open, true);
        assert_eq!(bottom_dock.size, px(200.0));
        assert_eq!(bottom_dock.panel.panel_name, "TabPanel");
        assert_eq!(bottom_dock.panel.children.len(), 2);
        assert_eq!(bottom_dock.panel.children[0].panel_name, "StoryContainer");

        let right_dock = state.right_dock.unwrap();
        assert_eq!(right_dock.open, true);
        assert_eq!(right_dock.size, px(320.0));
        assert_eq!(right_dock.panel.panel_name, "TabPanel");
        assert_eq!(right_dock.panel.children.len(), 1);
        assert_eq!(right_dock.panel.children[0].panel_name, "StoryContainer");
    }

    #[test]
    fn test_validate_for_load_rejects_version_mismatch() {
        let state = DockAreaState {
            version: Some(1),
            ..Default::default()
        };

        let err = state.validate_for_load_with(Some(2), |_| true).unwrap_err();
        assert_eq!(
            err,
            DockLoadError::IncompatibleVersion {
                expected: Some(2),
                found: Some(1),
            }
        );
    }

    #[test]
    fn test_validate_for_load_rejects_non_panel_tabs_children() {
        let state = DockAreaState {
            version: Some(5),
            center: PanelState {
                panel_name: "TabPanel".into(),
                children: vec![PanelState {
                    panel_name: "StackPanel".into(),
                    children: vec![],
                    info: PanelInfo::stack(vec![], Axis::Horizontal),
                }],
                info: PanelInfo::tabs(0),
            },
            ..Default::default()
        };

        let err = state.validate_for_load_with(Some(5), |_| true).unwrap_err();
        assert_eq!(
            err,
            DockLoadError::InvalidTabsPayload {
                panel_name: "TabPanel".into(),
                child_panel_name: "StackPanel".into(),
            }
        );
    }

    #[test]
    fn test_validate_for_load_rejects_unknown_panel() {
        let state = DockAreaState {
            version: Some(5),
            center: PanelState {
                panel_name: "StoryContainer".into(),
                children: vec![],
                info: PanelInfo::panel(serde_json::Value::Null),
            },
            ..Default::default()
        };

        let err = state
            .validate_for_load_with(Some(5), |_| false)
            .unwrap_err();
        assert_eq!(
            err,
            DockLoadError::UnknownPanel {
                panel_name: "StoryContainer".into(),
            }
        );
    }

    #[test]
    fn test_layout_state_round_trips_through_json() {
        let json = include_str!("../../tests/fixtures/layout.json");
        let state: DockAreaState = serde_json::from_str(json).unwrap();
        let round_trip = serde_json::to_string(&state).unwrap();
        let restored: DockAreaState = serde_json::from_str(&round_trip).unwrap();

        assert_eq!(restored, state);
        restored.validate_for_load_with(None, |_| true).unwrap();
    }
}
