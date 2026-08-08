//! View component for the Command Palette.

use super::provider::CommandPaletteProvider;
use super::reveal_delay;
use super::state::{CommandPaletteEvent, CommandPaletteState};
use super::types::{CommandPaletteConfig, MatchedItem};
use crate::actions::{Cancel, Confirm, SelectDown, SelectUp};
use crate::animation::{exit_animation, fade_animation, spring_animation, standard_animation};
use crate::global_state::GlobalState;
use crate::input::{Input, InputEvent, InputState};
use crate::kbd::Kbd;
use crate::spinner::Spinner;
use crate::{
    ActiveTheme, FlyoutTokens, Icon, IconName, Sizable, Size, SurfaceContext, SurfacePreset,
    VirtualListScrollHandle, WindowExt as _, flyout_accent_foreground, flyout_secondary_foreground,
    h_flex, is_layer_closing, v_flex, v_virtual_list,
};
use gpui::{
    AnimationExt, App, AppContext as _, Context, ElementId, Entity, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyBinding, ParentElement, Pixels, Render, ScrollStrategy,
    SharedString, Size as GpuiSize, Styled, Subscription, Task, Window, div,
    prelude::FluentBuilder, px,
};
use std::rc::Rc;
use std::sync::Arc;

const CONTEXT: &str = "CommandPalette";

// Height constants for layout calculations. Row geometry (section header height,
// list padding, row radius) comes from `FlyoutTokens` so the palette stays in the
// same geometry family as menus and select popups.
const HEADER_HEIGHT: f32 = 56.0;
const FOOTER_HEIGHT: f32 = 40.0;
const EMPTY_STATE_HEIGHT: f32 = 120.0;

/// A render row for the command palette list.
#[derive(Clone)]
enum CommandPaletteRow {
    Header(SharedString),
    Item(usize),
}

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", Cancel, Some(CONTEXT)),
        KeyBinding::new("enter", Confirm { secondary: false }, Some(CONTEXT)),
        KeyBinding::new("up", SelectUp, Some(CONTEXT)),
        KeyBinding::new("down", SelectDown, Some(CONTEXT)),
    ]);
}

/// The Command Palette view component.
pub struct CommandPaletteView {
    /// The internal state entity.
    pub(crate) state: Entity<CommandPaletteState>,
    /// The search input state.
    input_state: Entity<InputState>,
    /// Focus handle for the palette.
    focus_handle: FocusHandle,
    /// Scroll handle for the list.
    scroll_handle: VirtualListScrollHandle,
    /// Item height for virtualization.
    item_height: Pixels,
    /// Tracks whether we've focused the input once after open.
    did_focus: bool,
    /// Whether the results list has been revealed.
    list_revealed: bool,
    /// Task for delayed reveal.
    _reveal_task: Option<Task<()>>,
    /// Subscriptions.
    _subscriptions: Vec<Subscription>,
}

impl CommandPaletteView {
    /// Create a new command palette view.
    pub fn new(
        config: CommandPaletteConfig,
        provider: Arc<dyn CommandPaletteProvider>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let placeholder = config.placeholder.clone();

        let state = cx.new(|cx| CommandPaletteState::new(config, provider, window, cx));

        let input_state = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));

        let focus_handle = cx.focus_handle();

        // Subscribe to input changes
        let input_subscription = cx.subscribe_in(&input_state, window, Self::on_input_event);

        // Subscribe to state events
        let state_subscription = cx.subscribe_in(&state, window, Self::on_state_event);

        Self {
            state,
            input_state,
            focus_handle,
            scroll_handle: VirtualListScrollHandle::new(),
            item_height: px(46.),
            did_focus: false,
            list_revealed: false,
            _reveal_task: None,
            _subscriptions: vec![input_subscription, state_subscription],
        }
    }

    fn schedule_reveal(&mut self, cx: &mut Context<Self>) {
        if self.list_revealed || self._reveal_task.is_some() {
            return;
        }

        let delay = reveal_delay(cx);
        self._reveal_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;

            if let Some(this) = this.upgrade() {
                this.update(cx, |this, cx| {
                    if !this.list_revealed {
                        this.list_revealed = true;
                        this._reveal_task = None;
                        cx.notify();
                    }
                });
            }
        }));
    }

    fn on_input_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                let query = self.input_state.read(cx).value().to_string();
                self.state.update(cx, |state, cx| {
                    state.set_query(query, window, cx);
                });
            }
            InputEvent::PressEnter { .. } => {
                self.state.update(cx, |state, cx| {
                    state.confirm(cx);
                });
            }
            _ => {}
        }
    }

    fn on_state_event(
        &mut self,
        _: &Entity<CommandPaletteState>,
        event: &CommandPaletteEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Close the dialog on selection or dismissal
        match event {
            CommandPaletteEvent::Selected { .. } | CommandPaletteEvent::Dismissed => {
                window.close_dialog(cx);
            }
        }
        // Forward events
        cx.emit(event.clone());
    }

    fn on_action_cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.state.update(cx, |state, cx| {
            state.dismiss(cx);
        });
    }

    fn on_action_confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.state.update(cx, |state, cx| {
            state.confirm(cx);
        });
    }

    fn on_action_select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.state.update(cx, |state, cx| {
            state.select_prev(cx);
        });
        self.scroll_to_selected(cx);
    }

    fn on_action_select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.state.update(cx, |state, cx| {
            state.select_next(cx);
        });
        self.scroll_to_selected(cx);
    }

    fn scroll_to_selected(&mut self, cx: &App) {
        let state = self.state.read(cx);
        if let Some(index) = state.selected_index {
            let row_index = self.row_index_for_item(&state, index);
            self.scroll_handle
                .scroll_to_item(row_index, ScrollStrategy::Top);
        }
    }

    fn render_item(
        &self,
        item: &MatchedItem,
        item_index: usize,
        selected: bool,
        show_category: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let tokens = FlyoutTokens::new(cx);
        let item_data = item.item.clone();
        let match_info = item.match_info.clone();
        let disabled = item_data.disabled;
        let show_inline_category = show_category && !item_data.category.is_empty();
        let secondary_foreground = flyout_secondary_foreground(cx);

        let shortcut_element = item_data
            .shortcut
            .as_ref()
            .and_then(|s| gpui::Keystroke::parse(s).ok())
            .map(|keystroke| {
                Kbd::new(keystroke)
                    .outline()
                    .when(!selected && !disabled, |this| {
                        this.text_color(secondary_foreground)
                    })
                    .when(selected && !disabled, |this| {
                        this.bg(cx.theme().list_active)
                            .border_color(cx.theme().list_active_border)
                            .text_color(cx.theme().foreground)
                    })
            });
        let has_shortcut = shortcut_element.is_some();

        h_flex()
            .id(SharedString::from(format!("cmd-item-{}", item_index)))
            .w_full()
            .h(self.item_height)
            .px(tokens.item_padding_x)
            .gap(tokens.accessory_gap)
            .items_center()
            .rounded(tokens.item_radius)
            .cursor_pointer()
            .when(disabled, |this| this.opacity(0.5).cursor_not_allowed())
            // Reserve the selected border on every row: the row height is fixed,
            // so adding a border only when selected would shrink the content box
            // and shift the label as selection moves.
            .border_1()
            .border_color(cx.theme().transparent)
            .when(selected && !disabled, |this| {
                this.bg(cx.theme().list_active)
                    .border_color(cx.theme().list_active_border)
                    .text_color(cx.theme().foreground)
            })
            .when(!selected && !disabled, |this| {
                this.hover(|this| this.bg(cx.theme().list_hover))
            })
            .when(!disabled, |this| {
                let index = item_index;
                this.on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(move |view, _, _, cx| {
                        view.state.update(cx, |state, cx| {
                            state.select_index(index, cx);
                            state.confirm(cx);
                        });
                    }),
                )
            })
            // Icon + title/subtitle share the tight icon gutter, matching a menu row.
            .child(
                h_flex()
                    .flex_1()
                    .items_center()
                    .overflow_hidden()
                    .gap(tokens.icon_gap)
                    .when_some(item_data.icon, |this, icon| {
                        this.child(Icon::new(icon).size_4().flex_shrink_0().text_color(
                            if selected && !disabled {
                                cx.theme().foreground
                            } else {
                                secondary_foreground
                            },
                        ))
                    })
                    .child(
                        v_flex()
                            .flex_1()
                            .overflow_hidden()
                            .child(
                                div()
                                    .text_size(tokens.label_size)
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(cx.theme().foreground)
                                    .truncate()
                                    .child(self.render_highlighted_text(
                                        &item_data.title,
                                        &match_info.title_ranges,
                                        cx,
                                    )),
                            )
                            .when_some(item_data.subtitle.clone(), |this, subtitle| {
                                this.child(
                                    div()
                                        .text_size(tokens.meta_size)
                                        .text_color(secondary_foreground)
                                        .truncate()
                                        .child(subtitle),
                                )
                            }),
                    ),
            )
            .when(show_inline_category || has_shortcut, |this| {
                this.child(
                    h_flex()
                        .flex_shrink_0()
                        .items_center()
                        .gap(tokens.icon_gap)
                        .when(show_inline_category, |this| {
                            this.child(
                                div()
                                    .text_size(tokens.meta_size)
                                    .text_color(if selected && !disabled {
                                        cx.theme().foreground
                                    } else {
                                        secondary_foreground
                                    })
                                    .truncate()
                                    .child(item_data.category.clone()),
                            )
                        })
                        .when_some(shortcut_element, |this, kbd| this.child(kbd)),
                )
            })
    }

    fn render_section_header(&self, title: SharedString, cx: &App) -> impl IntoElement {
        let tokens = FlyoutTokens::new(cx);

        div()
            .w_full()
            .h(tokens.section_header_height)
            .flex()
            .items_center()
            // Same left edge as a row's label, so headers and rows share one
            // alignment line instead of stepping in and out by 4px.
            .px(tokens.item_padding_x)
            .text_size(tokens.meta_size)
            .font_weight(tokens.section_weight)
            .text_color(flyout_secondary_foreground(cx))
            .child(title)
    }

    fn render_highlighted_text(
        &self,
        text: &str,
        ranges: &[(usize, usize)],
        cx: &App,
    ) -> impl IntoElement {
        if ranges.is_empty() {
            return div().truncate().child(text.to_string()).into_any_element();
        }

        let mut elements = Vec::new();
        let mut last_end = 0;

        for &(start, end) in ranges {
            // Text before the highlight
            if start > last_end {
                elements.push(
                    div()
                        .child(text[last_end..start].to_string())
                        .into_any_element(),
                );
            }
            // Highlighted text
            let end = end.min(text.len());
            if start < end {
                elements.push(
                    div()
                        .text_color(flyout_accent_foreground(cx))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(text[start..end].to_string())
                        .into_any_element(),
                );
            }
            last_end = end;
        }

        // Remaining text
        if last_end < text.len() {
            elements.push(div().child(text[last_end..].to_string()).into_any_element());
        }

        h_flex().truncate().children(elements).into_any_element()
    }

    fn render_footer(&self, status_text: Option<SharedString>, cx: &App) -> impl IntoElement {
        let reduced_motion = GlobalState::global(cx).reduced_motion();
        let has_status = status_text.is_some();
        let tokens = FlyoutTokens::new(cx);

        h_flex()
            .w_full()
            .h(px(FOOTER_HEIGHT))
            // Align with row labels: list inset + row padding.
            .px(tokens.inset + tokens.item_padding_x)
            .border_t_1()
            .border_color(cx.theme().border)
            .justify_between()
            .text_size(tokens.meta_size)
            .text_color(flyout_secondary_foreground(cx))
            .child(
                h_flex()
                    .gap_4()
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Kbd::new(gpui::Keystroke::parse("up").unwrap()).appearance(false),
                            )
                            .child(
                                Kbd::new(gpui::Keystroke::parse("down").unwrap()).appearance(false),
                            )
                            .child("Navigate"),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Kbd::new(gpui::Keystroke::parse("enter").unwrap())
                                    .appearance(false),
                            )
                            .child("Run"),
                    ),
            )
            .when_some(status_text, |this, status| {
                this.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .text_color(flyout_secondary_foreground(cx))
                        .when(reduced_motion, |this| {
                            this.child(Icon::new(IconName::LoaderCircle).size_4())
                        })
                        .when(!reduced_motion, |this| {
                            this.child(
                                Spinner::new()
                                    .icon(IconName::LoaderCircle)
                                    .with_size(Size::Small)
                                    .color(flyout_secondary_foreground(cx)),
                            )
                        })
                        .child(status),
                )
            })
            .when(!has_status, |this| {
                this.child(
                    h_flex()
                        .gap_1()
                        .child(
                            Kbd::new(gpui::Keystroke::parse("escape").unwrap()).appearance(false),
                        )
                        .child("Close"),
                )
            })
    }

    fn render_empty(&self, query_empty: bool, cx: &App) -> impl IntoElement {
        let tokens = FlyoutTokens::new(cx);

        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_1()
            .text_color(flyout_secondary_foreground(cx))
            .child(Icon::new(IconName::Search).size_6().opacity(0.62))
            .child(
                div()
                    .mt_1()
                    .text_size(tokens.label_size)
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(cx.theme().foreground)
                    .child(if query_empty {
                        "No commands available"
                    } else {
                        "No commands found"
                    }),
            )
            .when(!query_empty, |this| {
                this.child(
                    div()
                        .text_size(tokens.meta_size)
                        .child("Try a different search."),
                )
            })
    }

    fn build_rows(
        &self,
        state: &CommandPaletteState,
        matched_items: &[MatchedItem],
    ) -> Vec<CommandPaletteRow> {
        let static_len = state.matched_static_len.min(matched_items.len());
        let async_len = matched_items.len().saturating_sub(static_len);
        let query_empty = state.query.is_empty();

        let mut rows = Vec::new();
        if query_empty {
            rows.extend((0..static_len).map(CommandPaletteRow::Item));
            return rows;
        }

        if static_len > 0 {
            if let Some(title) = state.config.commands_section_title.clone() {
                rows.push(CommandPaletteRow::Header(title));
            }
            rows.extend((0..static_len).map(CommandPaletteRow::Item));
        }

        if async_len > 0 {
            if let Some(title) = state.config.results_section_title.clone() {
                rows.push(CommandPaletteRow::Header(title));
            }
            rows.extend((static_len..matched_items.len()).map(CommandPaletteRow::Item));
        }

        rows
    }

    fn row_index_for_item(&self, state: &CommandPaletteState, item_index: usize) -> usize {
        let static_len = state.matched_static_len.min(state.matched_items.len());
        let async_len = state.matched_items.len().saturating_sub(static_len);
        let query_empty = state.query.is_empty();

        if query_empty {
            return item_index;
        }

        let commands_header = static_len > 0 && state.config.commands_section_title.is_some();
        let results_header = async_len > 0 && state.config.results_section_title.is_some();
        let mut row_index = item_index + usize::from(commands_header);
        if item_index >= static_len && results_header {
            row_index += 1;
        }
        row_index
    }
}

impl gpui::EventEmitter<CommandPaletteEvent> for CommandPaletteView {}

impl Focusable for CommandPaletteView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for CommandPaletteView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = FlyoutTokens::new(cx);
        let state = self.state.read(cx);
        let config = state.config.clone();
        let matched_items = state.matched_items.clone();
        let selected_index = state.selected_index;
        let rows = Rc::new(self.build_rows(&state, &matched_items));
        let row_count = rows.len();

        // Prepare item sizes for virtual list
        let item_sizes: Rc<Vec<GpuiSize<Pixels>>> = Rc::new(
            rows.iter()
                .map(|row| GpuiSize {
                    width: px(0.),
                    height: match row {
                        CommandPaletteRow::Header(_) => tokens.section_header_height,
                        CommandPaletteRow::Item(_) => self.item_height,
                    },
                })
                .collect(),
        );

        let show_categories = config.show_categories_inline;
        let show_footer = config.show_footer;
        let query_empty = state.query.is_empty();
        let footer_status = config
            .status_provider
            .as_ref()
            .and_then(|provider| provider(&state.query));
        let max_height = px(config.max_height);
        let reduced_motion = GlobalState::global(cx).reduced_motion();
        let motion = cx.theme().motion.clone();
        let reveal_opacity_animation = fade_animation(&motion, reduced_motion);
        let reveal_transform_animation = spring_animation(&motion, reduced_motion);

        // Focus input once after opening to avoid render jitter
        if !self.did_focus {
            self.input_state.update(cx, |input, cx| {
                input.focus(window, cx);
            });
            self.did_focus = true;

            if !self.list_revealed {
                if reduced_motion {
                    self.list_revealed = true;
                } else {
                    self.schedule_reveal(cx);
                }
            }
        }

        // Compute height for surface bounds
        let list_content_height = if row_count == 0 {
            px(EMPTY_STATE_HEIGHT)
        } else {
            // Must match the padding actually applied to the list below, or the
            // surface ends up taller than its content.
            tokens.inset * 2.
                + rows.iter().fold(px(0.0), |sum, row| {
                    sum + match row {
                        CommandPaletteRow::Header(_) => tokens.section_header_height,
                        CommandPaletteRow::Item(_) => self.item_height,
                    }
                })
        };
        let list_height = max_height.min(list_content_height);
        let expanded_height = px(HEADER_HEIGHT)
            + list_height
            + if show_footer {
                px(FOOTER_HEIGHT)
            } else {
                px(0.0)
            };

        let closing = is_layer_closing(cx);
        let surface_ctx = SurfaceContext::new(cx);

        let content = v_flex()
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_action_cancel))
            .on_action(cx.listener(Self::on_action_confirm))
            .on_action(cx.listener(Self::on_action_select_up))
            .on_action(cx.listener(Self::on_action_select_down))
            .h(expanded_height)
            .w_full()
            .overflow_hidden()
            // Search input
            .child(
                div()
                    .w_full()
                    .h(px(HEADER_HEIGHT))
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        Input::new(&self.input_state)
                            .with_size(Size::Medium)
                            .prefix(
                                Icon::new(IconName::Search)
                                    .size_4()
                                    .text_color(flyout_secondary_foreground(cx)),
                            )
                            .appearance(false)
                            .cleanable(true)
                            // The input carries its own size-derived padding. Override it
                            // onto the row rail so the search glyph sits on the same
                            // vertical line as the result-row icons directly below it.
                            .px(tokens.inset + tokens.item_padding_x),
                    ),
            )
            // Results list
            .when(self.list_revealed, |this| {
                this.child({
                    let results = v_flex()
                        .w_full()
                        .child(
                            div()
                                .w_full()
                                .h(list_height)
                                .overflow_hidden()
                                .when(row_count == 0, |this| {
                                    this.child(self.render_empty(query_empty, cx))
                                })
                                .when(row_count > 0, |this| {
                                    this.child(
                                        v_virtual_list(
                                            cx.entity(),
                                            "command-palette-list",
                                            item_sizes,
                                            {
                                                let matched_items = matched_items.clone();
                                                let rows = rows.clone();
                                                move |view, visible_range, window, cx| {
                                                    visible_range
                                                        .filter_map(|ix| {
                                                            let row = rows.get(ix)?;
                                                            match row {
                                                                CommandPaletteRow::Header(
                                                                    title,
                                                                ) => Some(
                                                                    view.render_section_header(
                                                                        title.clone(),
                                                                        cx,
                                                                    )
                                                                    .into_any_element(),
                                                                ),
                                                                CommandPaletteRow::Item(
                                                                    item_index,
                                                                ) => matched_items
                                                                    .get(*item_index)
                                                                    .map(|item| {
                                                                        view.render_item(
                                                                            item,
                                                                            *item_index,
                                                                            selected_index
                                                                                == Some(
                                                                                    *item_index,
                                                                                ),
                                                                            show_categories,
                                                                            window,
                                                                            cx,
                                                                        )
                                                                        .into_any_element()
                                                                    }),
                                                            }
                                                        })
                                                        .collect()
                                                }
                                            },
                                        )
                                        .track_scroll(&self.scroll_handle)
                                        .p(tokens.inset),
                                    )
                                }),
                        )
                        .when(show_footer, |this| {
                            this.child(self.render_footer(footer_status.clone(), cx))
                        });

                    let transformed = if let Some(anim) = reveal_transform_animation {
                        div()
                            .child(results)
                            .with_animation(
                                ElementId::NamedInteger(
                                    "command-palette-results-transform".into(),
                                    1,
                                ),
                                anim,
                                move |el, delta| el.translate_y(px(4.0 * (1.0 - delta.max(0.0)))),
                            )
                            .into_any_element()
                    } else {
                        div().child(results).into_any_element()
                    };

                    if let Some(anim) = reveal_opacity_animation {
                        div()
                            .child(transformed)
                            .with_animation(
                                ElementId::NamedInteger(
                                    "command-palette-results-opacity".into(),
                                    1,
                                ),
                                anim,
                                move |el, delta| el.opacity(delta.clamp(0.0, 1.0)),
                            )
                            .into_any_element()
                    } else {
                        transformed
                    }
                })
            });

        let surface = SurfacePreset::flyout()
            .with_radius(tokens.radius)
            .wrap_with_bounds(
                content,
                px(config.width),
                expanded_height,
                window,
                cx,
                surface_ctx,
            )
            .h(expanded_height)
            .w(px(config.width));

        // Enter motion for the surface itself (it previously popped in while
        // only the results animated). Mount-only: the view is created on open,
        // so the animation plays exactly once. On close the host dialog defers
        // unmount (`defer_close`) and flags the subtree via `is_layer_closing`,
        // which is when the exit below runs.
        if closing {
            return if let Some(anim) = exit_animation(&motion, reduced_motion) {
                div()
                    .child(surface)
                    .with_animation("command-palette-exit", anim, |el, delta| {
                        let progress = (1.0 - delta).clamp(0.0, 1.0);
                        el.opacity(progress).translate_y(px(4.0 * delta))
                    })
                    .into_any_element()
            } else {
                surface.into_any_element()
            };
        }

        let open_fade_anim = standard_animation(&motion, reduced_motion);
        let open_transform_anim = spring_animation(&motion, reduced_motion);
        let transformed = if let Some(anim) = open_transform_anim {
            div()
                .child(surface)
                .with_animation("command-palette-open-transform", anim, |el, delta| {
                    el.translate_y(px(4.0 * (1.0 - delta)))
                })
                .into_any_element()
        } else {
            surface.into_any_element()
        };
        if let Some(anim) = open_fade_anim {
            div()
                .child(transformed)
                .with_animation("command-palette-open-fade", anim, |el, delta| {
                    el.opacity(delta.clamp(0.0, 1.0))
                })
                .into_any_element()
        } else {
            transformed
        }
    }
}
