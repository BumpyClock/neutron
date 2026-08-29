use anyhow::Result;
use gpui::*;
use neutron_components::{
    IconName, Sizable, WindowExt as _,
    button::{Button, ButtonVariants as _},
    dock::{ClosePanel, DockArea, DockAreaState, DockEvent, DockItem, DockPlacement, ToggleZoom},
    h_flex,
    menu::DropdownMenu,
    notification::Notification,
};

use neutron_components_app::prelude::*;
use neutron_components_app::{
    AppDeclaration, SetupContext, SetupKey, SetupModule, Surface, SurfaceKey,
};
use neutron_story::{
    AccordionStory, AppState, ButtonStory, CalendarStory, DialogStory, FormStory, IconStory,
    ImageStory, InputStory, LabelStory, ListStory, NotificationStory, PopoverStory, ProgressStory,
    ResizableStory, ScrollbarStory, SelectStory, SidebarStory, StoryContainer, SwitchStory,
    TableStory, TooltipStory, default_example_window_size, example_failure,
    example_http_client_module, example_theme_source, story_preferences_key,
    story_preferences_module, with_example_window_defaults,
};
use serde::Deserialize;
use std::{sync::Arc, time::Duration};

neutron_components_app::include_identity!();

#[derive(Action, Clone, PartialEq, Eq, Deserialize)]
#[action(namespace = story, no_json)]
pub struct AddPanel(DockPlacement);

#[derive(Action, Clone, PartialEq, Eq, Deserialize)]
#[action(namespace = story, no_json)]
pub struct TogglePanelVisible(SharedString);

actions!(story, [ToggleDockToggleButton]);

const MAIN_DOCK_AREA: DockAreaTab = DockAreaTab {
    id: "main-dock",
    version: 5,
};

#[cfg(debug_assertions)]
const STATE_FILE: &str = "target/docks.json";
#[cfg(not(debug_assertions))]
const STATE_FILE: &str = "docks.json";

pub struct StoryWorkspace {
    dock_area: Entity<DockArea>,
    last_layout_state: Option<DockAreaState>,
    toggle_button_visible: bool,
    _save_layout_task: Option<Task<()>>,
}

struct DockAreaTab {
    id: &'static str,
    version: usize,
}

struct DockLayoutResetNotification;

impl StoryWorkspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let dock_area =
            cx.new(|cx| DockArea::new(MAIN_DOCK_AREA.id, Some(MAIN_DOCK_AREA.version), window, cx));
        let weak_dock_area = dock_area.downgrade();

        match Self::load_layout(dock_area.clone(), window, cx) {
            Ok(_) => {
                println!("load layout success");
            }
            Err(err) => {
                eprintln!("load layout error: {:?}", err);
                Self::reset_default_layout(weak_dock_area, window, cx);
            }
        };

        cx.subscribe_in(
            &dock_area,
            window,
            |this, dock_area, ev: &DockEvent, window, cx| match ev {
                DockEvent::LayoutChanged => this.save_layout(dock_area, window, cx),
                _ => {}
            },
        )
        .detach();

        cx.on_app_quit({
            let dock_area = dock_area.clone();
            move |_, cx| {
                let state = dock_area.read(cx).dump(cx);
                cx.background_executor().spawn(async move {
                    // Save layout before quitting
                    Self::save_state(&state).unwrap();
                })
            }
        })
        .detach();

        Self {
            dock_area,
            last_layout_state: None,
            toggle_button_visible: true,
            _save_layout_task: None,
        }
    }

    /// The toolbar's Add Panel dropdown button: was previously hosted in the
    /// deleted `AppTitleBar`'s custom title-bar chrome; `AppShell`'s Surface
    /// now owns the native title bar, so this renders as a small toolbar row
    /// above the dock area instead. Behavior (menu items, `AddPanel`/
    /// `TogglePanelVisible`/`ToggleDockToggleButton` dispatch) is unchanged.
    fn add_panel_button(&self, cx: &Context<Self>) -> impl IntoElement {
        Button::new("add-panel")
            .icon(IconName::LayoutDashboard)
            .small()
            .ghost()
            .dropdown_menu({
                let invisible_panels = AppState::global(cx).invisible_panels.clone();

                move |menu, _, cx| {
                    menu.menu(
                        "Add Panel to Center",
                        Box::new(AddPanel(DockPlacement::Center)),
                    )
                    .separator()
                    .menu("Add Panel to Left", Box::new(AddPanel(DockPlacement::Left)))
                    .menu(
                        "Add Panel to Right",
                        Box::new(AddPanel(DockPlacement::Right)),
                    )
                    .menu(
                        "Add Panel to Bottom",
                        Box::new(AddPanel(DockPlacement::Bottom)),
                    )
                    .separator()
                    .menu(
                        "Show / Hide Dock Toggle Button",
                        Box::new(ToggleDockToggleButton),
                    )
                    .separator()
                    .menu_with_check(
                        "Sidebar",
                        !invisible_panels
                            .read(cx)
                            .contains(&SharedString::from("Sidebar")),
                        Box::new(TogglePanelVisible(SharedString::from("Sidebar"))),
                    )
                    .menu_with_check(
                        "Dialog",
                        !invisible_panels
                            .read(cx)
                            .contains(&SharedString::from("Dialog")),
                        Box::new(TogglePanelVisible(SharedString::from("Dialog"))),
                    )
                    .menu_with_check(
                        "Accordion",
                        !invisible_panels
                            .read(cx)
                            .contains(&SharedString::from("Accordion")),
                        Box::new(TogglePanelVisible(SharedString::from("Accordion"))),
                    )
                    .menu_with_check(
                        "List",
                        !invisible_panels
                            .read(cx)
                            .contains(&SharedString::from("List")),
                        Box::new(TogglePanelVisible(SharedString::from("List"))),
                    )
                }
            })
            .anchor(Corner::TopRight)
    }

    fn save_layout(
        &mut self,
        dock_area: &Entity<DockArea>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dock_area = dock_area.clone();
        self._save_layout_task = Some(cx.spawn_in(window, async move |story, window| {
            window
                .background_executor()
                .timer(Duration::from_secs(10))
                .await;

            _ = story.update_in(window, move |this, _, cx| {
                let dock_area = dock_area.read(cx);
                let state = dock_area.dump(cx);

                let last_layout_state = this.last_layout_state.clone();
                if Some(&state) == last_layout_state.as_ref() {
                    return;
                }

                Self::save_state(&state).unwrap();
                this.last_layout_state = Some(state);
            });
        }));
    }

    fn save_state(state: &DockAreaState) -> Result<()> {
        println!("Save layout...");
        let json = serde_json::to_string_pretty(state)?;
        std::fs::write(STATE_FILE, json)?;
        Ok(())
    }

    fn load_layout(
        dock_area: Entity<DockArea>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let json = std::fs::read_to_string(STATE_FILE)?;
        let state = serde_json::from_str::<DockAreaState>(&json)?;
        let weak_dock_area = dock_area.downgrade();

        match dock_area.update(cx, |dock_area, cx| dock_area.load(state, window, cx)) {
            Ok(()) => {
                dock_area.update(cx, |dock_area, cx| {
                    Self::set_dock_collapsible(dock_area, window, cx);
                });
                Ok(())
            }
            Err(neutron_components::dock::DockLoadError::IncompatibleVersion { .. }) => {
                Self::reset_default_layout(weak_dock_area, window, cx);
                window.push_notification(
                    Notification::warning(
                        "Saved dock layout was reset because it is incompatible with this version.",
                    )
                    .id::<DockLayoutResetNotification>(),
                    cx,
                );
                Ok(())
            }
            Err(err) => Err(anyhow::Error::new(err).context("load layout")),
        }
    }

    fn set_dock_collapsible(
        dock_area: &mut DockArea,
        window: &mut Window,
        cx: &mut Context<DockArea>,
    ) {
        dock_area.set_dock_collapsible(
            Edges {
                left: true,
                bottom: true,
                right: true,
                ..Default::default()
            },
            window,
            cx,
        );
    }

    fn reset_default_layout(dock_area: WeakEntity<DockArea>, window: &mut Window, cx: &mut App) {
        let dock_item = Self::init_default_layout(&dock_area, window, cx);

        let left_panels = DockItem::v_split(
            vec![
                DockItem::tab(
                    StoryContainer::panel::<ListStory>(window, cx),
                    &dock_area,
                    window,
                    cx,
                ),
                DockItem::tabs(
                    vec![
                        Arc::new(StoryContainer::panel::<ScrollbarStory>(window, cx)),
                        Arc::new(StoryContainer::panel::<AccordionStory>(window, cx)),
                    ],
                    &dock_area,
                    window,
                    cx,
                )
                .size(px(360.)),
            ],
            &dock_area,
            window,
            cx,
        );

        let bottom_panels = DockItem::v_split(
            vec![DockItem::tabs(
                vec![
                    Arc::new(StoryContainer::panel::<TooltipStory>(window, cx)),
                    Arc::new(StoryContainer::panel::<IconStory>(window, cx)),
                ],
                &dock_area,
                window,
                cx,
            )],
            &dock_area,
            window,
            cx,
        );

        let right_panels = DockItem::v_split(
            vec![
                DockItem::tab(
                    StoryContainer::panel::<ImageStory>(window, cx),
                    &dock_area,
                    window,
                    cx,
                ),
                DockItem::tab(
                    StoryContainer::panel::<IconStory>(window, cx),
                    &dock_area,
                    window,
                    cx,
                ),
            ],
            &dock_area,
            window,
            cx,
        );

        _ = dock_area.update(cx, |view, cx| {
            view.set_version(MAIN_DOCK_AREA.version, window, cx);
            view.set_center(dock_item, window, cx);
            view.set_left_dock(left_panels, Some(px(350.)), true, window, cx);
            view.set_bottom_dock(bottom_panels, Some(px(200.)), true, window, cx);
            view.set_right_dock(right_panels, Some(px(320.)), true, window, cx);
            Self::set_dock_collapsible(view, window, cx);

            Self::save_state(&view.dump(cx)).unwrap();
        });
    }

    fn init_default_layout(
        dock_area: &WeakEntity<DockArea>,
        window: &mut Window,
        cx: &mut App,
    ) -> DockItem {
        DockItem::v_split(
            vec![DockItem::tabs(
                vec![
                    Arc::new(StoryContainer::panel::<ButtonStory>(window, cx)),
                    Arc::new(StoryContainer::panel::<InputStory>(window, cx)),
                    Arc::new(StoryContainer::panel::<SelectStory>(window, cx)),
                    Arc::new(StoryContainer::panel::<LabelStory>(window, cx)),
                    Arc::new(StoryContainer::panel::<DialogStory>(window, cx)),
                    Arc::new(StoryContainer::panel::<PopoverStory>(window, cx)),
                    Arc::new(StoryContainer::panel::<SwitchStory>(window, cx)),
                    Arc::new(StoryContainer::panel::<ProgressStory>(window, cx)),
                    Arc::new(StoryContainer::panel::<TableStory>(window, cx)),
                    Arc::new(StoryContainer::panel::<ImageStory>(window, cx)),
                    Arc::new(StoryContainer::panel::<IconStory>(window, cx)),
                    Arc::new(StoryContainer::panel::<TooltipStory>(window, cx)),
                    Arc::new(StoryContainer::panel::<CalendarStory>(window, cx)),
                    Arc::new(StoryContainer::panel::<ResizableStory>(window, cx)),
                    Arc::new(StoryContainer::panel::<ScrollbarStory>(window, cx)),
                    Arc::new(StoryContainer::panel::<AccordionStory>(window, cx)),
                    Arc::new(StoryContainer::panel::<SidebarStory>(window, cx)),
                    Arc::new(StoryContainer::panel::<FormStory>(window, cx)),
                    Arc::new(StoryContainer::panel::<NotificationStory>(window, cx)),
                ],
                &dock_area,
                window,
                cx,
            )],
            &dock_area,
            window,
            cx,
        )
    }

    fn on_action_add_panel(
        &mut self,
        action: &AddPanel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Random pick up a panel to add
        let panel = match rand::random::<usize>() % 18 {
            0 => Arc::new(StoryContainer::panel::<ButtonStory>(window, cx)),
            1 => Arc::new(StoryContainer::panel::<InputStory>(window, cx)),
            2 => Arc::new(StoryContainer::panel::<SelectStory>(window, cx)),
            3 => Arc::new(StoryContainer::panel::<LabelStory>(window, cx)),
            4 => Arc::new(StoryContainer::panel::<DialogStory>(window, cx)),
            5 => Arc::new(StoryContainer::panel::<PopoverStory>(window, cx)),
            6 => Arc::new(StoryContainer::panel::<SwitchStory>(window, cx)),
            7 => Arc::new(StoryContainer::panel::<ProgressStory>(window, cx)),
            8 => Arc::new(StoryContainer::panel::<TableStory>(window, cx)),
            9 => Arc::new(StoryContainer::panel::<ImageStory>(window, cx)),
            10 => Arc::new(StoryContainer::panel::<IconStory>(window, cx)),
            11 => Arc::new(StoryContainer::panel::<TooltipStory>(window, cx)),
            12 => Arc::new(StoryContainer::panel::<ProgressStory>(window, cx)),
            13 => Arc::new(StoryContainer::panel::<CalendarStory>(window, cx)),
            14 => Arc::new(StoryContainer::panel::<ResizableStory>(window, cx)),
            15 => Arc::new(StoryContainer::panel::<ScrollbarStory>(window, cx)),
            16 => Arc::new(StoryContainer::panel::<AccordionStory>(window, cx)),
            _ => Arc::new(StoryContainer::panel::<ButtonStory>(window, cx)),
        };

        self.dock_area.update(cx, |dock_area, cx| {
            dock_area.add_panel(panel, action.0, None, window, cx);
        });
    }

    fn on_action_toggle_panel_visible(
        &mut self,
        action: &TogglePanelVisible,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let panel_name = action.0.clone();
        let invisible_panels = AppState::global(cx).invisible_panels.clone();
        invisible_panels.update(cx, |names, cx| {
            if names.contains(&panel_name) {
                names.retain(|id| id != &panel_name);
            } else {
                names.push(panel_name);
            }
            cx.notify();
        });
        cx.notify();
    }

    fn on_action_toggle_dock_toggle_button(
        &mut self,
        _: &ToggleDockToggleButton,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_button_visible = !self.toggle_button_visible;

        self.dock_area.update(cx, |dock_area, cx| {
            dock_area.set_toggle_button_visible(self.toggle_button_visible, cx);
        });
    }
}

impl Render for StoryWorkspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("story-workspace")
            .on_action(cx.listener(Self::on_action_add_panel))
            .on_action(cx.listener(Self::on_action_toggle_panel_visible))
            .on_action(cx.listener(Self::on_action_toggle_dock_toggle_button))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(
                h_flex()
                    .id("story-workspace-toolbar")
                    .w_full()
                    .justify_end()
                    .p_2()
                    .child(self.add_panel_button(cx)),
            )
            .child(self.dock_area.clone())
    }
}

fn build_story_workspace(_args: &(), window: &mut Window, cx: &mut App) -> Entity<StoryWorkspace> {
    cx.new(|cx| StoryWorkspace::new(window, cx))
}

fn primary_surface() -> Surface<StoryWorkspace, ()> {
    with_example_window_defaults(
        Surface::new(SurfaceKey::primary(), build_story_workspace).title("GPUI App"),
        default_example_window_size(),
    )
    .min_size(size(px(640.0), px(480.0)))
}

/// Registers the `AppState` global and the `StoryContainer` panel/restore
/// factory: `AppState` first, matching the deleted `init()`'s order, since
/// `on_action_toggle_panel_visible` and the add-panel dropdown both read it,
/// and dock restoration reads the registry it restores from.
fn init_panels_setup(cx: &mut SetupContext<'_>) -> anyhow::Result<()> {
    neutron_story::init_app_state(cx.app());
    neutron_story::init_panels(cx.app());
    Ok(())
}

const PANELS_SETUP_KEY: SetupKey = SetupKey::new("dock-example.panels");

/// The deleted global `shift-escape`/`ctrl-w` bindings for `ToggleZoom` and
/// `ClosePanel`, installed once at startup exactly as before.
fn init_dock_bindings(cx: &mut SetupContext<'_>) -> anyhow::Result<()> {
    cx.app().bind_keys(vec![
        KeyBinding::new("shift-escape", ToggleZoom, None),
        KeyBinding::new("ctrl-w", ClosePanel, None),
    ]);
    Ok(())
}

/// The `dock` example's `DesktopApp` declaration. Zero-sized: `AppShell`
/// never creates or retains an application object.
struct DockExampleApp;

impl DesktopApp for DockExampleApp {
    fn declaration() -> AppDeclaration {
        AppDeclaration::new(APP_IDENTITY)
            .initial_activation(InitialActivation::Forced)
            .theme(example_theme_source())
            .settings_store::<neutron_story::StoryUiPreferences>(story_preferences_key())
            .setup(example_http_client_module())
            .setup(story_preferences_module())
            .setup(SetupModule::new(PANELS_SETUP_KEY, init_panels_setup))
            .setup(
                SetupModule::new(SetupKey::new("dock-example.bindings"), init_dock_bindings)
                    .after(PANELS_SETUP_KEY),
            )
            .primary_surface(primary_surface())
    }
}

fn main() -> std::process::ExitCode {
    match AppShell::run::<DockExampleApp>() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => example_failure("dock example", error),
    }
}
