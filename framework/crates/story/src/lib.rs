use gpui::{
    AnyElement, AnyView, App, AppContext, Context, Div, Entity, EventEmitter, Focusable, Global,
    Hsla, InteractiveElement, IntoElement, ParentElement, Pixels, Render, RenderOnce, SharedString,
    StyleRefinement, Styled, Window, actions, div, prelude::FluentBuilder as _, px, rems,
};
use neutron_components::{
    ActiveTheme, IconName, Root, WindowExt,
    button::Button,
    dock::{Panel, PanelControl, PanelEvent, PanelInfo, PanelState, TitleStyle, register_panel},
    group_box::{GroupBox, GroupBoxVariants as _},
    h_flex,
    menu::PopupMenu,
    notification::Notification,
    scroll::ScrollableElement as _,
};
use serde::{Deserialize, Serialize};

mod example_support;
mod settings;
mod stories;
#[cfg(test)]
mod story_registry_tests;
pub use example_support::{
    ExampleThemes, default_example_window_size, example_failure, example_http_client_module,
    example_theme_source, focus_example, with_example_window_defaults,
};
pub use settings::{
    StorySettings, StoryUiPreferences, build_settings, story_preferences_key,
    story_preferences_module, update_locale, update_story_preferences,
};
pub use stories::*;

actions!(story, [TestAction, ShowPanelInfo]);

const PANEL_NAME: &str = "StoryContainer";

pub type StoryPanelFactory = fn(&mut Window, &mut App) -> Entity<StoryContainer>;
pub type StoryRestoreFactory = fn(&mut Window, &mut App) -> StoryRestore;

pub struct StoryDescriptor {
    pub group: &'static str,
    pub story_klass: &'static str,
    pub panel_factory: StoryPanelFactory,
    pub restore_factory: StoryRestoreFactory,
}

pub struct StoryRestore {
    pub story_klass: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub closable: bool,
    pub zoomable: Option<PanelControl>,
    pub story: AnyView,
    pub on_active: fn(AnyView, bool, &mut Window, &mut App),
}

impl StoryDescriptor {
    pub fn panel(&self, window: &mut Window, cx: &mut App) -> Entity<StoryContainer> {
        (self.panel_factory)(window, cx)
    }

    pub fn restore(&self, window: &mut Window, cx: &mut App) -> StoryRestore {
        let mut restored = (self.restore_factory)(window, cx);
        restored.story_klass = self.story_klass;
        restored
    }
}

fn panel_factory<S: Story>(window: &mut Window, cx: &mut App) -> Entity<StoryContainer> {
    StoryContainer::panel::<S>(window, cx)
}

fn restore_factory<S: Story>(window: &mut Window, cx: &mut App) -> StoryRestore {
    StoryRestore {
        story_klass: S::klass(),
        title: S::title(),
        description: S::description(),
        closable: S::closable(),
        zoomable: S::zoomable(),
        story: S::new_view(window, cx).into(),
        on_active: S::on_active_any,
    }
}

macro_rules! story_descriptor {
    ($group:literal, $story:ty, $story_klass:literal) => {
        StoryDescriptor {
            group: $group,
            story_klass: $story_klass,
            panel_factory: panel_factory::<$story>,
            restore_factory: restore_factory::<$story>,
        }
    };
}

static STORY_DESCRIPTORS: &[StoryDescriptor] = &[
    story_descriptor!("Getting Started", WelcomeStory, "WelcomeStory"),
    story_descriptor!("Components", AccordionStory, "AccordionStory"),
    story_descriptor!("Components", AlertDialogStory, "AlertDialogStory"),
    story_descriptor!("Components", AlertStory, "AlertStory"),
    story_descriptor!("Components", AppShellStory, "AppShellStory"),
    story_descriptor!("Components", AvatarStory, "AvatarStory"),
    story_descriptor!("Components", BadgeStory, "BadgeStory"),
    story_descriptor!("Components", BreadcrumbStory, "BreadcrumbStory"),
    story_descriptor!("Components", ButtonStory, "ButtonStory"),
    story_descriptor!("Components", CalendarStory, "CalendarStory"),
    story_descriptor!("Components", ChartStory, "ChartStory"),
    story_descriptor!("Components", CheckboxStory, "CheckboxStory"),
    story_descriptor!("Components", ClipboardStory, "ClipboardStory"),
    story_descriptor!("Components", CollapsibleStory, "CollapsibleStory"),
    story_descriptor!("Components", ColorPickerStory, "ColorPickerStory"),
    story_descriptor!("Components", ComboboxStory, "ComboboxStory"),
    story_descriptor!("Components", CommandPaletteStory, "CommandPaletteStory"),
    story_descriptor!("Components", DatePickerStory, "DatePickerStory"),
    story_descriptor!("Components", DescriptionListStory, "DescriptionListStory"),
    story_descriptor!("Components", DialogStory, "DialogStory"),
    story_descriptor!("Components", DividerStory, "DividerStory"),
    story_descriptor!("Components", DropdownButtonStory, "DropdownButtonStory"),
    story_descriptor!("Components", FormStory, "FormStory"),
    story_descriptor!("Components", GroupBoxStory, "GroupBoxStory"),
    story_descriptor!("Components", HoverCardStory, "HoverCardStory"),
    story_descriptor!("Components", IconStory, "IconStory"),
    story_descriptor!("Components", ImageStory, "ImageStory"),
    story_descriptor!("Components", InputStory, "InputStory"),
    story_descriptor!("Components", KbdStory, "KbdStory"),
    story_descriptor!("Components", LabelStory, "LabelStory"),
    story_descriptor!("Components", ListStory, "ListStory"),
    story_descriptor!("Components", MenuStory, "MenuStory"),
    story_descriptor!("Components", NotificationStory, "NotificationStory"),
    story_descriptor!("Components", NumberInputStory, "NumberInputStory"),
    story_descriptor!("Components", OtpInputStory, "OtpInputStory"),
    story_descriptor!("Components", PaginationStory, "PaginationStory"),
    story_descriptor!("Components", PopoverStory, "PopoverStory"),
    story_descriptor!("Components", ProgressStory, "ProgressStory"),
    story_descriptor!("Components", RadioStory, "RadioStory"),
    story_descriptor!("Components", RatingStory, "RatingStory"),
    story_descriptor!("Components", ResizableStory, "ResizableStory"),
    story_descriptor!("Components", ScrollbarStory, "ScrollbarStory"),
    story_descriptor!("Components", SelectStory, "SelectStory"),
    story_descriptor!("Components", SettingsStory, "SettingsStory"),
    story_descriptor!("Components", SheetStory, "SheetStory"),
    story_descriptor!("Components", FloatingSidebarStory, "FloatingSidebarStory"),
    story_descriptor!("Components", SidebarStory, "SidebarStory"),
    story_descriptor!("Components", SkeletonStory, "SkeletonStory"),
    story_descriptor!("Components", SliderStory, "SliderStory"),
    story_descriptor!("Components", SpinnerStory, "SpinnerStory"),
    story_descriptor!("Components", StatusBarStory, "StatusBarStory"),
    story_descriptor!("Components", StepperStory, "StepperStory"),
    story_descriptor!("Components", SwitchStory, "SwitchStory"),
    story_descriptor!("Components", TableStory, "TableStory"),
    story_descriptor!("Components", TabsStory, "TabsStory"),
    story_descriptor!("Components", TagStory, "TagStory"),
    story_descriptor!("Components", TextareaStory, "TextareaStory"),
    story_descriptor!("Components", ThemeColorsStory, "ThemeColorsStory"),
    story_descriptor!("Components", ToggleStory, "ToggleStory"),
    story_descriptor!("Components", TooltipStory, "TooltipStory"),
    story_descriptor!("Components", TreeStory, "TreeStory"),
    story_descriptor!("Components", VirtualListStory, "VirtualListStory"),
];

pub fn story_descriptors() -> &'static [StoryDescriptor] {
    STORY_DESCRIPTORS
}

pub fn story_descriptor(story_klass: &str) -> Option<&'static StoryDescriptor> {
    story_descriptors()
        .iter()
        .find(|descriptor| descriptor.story_klass == story_klass)
}

pub struct AppState {
    pub invisible_panels: Entity<Vec<SharedString>>,
}
impl AppState {
    fn init(cx: &mut App) {
        let state = Self {
            invisible_panels: cx.new(|_| Vec::new()),
        };
        cx.set_global::<AppState>(state);
    }

    pub fn global(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    pub fn global_mut(cx: &mut App) -> &mut Self {
        cx.global_mut::<Self>()
    }
}

impl Global for AppState {}

/// Initialize story-registry state: [`AppState`], per-story key bindings, and
/// the `ShowPanelInfo` handler every `StoryContainer` panel's dropdown menu
/// dispatches.
///
/// AppShell owns `neutron_components::init` and process bootstrap; this is the
/// library's own one-time registration, meant to be called from an
/// application's `story.app-state` setup module. Panel/restore factory
/// registration is a separate step; see [`init_panels`].
pub fn init_app_state(cx: &mut App) {
    AppState::init(cx);
    stories::init(cx);

    cx.on_action(|_: &ShowPanelInfo, cx: &mut App| {
        if let Some(window) = cx
            .active_window()
            .and_then(|window| window.downcast::<Root>())
        {
            cx.defer(move |cx| {
                let _ = window.update(cx, |_, window, cx| {
                    struct Info;
                    let note = Notification::new()
                        .message("You have clicked panel info.")
                        .id::<Info>();
                    window.push_notification(note, cx);
                });
            });
        }
    });
}

/// Register the dock-restorable `StoryContainer` panel/restore factory.
///
/// Kept separate from [`init_app_state`] so an application's setup module can
/// depend on `AppState` having initialized first without mixing panel
/// registration into that same step.
pub fn init_panels(cx: &mut App) {
    register_panel(cx, PANEL_NAME, |_, _, info, window, cx| {
        let story_state = match info {
            PanelInfo::Panel(value) => StoryState::from_value(value.clone()),
            _ => {
                unreachable!("Invalid PanelInfo: {:?}", info)
            }
        };

        let view = cx.new(|cx| {
            let StoryRestore {
                title,
                description,
                closable,
                zoomable,
                story,
                on_active,
                story_klass: _,
            } = story_state.to_story(window, cx);
            let mut container = StoryContainer::new(window, cx)
                .story(story, story_state.story_klass)
                .on_active(on_active);

            cx.on_focus_in(
                &container.focus_handle,
                window,
                |this: &mut StoryContainer, _, _| {
                    println!("StoryContainer focus in: {}", this.name);
                },
            )
            .detach();

            container.name = title.into();
            container.description = description.into();
            container.closable = closable;
            container.zoomable = zoomable;
            container
        });
        Box::new(view)
    });
}

#[derive(IntoElement)]
struct StorySection {
    base: Div,
    title: SharedString,
    sub_title: Vec<AnyElement>,
    children: Vec<AnyElement>,
}

impl StorySection {
    pub fn sub_title(mut self, sub_title: impl IntoElement) -> Self {
        self.sub_title.push(sub_title.into_any_element());
        self
    }

    #[allow(unused)]
    fn max_w_md(mut self) -> Self {
        self.base = self.base.max_w(rems(48.));
        self
    }

    #[allow(unused)]
    fn max_w_lg(mut self) -> Self {
        self.base = self.base.max_w(rems(64.));
        self
    }

    #[allow(unused)]
    fn max_w_xl(mut self) -> Self {
        self.base = self.base.max_w(rems(80.));
        self
    }

    #[allow(unused)]
    fn max_w_2xl(mut self) -> Self {
        self.base = self.base.max_w(rems(96.));
        self
    }
}

impl ParentElement for StorySection {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for StorySection {
    fn style(&mut self) -> &mut gpui::StyleRefinement {
        self.base.style()
    }
}

impl RenderOnce for StorySection {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        GroupBox::new()
            .id(self.title.clone())
            .outline()
            .title(
                h_flex()
                    .justify_between()
                    .w_full()
                    .gap_4()
                    .child(self.title)
                    .children(self.sub_title),
            )
            .content_style(
                StyleRefinement::default()
                    .rounded(cx.theme().radius_lg)
                    .overflow_x_hidden()
                    .items_center()
                    .justify_center(),
            )
            .child(self.base.children(self.children))
    }
}

pub(crate) fn section(title: impl Into<SharedString>) -> StorySection {
    StorySection {
        title: title.into(),
        sub_title: vec![],
        base: h_flex()
            .flex_wrap()
            .justify_center()
            .items_center()
            .w_full()
            .gap_4(),
        children: vec![],
    }
}

pub struct StoryContainer {
    focus_handle: gpui::FocusHandle,
    pub name: SharedString,
    pub title_bg: Option<Hsla>,
    pub description: SharedString,
    width: Option<gpui::Pixels>,
    height: Option<gpui::Pixels>,
    story: Option<AnyView>,
    story_klass: Option<SharedString>,
    closable: bool,
    zoomable: Option<PanelControl>,
    paddings: Pixels,
    on_active: Option<fn(AnyView, bool, &mut Window, &mut App)>,
}

#[derive(Debug)]
pub enum ContainerEvent {
    Close,
}

impl EventEmitter<ContainerEvent> for StoryContainer {}

impl StoryContainer {
    pub fn new(_window: &mut Window, cx: &mut App) -> Self {
        let focus_handle = cx.focus_handle();

        Self {
            focus_handle,
            name: "".into(),
            title_bg: None,
            description: "".into(),
            width: None,
            height: None,
            story: None,
            story_klass: None,
            closable: true,
            zoomable: Some(PanelControl::default()),
            paddings: px(16.),
            on_active: None,
        }
    }

    pub fn panel<S: Story>(window: &mut Window, cx: &mut App) -> Entity<Self> {
        let name = S::title();
        let description = S::description();
        let story = S::new_view(window, cx);
        let story_klass = S::klass();

        let view = cx.new(|cx| {
            let mut story = Self::new(window, cx)
                .story(story.into(), story_klass)
                .on_active(S::on_active_any);
            story.focus_handle = cx.focus_handle();
            story.closable = S::closable();
            story.zoomable = S::zoomable();
            story.name = name.into();
            story.description = description.into();
            story.title_bg = S::title_bg();
            story.paddings = S::paddings();
            story
        });

        view
    }

    pub fn width(mut self, width: gpui::Pixels) -> Self {
        self.width = Some(width);
        self
    }

    pub fn height(mut self, height: gpui::Pixels) -> Self {
        self.height = Some(height);
        self
    }

    pub fn story(mut self, story: AnyView, story_klass: impl Into<SharedString>) -> Self {
        self.story = Some(story);
        self.story_klass = Some(story_klass.into());
        self
    }

    pub fn on_active(mut self, on_active: fn(AnyView, bool, &mut Window, &mut App)) -> Self {
        self.on_active = Some(on_active);
        self
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StoryState {
    pub story_klass: SharedString,
}

impl StoryState {
    fn to_value(&self) -> serde_json::Value {
        serde_json::json!({
            "story_klass": self.story_klass,
        })
    }

    fn from_value(value: serde_json::Value) -> Self {
        serde_json::from_value(value).unwrap()
    }

    pub fn descriptor(&self) -> Option<&'static StoryDescriptor> {
        story_descriptor(&self.story_klass)
    }

    fn to_story(&self, window: &mut Window, cx: &mut App) -> StoryRestore {
        let descriptor = self
            .descriptor()
            .unwrap_or_else(|| unreachable!("Invalid story klass: {}", self.story_klass));
        descriptor.restore(window, cx)
    }
}

impl Panel for StoryContainer {
    fn panel_name(&self) -> &'static str {
        "StoryContainer"
    }

    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.name.clone().into_any_element()
    }

    fn title_style(&self, cx: &App) -> Option<TitleStyle> {
        if let Some(bg) = self.title_bg {
            Some(TitleStyle {
                background: bg,
                foreground: cx.theme().foreground,
            })
        } else {
            None
        }
    }

    fn closable(&self, _cx: &App) -> bool {
        self.closable
    }

    fn zoomable(&self, _cx: &App) -> Option<PanelControl> {
        self.zoomable
    }

    fn visible(&self, cx: &App) -> bool {
        !AppState::global(cx)
            .invisible_panels
            .read(cx)
            .contains(&self.name)
    }

    fn set_zoomed(&mut self, zoomed: bool, _window: &mut Window, _cx: &mut Context<Self>) {
        println!("panel: {} zoomed: {}", self.name, zoomed);
    }

    fn set_active(&mut self, active: bool, _window: &mut Window, cx: &mut Context<Self>) {
        println!("panel: {} active: {}", self.name, active);
        if let Some(on_active) = self.on_active {
            if let Some(story) = self.story.clone() {
                on_active(story, active, _window, cx);
            }
        }
    }

    fn dropdown_menu(
        &mut self,
        menu: PopupMenu,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> PopupMenu {
        menu.menu("Info", Box::new(ShowPanelInfo))
    }

    fn toolbar_buttons(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Vec<Button>> {
        Some(vec![
            Button::new("info")
                .icon(IconName::Info)
                .on_click(|_, window, cx| {
                    window.push_notification("You have clicked info button", cx);
                }),
            Button::new("search")
                .icon(IconName::Search)
                .on_click(|_, window, cx| {
                    window.push_notification("You have clicked search button", cx);
                }),
        ])
    }

    fn dump(&self, _cx: &App) -> PanelState {
        let mut state = PanelState::new(self);
        let story_state = StoryState {
            story_klass: self.story_klass.clone().unwrap(),
        };
        state.info = PanelInfo::panel(story_state.to_value());
        state
    }
}

impl EventEmitter<PanelEvent> for StoryContainer {}
impl Focusable for StoryContainer {
    fn focus_handle(&self, _: &App) -> gpui::FocusHandle {
        self.focus_handle.clone()
    }
}
impl Render for StoryContainer {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("story-container")
            .size_full()
            .overflow_y_scrollbar()
            .track_focus(&self.focus_handle)
            .when_some(self.story.clone(), |this, story| {
                this.child(div().size_full().p(self.paddings).child(story))
            })
    }
}
