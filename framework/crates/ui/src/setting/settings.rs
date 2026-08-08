use std::rc::Rc;

use crate::{
    FloatingSidebar, IconName, Side, Sizable, Size, StyledExt,
    group_box::GroupBoxVariant,
    input::{Input, InputState},
    resizable::{h_resizable, resizable_panel},
    setting::{SettingGroup, SettingPage},
    sidebar::{Sidebar, SidebarMenu, SidebarMenuItem},
};
use gpui::{
    AnyElement, App, AppContext as _, Axis, ElementId, Entity, IntoElement, ParentElement as _,
    Pixels, RenderOnce, SharedString, StyleRefinement, Styled, Window, div,
    prelude::FluentBuilder as _, px, relative,
};
use rust_i18n::t;

/// The settings structure containing multiple pages for app settings.
///
/// The hierarchy of settings is as follows:
///
/// ```ignore
/// Settings
///   SettingPage     <- The single active page displayed
///     SettingGroup
///       SettingItem
///         Label
///         SettingControl (e.g., Switch, Dropdown, Input)
/// ```
#[derive(IntoElement)]
pub struct Settings {
    id: ElementId,
    pages: Vec<SettingPage>,
    group_variant: GroupBoxVariant,
    size: Size,
    sidebar_width: Pixels,
    use_sidebar_shell: bool,
    safe_area_top: Pixels,
    sidebar_shell_inset: Pixels,
    sidebar_title: Option<SharedString>,
    page_header_action: Option<Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>>,
    sidebar_style: StyleRefinement,
    default_selected_index: SelectIndex,
    header_style: StyleRefinement,
}

impl Settings {
    /// Create a new settings with the given ID.
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            pages: vec![],
            group_variant: GroupBoxVariant::default(),
            size: Size::default(),
            sidebar_width: px(250.0),
            use_sidebar_shell: false,
            safe_area_top: px(0.0),
            sidebar_shell_inset: px(8.0),
            sidebar_title: None,
            page_header_action: None,
            sidebar_style: StyleRefinement::default(),
            default_selected_index: SelectIndex::default(),
            header_style: StyleRefinement::default(),
        }
    }

    /// Set the width of the sidebar, default is `250px`.
    pub fn sidebar_width(mut self, width: impl Into<Pixels>) -> Self {
        self.sidebar_width = width.into();
        self
    }

    /// Toggle rendering the left panel with FloatingSidebar.
    pub fn use_sidebar_shell(mut self, enabled: bool) -> Self {
        self.use_sidebar_shell = enabled;
        self
    }

    /// Set top safe-area offset to avoid titlebar control collisions.
    pub fn safe_area_top(mut self, inset: impl Into<Pixels>) -> Self {
        self.safe_area_top = inset.into();
        self
    }

    /// Set the floating sidebar inset when `use_sidebar_shell` is enabled.
    ///
    /// Default is `8px`.
    pub fn sidebar_shell_inset(mut self, inset: impl Into<Pixels>) -> Self {
        self.sidebar_shell_inset = inset.into();
        self
    }

    /// Set an optional title displayed at the top of the sidebar.
    pub fn sidebar_title(mut self, title: impl Into<SharedString>) -> Self {
        self.sidebar_title = Some(title.into());
        self
    }

    /// Render a custom action in the active page header.
    pub fn page_header_action(
        mut self,
        action: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        self.page_header_action = Some(Rc::new(action));
        self
    }

    /// Add a page to the settings.
    pub fn page(mut self, page: SettingPage) -> Self {
        self.pages.push(page);
        self
    }

    /// Add pages to the settings.
    pub fn pages(mut self, pages: impl IntoIterator<Item = SettingPage>) -> Self {
        self.pages.extend(pages);
        self
    }

    /// Set the default variant for all setting groups.
    ///
    /// All setting groups will use this variant unless overridden individually.
    pub fn with_group_variant(mut self, variant: GroupBoxVariant) -> Self {
        self.group_variant = variant;
        self
    }

    /// Set the style refinement for the sidebar.
    pub fn sidebar_style(mut self, style: &StyleRefinement) -> Self {
        self.sidebar_style = style.clone();
        self
    }

    /// Set the default index of the page to be selected.
    pub fn default_selected_index(mut self, index: SelectIndex) -> Self {
        self.default_selected_index = index;
        self
    }

    /// Set the style refinement for the header.
    pub fn header_style(mut self, style: &StyleRefinement) -> Self {
        self.header_style = style.clone();
        self
    }

    fn filtered_pages(&self, query: &str, cx: &App) -> Vec<SettingPage> {
        self.pages
            .iter()
            .filter_map(|page| {
                let filtered_groups: Vec<SettingGroup> = page
                    .groups
                    .iter()
                    .filter_map(|group| {
                        let mut group = group.clone();
                        group.items = group
                            .items
                            .iter()
                            .filter(|item| item.is_match(&query, cx))
                            .cloned()
                            .collect();
                        if group.items.is_empty() {
                            None
                        } else {
                            Some(group)
                        }
                    })
                    .collect();
                let mut page = page.clone();
                page.groups = filtered_groups;
                if page.groups.is_empty() {
                    None
                } else {
                    Some(page)
                }
            })
            .collect()
    }

    fn render_active_page(
        &self,
        state: &Entity<SettingsState>,
        pages: &Vec<SettingPage>,
        options: &RenderOptions,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        let selected_index = state.read(cx).selected_index;

        for (ix, page) in pages.into_iter().enumerate() {
            if selected_index.page_ix == ix {
                let page_header_action = self
                    .page_header_action
                    .as_ref()
                    .map(|action| action(window, cx));
                return page
                    .render(ix, state, &options, page_header_action, window, cx)
                    .into_any_element();
            }
        }

        return div().into_any_element();
    }

    fn render_sidebar_header(
        &self,
        state: &Entity<SettingsState>,
        cx: &mut App,
    ) -> impl IntoElement {
        let search_input = state.read(cx).search_input.clone();
        let sidebar_title = self.sidebar_title.clone();
        let sidebar_title_top_offset = (self.safe_area_top - self.sidebar_shell_inset).max(px(0.0));

        div()
            .w_full()
            .refine_style(&self.header_style)
            .when_some(sidebar_title, |this, title| {
                this.child(div().h(sidebar_title_top_offset))
                    .child(div().px_1().pb_2().font_semibold().child(title))
            })
            .child(Input::new(&search_input).prefix(IconName::Search))
    }

    fn render_sidebar_menu(
        &self,
        state: &Entity<SettingsState>,
        pages: &Vec<SettingPage>,
        cx: &mut App,
    ) -> SidebarMenu {
        let selected_index = state.read(cx).selected_index;

        SidebarMenu::new().children(pages.iter().enumerate().map(|(page_ix, page)| {
            let is_page_active =
                selected_index.page_ix == page_ix && selected_index.group_ix.is_none();
            SidebarMenuItem::new(page.title.clone())
                .default_open(page.default_open)
                .active(is_page_active)
                .on_click({
                    let state = state.clone();
                    move |_, _, cx| {
                        state.update(cx, |state, cx| {
                            state.selected_index = SelectIndex {
                                page_ix,
                                ..Default::default()
                            };
                            cx.notify();
                        })
                    }
                })
                .when(page.groups.len() > 1, |this| {
                    this.children(
                        page.groups
                            .iter()
                            .filter(|g| g.title.is_some())
                            .enumerate()
                            .map(|(group_ix, group)| {
                                let is_active = selected_index.page_ix == page_ix
                                    && selected_index.group_ix == Some(group_ix);
                                let title = group.title.clone().unwrap_or_default();

                                SidebarMenuItem::new(title).active(is_active).on_click({
                                    let state = state.clone();
                                    move |_, _, cx| {
                                        state.update(cx, |state, cx| {
                                            state.selected_index = SelectIndex {
                                                page_ix,
                                                group_ix: Some(group_ix),
                                            };
                                            state.deferred_scroll_group_ix = Some(group_ix);
                                            cx.notify();
                                        })
                                    }
                                })
                            }),
                    )
                })
        }))
    }

    fn render_sidebar(
        &self,
        state: &Entity<SettingsState>,
        pages: &Vec<SettingPage>,
        _: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        Sidebar::new("settings-sidebar")
            .w(relative(1.))
            .border_0()
            .refine_style(&self.sidebar_style)
            .collapsed(false)
            .header(self.render_sidebar_header(state, cx))
            .child(self.render_sidebar_menu(state, pages, cx))
    }
}

impl Sizable for Settings {
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.size = size.into();
        self
    }
}

pub(super) struct SettingsState {
    pub(super) selected_index: SelectIndex,
    /// If set, defer scrolling to this group index after rendering.
    pub(super) deferred_scroll_group_ix: Option<usize>,
    pub(super) search_input: Entity<InputState>,
}

/// Options for rendering setting item.
#[derive(Clone, Copy)]
pub struct RenderOptions {
    pub page_ix: usize,
    pub group_ix: usize,
    pub item_ix: usize,
    pub size: Size,
    pub group_variant: GroupBoxVariant,
    pub layout: Axis,
}

#[derive(Clone, Copy, Default)]
pub struct SelectIndex {
    pub page_ix: usize,
    pub group_ix: Option<usize>,
}

impl RenderOnce for Settings {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |window, cx| {
            let search_input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(t!("Settings.search_placeholder"))
                    .default_value("")
            });

            SettingsState {
                search_input,
                selected_index: self.default_selected_index,
                deferred_scroll_group_ix: None,
            }
        });

        let query = state.read(cx).search_input.read(cx).value();
        let filtered_pages = self.filtered_pages(&query, cx);
        let options = RenderOptions {
            page_ix: 0,
            group_ix: 0,
            item_ix: 0,
            size: self.size,
            group_variant: self.group_variant,
            layout: Axis::Horizontal,
        };

        if self.use_sidebar_shell {
            let sidebar_width = self.sidebar_width;
            let sidebar_inset = self.sidebar_shell_inset;
            let clamped_sidebar_width = sidebar_width.max(px(200.0)).min(px(400.0));
            let mut floating_sidebar = FloatingSidebar::new("settings-floating-sidebar")
                .side(Side::Left)
                .width(sidebar_width)
                .resizer_width(px(0.0))
                .refine_style(&self.sidebar_style)
                .header(self.render_sidebar_header(&state, cx))
                .child(self.render_sidebar_menu(&state, &filtered_pages, cx));

            if sidebar_inset != px(8.0) {
                floating_sidebar = floating_sidebar.inset(sidebar_inset);
            }

            return div()
                .relative()
                .size_full()
                .overflow_hidden()
                .child(floating_sidebar)
                .child(
                    div()
                        .size_full()
                        .pl(clamped_sidebar_width + sidebar_inset)
                        .pt(self.safe_area_top)
                        .child(self.render_active_page(
                            &state,
                            &filtered_pages,
                            &options,
                            window,
                            cx,
                        )),
                )
                .into_any_element();
        }

        h_resizable(self.id.clone())
            .child(
                resizable_panel()
                    .size(self.sidebar_width)
                    .child(self.render_sidebar(&state, &filtered_pages, window, cx)),
            )
            .child(resizable_panel().child(self.render_active_page(
                &state,
                &filtered_pages,
                &options,
                window,
                cx,
            )))
            .into_any_element()
    }
}
