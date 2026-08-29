//! The primary Gallery surface: search, sidebar navigation, and the active
//! story panel.

use gpui::{prelude::*, *};
use neutron_components::{
    ActiveTheme as _, FloatingSidebar, Icon, IconName, WindowExt as _, h_flex,
    input::{Input, InputEvent, InputState},
    sidebar::{SidebarGroup, SidebarHeader, SidebarMenu, SidebarMenuItem},
    v_flex,
};
use neutron_story::{StoryContainer, story_descriptors};

use crate::commands::ToggleSearch;
use crate::launch::StoryLaunch;

/// The Gallery's own GPUI key context. Scopes the "/" Toggle Search binding
/// (see [`crate::commands::toggle_search_command`]) so it only fires while
/// the Gallery (or a view inside it) has focus.
pub(crate) const GALLERY_KEY_CONTEXT: &str = "Gallery";

/// The ephemeral default sidebar width. UI layout state, not a settings-store
/// preference: no prior storage contract persisted it, so it stays
/// per-session only.
const DEFAULT_SIDEBAR_WIDTH: Pixels = px(255.0);

pub(crate) struct Gallery {
    focus_handle: FocusHandle,
    stories: Vec<(&'static str, Vec<Entity<StoryContainer>>)>,
    active_group_index: Option<usize>,
    active_index: Option<usize>,
    collapsed: bool,
    sidebar_width: Pixels,
    search_input: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl Gallery {
    pub(crate) fn new(
        init_story_klass: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("Search..."));
        let _subscriptions = vec![cx.subscribe(&search_input, |this, _, e, cx| match e {
            InputEvent::Change => {
                this.active_group_index = Some(0);
                this.active_index = Some(0);
                cx.notify()
            }
            _ => {}
        })];
        let mut stories = Vec::new();
        for descriptor in story_descriptors() {
            if stories
                .last()
                .is_none_or(|(group, _)| *group != descriptor.group)
            {
                stories.push((descriptor.group, Vec::new()));
            }
            stories
                .last_mut()
                .expect("story registry always has a group")
                .1
                .push(descriptor.panel(window, cx));
        }

        let (active_group_index, active_index) = init_story_klass
            .and_then(story_position)
            .map(|(group_ix, item_ix)| (Some(group_ix), Some(item_ix)))
            .unwrap_or((Some(0), Some(0)));

        Self {
            focus_handle: cx.focus_handle(),
            search_input,
            stories,
            active_group_index,
            active_index,
            collapsed: false,
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
            _subscriptions,
        }
    }

    fn on_toggle_search(&mut self, _: &ToggleSearch, window: &mut Window, cx: &mut Context<Self>) {
        let search_focus_handle = self.search_input.read(cx).focus_handle(cx);
        if window.has_focused_input(cx) && !search_focus_handle.is_focused(window) {
            cx.propagate();
            return;
        }
        if search_focus_handle.is_focused(window) {
            // Already focused: toggle back to the Gallery itself.
            self.focus_handle.focus(window, cx);
        } else {
            self.search_input.update(cx, |input, cx| {
                input.focus(window, cx);
            });
        }
    }

    pub(crate) fn view(
        init_story_klass: Option<&str>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|cx| Self::new(init_story_klass, window, cx))
    }
}

impl Focusable for Gallery {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// The `(group_ix, item_ix)` position of `klass` in the same grouped order
/// [`Gallery::new`] uses to build `stories`, or `None` if the klass has no
/// descriptor. Defensive: the CLI parser only ever resolves and carries an
/// exact, registry-validated klass, so a miss here never happens in practice.
fn story_position(klass: &str) -> Option<(usize, usize)> {
    let mut group_ix = 0usize;
    let mut item_ix = 0usize;
    let mut current_group: Option<&'static str> = None;
    for descriptor in story_descriptors() {
        if current_group == Some(descriptor.group) {
            item_ix += 1;
        } else {
            if current_group.is_some() {
                group_ix += 1;
            }
            item_ix = 0;
            current_group = Some(descriptor.group);
        }
        if descriptor.story_klass == klass {
            return Some((group_ix, item_ix));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::story_position;

    #[test]
    fn resolves_the_first_story_in_the_first_group() {
        assert_eq!(story_position("WelcomeStory"), Some((0, 0)));
    }

    #[test]
    fn resolves_positions_within_the_second_group() {
        assert_eq!(story_position("AccordionStory"), Some((1, 0)));
        assert_eq!(story_position("AlertDialogStory"), Some((1, 1)));
    }

    #[test]
    fn an_unknown_klass_has_no_position() {
        assert_eq!(story_position("NotARealStory"), None);
    }
}

/// The primary surface's build function: the typed launch value's resolved
/// story descriptor selects the initial story directly (never through the
/// fuzzy search field).
pub(crate) fn build_gallery(
    launch: &StoryLaunch,
    window: &mut Window,
    cx: &mut App,
) -> Entity<Gallery> {
    Gallery::view(launch.story_klass, window, cx)
}

impl Render for Gallery {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search_input.read(cx).value().trim().to_lowercase();

        let stories: Vec<_> = self
            .stories
            .iter()
            .filter_map(|(name, items)| {
                let filtered_items: Vec<_> = items
                    .iter()
                    .filter(|story| story.read(cx).name.to_lowercase().contains(&query))
                    .cloned()
                    .collect();

                if !filtered_items.is_empty() {
                    Some((name, filtered_items))
                } else {
                    None
                }
            })
            .collect();

        let active_group = self.active_group_index.and_then(|index| stories.get(index));
        let active_story = self
            .active_index
            .and(active_group)
            .and_then(|group| group.1.get(self.active_index.unwrap()));
        let (story_name, description) =
            if let Some(story) = active_story.as_ref().map(|story| story.read(cx)) {
                (story.name.clone(), story.description.clone())
            } else {
                ("".into(), "".into())
            };

        let inset = px(8.0);
        let collapsed_width = px(48.0);
        let sidebar_width = if self.collapsed {
            collapsed_width
        } else {
            self.sidebar_width
        };
        let content_offset = sidebar_width + inset;
        let view = cx.entity().downgrade();

        div()
            .id("gallery")
            .key_context(GALLERY_KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_toggle_search))
            .relative()
            .size_full()
            .overflow_hidden()
            .child(
                FloatingSidebar::new("gallery-sidebar")
                    .width(self.sidebar_width)
                    .collapsed(self.collapsed)
                    .inset(inset)
                    .on_resize_end(move |width, _, cx| {
                        if let Some(view) = view.upgrade() {
                            view.update(cx, |this, cx| {
                                this.sidebar_width = width;
                                cx.notify();
                            });
                        }
                    })
                    .header_with({
                        let search_input = self.search_input.clone();
                        move |collapsed, _, cx| {
                            v_flex()
                                .w_full()
                                .gap_4()
                                .child(
                                    SidebarHeader::new()
                                        .w_full()
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .rounded(cx.theme().radius)
                                                .bg(cx.theme().primary)
                                                .text_color(cx.theme().primary_foreground)
                                                .size_8()
                                                .flex_shrink_0()
                                                .when(!collapsed, |this| {
                                                    this.child(Icon::new(
                                                        IconName::GalleryVerticalEnd,
                                                    ))
                                                })
                                                .when(collapsed, |this| {
                                                    this.size_4()
                                                        .bg(cx.theme().transparent)
                                                        .text_color(cx.theme().foreground)
                                                        .child(Icon::new(
                                                            IconName::GalleryVerticalEnd,
                                                        ))
                                                })
                                                .rounded_lg(),
                                        )
                                        .when(!collapsed, |this| {
                                            this.child(
                                                v_flex()
                                                    .gap_0()
                                                    .text_sm()
                                                    .flex_1()
                                                    .line_height(relative(1.25))
                                                    .overflow_hidden()
                                                    .text_ellipsis()
                                                    .child("Neutron Story")
                                                    .child(
                                                        div()
                                                            .text_color(cx.theme().muted_foreground)
                                                            .child("Components")
                                                            .text_xs(),
                                                    ),
                                            )
                                        }),
                                )
                                .child(
                                    div()
                                        .bg(cx.theme().sidebar_accent)
                                        .rounded_full()
                                        .px_1()
                                        .when(cx.theme().radius.is_zero(), |this| {
                                            this.rounded(px(0.))
                                        })
                                        .flex_1()
                                        .mx_1()
                                        .child(
                                            Input::new(&search_input)
                                                .appearance(false)
                                                .cleanable(true),
                                        ),
                                )
                        }
                    })
                    .children(stories.clone().into_iter().enumerate().map(
                        |(group_ix, (group_name, sub_stories))| {
                            SidebarGroup::new(*group_name).child(SidebarMenu::new().children(
                                sub_stories.iter().enumerate().map(|(ix, story)| {
                                    SidebarMenuItem::new(story.read(cx).name.clone())
                                        .active(
                                            self.active_group_index == Some(group_ix)
                                                && self.active_index == Some(ix),
                                        )
                                        .on_click(cx.listener(
                                            move |this, _: &ClickEvent, _, cx| {
                                                this.active_group_index = Some(group_ix);
                                                this.active_index = Some(ix);
                                                cx.notify();
                                            },
                                        ))
                                }),
                            ))
                        },
                    )),
            )
            .child(
                v_flex()
                    .flex_1()
                    .h_full()
                    .overflow_x_hidden()
                    .pl(content_offset)
                    .child(
                        h_flex()
                            .id("header")
                            .p_4()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .justify_between()
                            .items_start()
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(div().text_xl().child(story_name))
                                    .child(
                                        div()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(description),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .id("story")
                            .flex_1()
                            .overflow_y_scroll()
                            .when_some(active_story, |this, active_story| {
                                this.child(active_story.clone())
                            }),
                    )
                    .into_any_element(),
            )
    }
}
