use crate::{
    ActiveTheme, Collapsible,
    animation::{
        PresenceOptions, PresencePhase, expand_collapse_durations,
        expand_collapse_layout_animation, keyed_presence,
    },
    global_state::GlobalState,
    h_flex,
    sidebar::SidebarItem,
    v_flex,
};
use gpui::{
    AnimationExt as _, App, ElementId, IntoElement, ParentElement, SharedString, Styled as _,
    Window, div, prelude::FluentBuilder as _, px,
};

const SIDEBAR_GROUP_LABEL_HEIGHT: gpui::Pixels = px(32.0);

/// A group of items in the [`super::Sidebar`].
#[derive(Clone)]
pub struct SidebarGroup<E: SidebarItem + 'static> {
    label: SharedString,
    collapsed: bool,
    children: Vec<E>,
}

impl<E: SidebarItem> SidebarGroup<E> {
    /// Create a new [`SidebarGroup`] with the given label.
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            collapsed: false,
            children: Vec::new(),
        }
    }

    /// Add a child to the sidebar group, the child should implement [`SidebarItem`].
    pub fn child(mut self, child: E) -> Self {
        self.children.push(child);
        self
    }

    /// Add multiple children to the sidebar group.
    ///
    /// See also [`SidebarGroup::child`].
    pub fn children(mut self, children: impl IntoIterator<Item = E>) -> Self {
        self.children.extend(children);
        self
    }
}

impl<E: SidebarItem> Collapsible for SidebarGroup<E> {
    fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }
}

impl<E: SidebarItem> SidebarItem for SidebarGroup<E> {
    fn render(
        self,
        id: impl Into<ElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        let id = id.into();
        let reduced_motion = GlobalState::global(cx).reduced_motion();
        let motion = cx.theme().motion.clone();
        let (open_duration, close_duration) = expand_collapse_durations(&motion);
        let label_presence = keyed_presence(
            SharedString::from(format!("{}-group-label-presence", id)),
            !self.collapsed,
            !reduced_motion,
            open_duration,
            close_duration,
            PresenceOptions::default(),
            window,
            cx,
        );

        v_flex()
            .relative()
            .when(label_presence.should_render(), |this| {
                this.child(
                    div()
                        .overflow_hidden()
                        .child(
                            h_flex()
                                .flex_shrink_0()
                                .px_2()
                                .rounded(cx.theme().radius)
                                .text_xs()
                                .text_color(cx.theme().sidebar_foreground.opacity(0.7))
                                .h_8()
                                .child(self.label),
                        )
                        .map(|el| {
                            if !label_presence.transition_active() {
                                return el.into_any_element();
                            }

                            let Some(anim) = expand_collapse_layout_animation(
                                &motion,
                                reduced_motion,
                                matches!(label_presence.phase, PresencePhase::Entering),
                            ) else {
                                return el.into_any_element();
                            };

                            el.with_animation(
                                SharedString::from(format!(
                                    "{}-group-label-{}",
                                    id,
                                    u8::from(matches!(
                                        label_presence.phase,
                                        PresencePhase::Entering
                                    ))
                                )),
                                anim,
                                move |el, delta| {
                                    let progress = label_presence.progress(delta);
                                    el.max_h(SIDEBAR_GROUP_LABEL_HEIGHT * progress)
                                        .opacity(progress)
                                },
                            )
                            .into_any_element()
                        }),
                )
            })
            .child(
                div()
                    .gap_2()
                    .flex_col()
                    .children(self.children.into_iter().enumerate().map(|(ix, child)| {
                        child
                            .collapsed(self.collapsed)
                            .render(format!("{}-{}", id, ix), window, cx)
                            .into_any_element()
                    })),
            )
    }
}
