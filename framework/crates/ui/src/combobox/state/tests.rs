use std::{cell::Cell, rc::Rc};

use gpui::{
    AppContext as _, Bounds, ClickEvent, Context, Entity, Focusable as _, InteractiveElement as _,
    IntoElement, Modifiers, MouseButton, MouseDownEvent, ParentElement as _, Pixels, Point, Render,
    RenderOnce as _, Role, SharedString, Subscription, TestAppContext, VisualTestContext, Window,
    div, point, px, size,
};

use crate::{
    IndexPath,
    actions::{Cancel, SelectDown},
    combobox::{Combobox, ComboboxEvent, ComboboxState},
    list::ListDelegate as _,
    searchable_list::{
        SearchableListChange, SearchableListDelegate, SearchableListItem, SearchableListState,
        SearchableVec,
    },
};

struct TestComboboxEventCollector {
    event_count: Rc<Cell<usize>>,
    _subscription: Subscription,
}

impl TestComboboxEventCollector {
    fn new(
        state: &Entity<ComboboxState<SearchableVec<&'static str>>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let event_count = Rc::new(Cell::new(0));
        let event_count_for_subscription = event_count.clone();
        let _subscription = cx.subscribe(
            state,
            move |_, _, _: &ComboboxEvent<SearchableVec<&'static str>>, _| {
                event_count_for_subscription.set(event_count_for_subscription.get() + 1);
            },
        );

        Self {
            event_count,
            _subscription,
        }
    }
}

struct TestComboboxObserver {
    notifications: Rc<Cell<usize>>,
    _subscription: Subscription,
}

impl TestComboboxObserver {
    fn new(
        state: &Entity<ComboboxState<SearchableVec<&'static str>>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let notifications = Rc::new(Cell::new(0));
        let notifications_for_observer = notifications.clone();
        let _subscription = cx.observe(state, move |_, _, _| {
            notifications_for_observer.set(notifications_for_observer.get() + 1);
        });

        Self {
            notifications,
            _subscription,
        }
    }
}

struct ConfirmTrackingDelegate {
    items: SearchableVec<&'static str>,
    confirms: Rc<Cell<usize>>,
}

impl ConfirmTrackingDelegate {
    fn new(confirms: Rc<Cell<usize>>) -> Self {
        Self {
            items: SearchableVec::new(vec!["Rust", "Go"]),
            confirms,
        }
    }
}

impl SearchableListDelegate for ConfirmTrackingDelegate {
    type Item = &'static str;

    fn items_count(&self, section: usize) -> usize {
        self.items.items_count(section)
    }

    fn item(&self, ix: IndexPath) -> Option<&&'static str> {
        self.items.item(ix)
    }

    fn position<V>(&self, value: &V) -> Option<IndexPath>
    where
        &'static str: SearchableListItem<Value = V>,
        V: PartialEq,
    {
        self.items.position(value)
    }

    fn on_confirm(&mut self, _: &[(IndexPath, Self::Item)]) {
        self.confirms.set(self.confirms.get() + 1);
    }
}

#[derive(Clone)]
struct VersionedItem {
    title: &'static str,
    value: &'static str,
}

impl SearchableListItem for VersionedItem {
    type Value = &'static str;

    fn title(&self) -> SharedString {
        self.title.into()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

struct SameIndexValueDelegate {
    item: VersionedItem,
    replacement: VersionedItem,
}

impl SearchableListDelegate for SameIndexValueDelegate {
    type Item = VersionedItem;

    fn items_count(&self, _: usize) -> usize {
        1
    }

    fn item(&self, ix: IndexPath) -> Option<&Self::Item> {
        (ix == IndexPath::new(0)).then_some(&self.item)
    }

    fn position<V>(&self, value: &V) -> Option<IndexPath>
    where
        Self::Item: SearchableListItem<Value = V>,
        V: PartialEq,
    {
        (self.item.value() == value).then_some(IndexPath::new(0))
    }

    fn on_will_change(
        &mut self,
        selection: &mut Vec<(IndexPath, Self::Item)>,
        _: &[SearchableListChange],
    ) {
        *selection = vec![(IndexPath::new(0), self.replacement.clone())];
    }
}

struct TrackedEventCollector<D: SearchableListDelegate + 'static> {
    change_count: Rc<Cell<usize>>,
    confirm_count: Rc<Cell<usize>>,
    _subscription: Subscription,
    _marker: std::marker::PhantomData<fn() -> D>,
}

impl<D> TrackedEventCollector<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    fn new(state: &Entity<ComboboxState<D>>, cx: &mut Context<Self>) -> Self {
        let change_count = Rc::new(Cell::new(0));
        let confirm_count = Rc::new(Cell::new(0));
        let change_count_for_subscription = change_count.clone();
        let confirm_count_for_subscription = confirm_count.clone();
        let _subscription = cx.subscribe(
            state,
            move |_, _, event: &ComboboxEvent<D>, _| match event {
                ComboboxEvent::Change(_) => {
                    change_count_for_subscription.set(change_count_for_subscription.get() + 1);
                }
                ComboboxEvent::Confirm(_) => {
                    confirm_count_for_subscription.set(confirm_count_for_subscription.get() + 1);
                }
            },
        );

        Self {
            change_count,
            confirm_count,
            _subscription,
            _marker: std::marker::PhantomData,
        }
    }
}

struct ComboboxEscapeHarness {
    state: Entity<ComboboxState<SearchableVec<&'static str>>>,
    parent_cancels: Rc<Cell<usize>>,
}

impl Render for ComboboxEscapeHarness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let parent_cancels = self.parent_cancels.clone();
        div()
            .on_action(move |_: &Cancel, _, _| {
                parent_cancels.set(parent_cancels.get() + 1);
            })
            .child(Combobox::new(&self.state))
    }
}

struct DisabledComboboxHarness {
    state: Entity<ComboboxState<SearchableVec<&'static str>>>,
    disabled: bool,
}

impl Render for DisabledComboboxHarness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Combobox::new(&self.state).disabled(self.disabled)
    }
}

#[derive(Clone)]
struct DisabledItem(&'static str);

impl SearchableListItem for DisabledItem {
    type Value = &'static str;

    fn title(&self) -> SharedString {
        self.0.into()
    }

    fn value(&self) -> &Self::Value {
        &self.0
    }

    fn disabled(&self) -> bool {
        true
    }
}

#[derive(Clone)]
struct A11yItem {
    title: &'static str,
    disabled: bool,
}

impl SearchableListItem for A11yItem {
    type Value = &'static str;

    fn title(&self) -> SharedString {
        self.title.into()
    }

    fn value(&self) -> &Self::Value {
        &self.title
    }

    fn disabled(&self) -> bool {
        self.disabled
    }
}

struct ComboboxA11yHarness {
    state: Entity<ComboboxState<SearchableVec<A11yItem>>>,
}

impl Render for ComboboxA11yHarness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Combobox::new(&self.state)
    }
}

#[gpui::test]
fn test_combo_box_builder(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let cx = cx.add_empty_window();
    cx.update(|window, cx| {
        let items = SearchableVec::new(vec!["Rust", "Go", "C++"]);
        let state = cx.new(|cx| ComboboxState::new(items, vec![], window, cx).searchable(true));

        let _cb = Combobox::new(&state)
            .placeholder("Select language")
            .search_placeholder("Search...")
            .menu_width(gpui::px(300.))
            .menu_max_h(gpui::rems(15.))
            .cleanable(true)
            .disabled(false)
            .appearance(true);
    });
}

#[gpui::test]
fn test_combo_box_search_filters_items(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let cx = cx.add_empty_window();
    cx.update(|window, cx| {
        let items = SearchableVec::new(vec!["Rust", "Go", "C++"]);
        let state = cx.new(|cx| ComboboxState::new(items, vec![], window, cx).searchable(true));

        let count_before = state
            .read(cx)
            .state
            .list
            .read(cx)
            .delegate()
            .delegate
            .items_count(0);
        assert_eq!(count_before, 3);

        state.update(cx, |s, cx| {
            s.state.list.update(cx, |list, cx| {
                let _ = list
                    .delegate_mut()
                    .delegate
                    .perform_search("Rust", window, cx);
            });
        });

        let count_after = state
            .read(cx)
            .state
            .list
            .read(cx)
            .delegate()
            .delegate
            .items_count(0);
        assert_eq!(count_after, 1);
    });
}

#[gpui::test]
fn test_combo_box_set_query_updates_text_and_filters_items(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let cx = cx.add_empty_window();
    cx.update(|window, cx| {
        let items = SearchableVec::new(vec!["Rust", "Go", "C++"]);
        let state = cx.new(|cx| ComboboxState::new(items, vec![], window, cx).searchable(true));

        state.update(cx, |state, cx| state.set_query(" Rust ", window, cx));

        assert_eq!(state.read(cx).query(cx).as_ref(), " Rust ");
        assert_eq!(
            state
                .read(cx)
                .state
                .list
                .read(cx)
                .delegate()
                .delegate
                .items_count(0),
            1,
        );
    });
}

#[gpui::test]
fn test_multi_combo_box_builder(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let cx = cx.add_empty_window();
    cx.update(|window, cx| {
        let items = SearchableVec::new(vec!["React", "Vue", "Angular"]);
        let state = cx.new(|cx| {
            ComboboxState::new(items, vec![IndexPath::new(0)], window, cx)
                .multiple(true)
                .searchable(true)
        });

        let _cb = Combobox::new(&state)
            .placeholder("Select frameworks")
            .search_placeholder("Search...")
            .menu_width(gpui::px(300.))
            .cleanable(true)
            .disabled(false);

        assert_eq!(state.read(cx).selected_values(), vec!["React"]);
    });
}

#[gpui::test]
fn test_combo_box_set_selected_values_uses_current_delegate(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let cx = cx.add_empty_window();
    cx.update(|window, cx| {
        let items = SearchableVec::new(vec!["React", "Vue", "Angular"]);
        let state = cx.new(|cx| ComboboxState::new(items, vec![], window, cx).multiple(true));

        state.update(cx, |state, cx| {
            state.set_selected_values(&["Vue", "Missing"], window, cx);

            assert_eq!(state.selected_values(), vec!["Vue"]);
            assert_eq!(
                state
                    .selection()
                    .iter()
                    .map(|(index, _)| *index)
                    .collect::<Vec<_>>(),
                vec![IndexPath::new(1)],
            );
            assert_eq!(
                state
                    .state
                    .list
                    .read(cx)
                    .delegate()
                    .selection_snapshot
                    .as_slice(),
                state.selection(),
            );

            state.set_items(SearchableVec::new(vec!["Vue", "Rust", "Go"]), window, cx);
            state.set_selected_values(&["Go", "Vue"], window, cx);

            assert_eq!(state.selected_values(), vec!["Go", "Vue"]);
            assert_eq!(
                state
                    .selection()
                    .iter()
                    .map(|(index, _)| *index)
                    .collect::<Vec<_>>(),
                vec![IndexPath::new(2), IndexPath::new(0)],
            );
            assert_eq!(
                state
                    .state
                    .list
                    .read(cx)
                    .delegate()
                    .selection_snapshot
                    .as_slice(),
                state.selection(),
            );

            state.set_selected_values(&[], window, cx);

            assert!(state.selection().is_empty());
            assert!(
                state
                    .state
                    .list
                    .read(cx)
                    .delegate()
                    .selection_snapshot
                    .is_empty()
            );
        });
    });
}

#[gpui::test]
fn test_combo_box_set_selected_values_does_not_emit_events(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let cx = cx.add_empty_window();
    let state = cx.update(|window, cx| {
        let items = SearchableVec::new(vec!["React", "Vue", "Angular"]);
        cx.new(|cx| ComboboxState::new(items, vec![], window, cx).multiple(true))
    });
    let collector = cx.update(|_, cx| cx.new(|cx| TestComboboxEventCollector::new(&state, cx)));

    cx.update(|window, cx| {
        state.update(cx, |state, cx| {
            state.set_selected_values(&["React", "Vue"], window, cx);
        });
    });

    cx.update(|_, cx| {
        assert_eq!(collector.read(cx).event_count.get(), 0);
    });
}

#[gpui::test]
fn test_single_combo_box_setters_keep_one_selection(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let cx = cx.add_empty_window();
    let state = cx.update(|window, cx| {
        let items = SearchableVec::new(vec!["Rust", "Go", "C++"]);
        cx.new(|cx| ComboboxState::new(items, vec![], window, cx))
    });
    let collector = cx.update(|_, cx| cx.new(|cx| TestComboboxEventCollector::new(&state, cx)));

    cx.update(|window, cx| {
        state.update(cx, |state, cx| {
            state.set_selected_indices([IndexPath::new(0), IndexPath::new(1)], window, cx);
            assert_eq!(state.selected_values(), vec!["Rust"]);
            assert_eq!(
                state
                    .state
                    .list
                    .read(cx)
                    .delegate()
                    .selection_snapshot
                    .as_slice(),
                state.selection(),
            );

            state.set_selected_values(&["C++", "Go"], window, cx);
            assert_eq!(state.selected_values(), vec!["C++"]);
            assert_eq!(
                state
                    .state
                    .list
                    .read(cx)
                    .delegate()
                    .selection_snapshot
                    .as_slice(),
                state.selection(),
            );
        });
    });

    cx.update(|_, cx| {
        assert_eq!(collector.read(cx).event_count.get(), 0);
    });
}

#[gpui::test]
fn test_combo_box_initial_selection_seeds_cursor(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let cx = cx.add_empty_window();
    cx.update(|window, cx| {
        let items = SearchableVec::new(vec!["React", "Vue", "Angular"]);
        let state = cx.new(|cx| {
            ComboboxState::new(items, vec![IndexPath::new(1)], window, cx).multiple(true)
        });

        let state_ref = state.read(cx);
        assert_eq!(
            state_ref.state.list.read(cx).selected_index(),
            Some(IndexPath::new(1)),
            "initial selected_indices should seed ListState.selected_index, not just the snapshot",
        );
        assert_eq!(state_ref.selected_values(), vec!["Vue"]);
    });
}

#[gpui::test]
fn test_multi_combo_box_toggle(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let cx = cx.add_empty_window();
    cx.update(|window, cx| {
        let items = SearchableVec::new(vec!["React", "Vue", "Angular"]);
        let state = cx.new(|cx| ComboboxState::new(items, vec![], window, cx).multiple(true));

        state.update(cx, |s, cx| s.add_selected_index(IndexPath::new(0), cx));
        assert_eq!(state.read(cx).selected_values(), &["React"]);

        state.update(cx, |s, cx| s.add_selected_index(IndexPath::new(1), cx));
        assert_eq!(state.read(cx).selected_values(), &["React", "Vue"]);

        state.update(cx, |s, cx| s.remove_selected_index(IndexPath::new(0), cx));
        assert_eq!(state.read(cx).selected_values(), &["Vue"]);
    });
}

#[gpui::test]
fn test_remove_selected_index_notifies_and_updates_snapshot(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let cx = cx.add_empty_window();
    let state = cx.update(|window, cx| {
        let items = SearchableVec::new(vec!["React", "Vue"]);
        cx.new(|cx| ComboboxState::new(items, vec![IndexPath::new(0)], window, cx).multiple(true))
    });
    let observer = cx.update(|_, cx| cx.new(|cx| TestComboboxObserver::new(&state, cx)));

    let before = cx.update(|_, cx| observer.read(cx).notifications.get());
    cx.update(|_, cx| {
        state.update(cx, |state, cx| {
            assert!(state.remove_selected_index(IndexPath::new(0), cx));
        });
    });
    cx.run_until_parked();

    cx.update(|_, cx| {
        assert!(state.read(cx).selection().is_empty());
        assert!(
            state
                .read(cx)
                .state
                .list
                .read(cx)
                .delegate()
                .selection_snapshot
                .is_empty()
        );
        assert!(observer.read(cx).notifications.get() > before);
    });
}

#[gpui::test]
fn test_set_items_notifies_and_preserves_valid_selection(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let cx = cx.add_empty_window();
    let state = cx.update(|window, cx| {
        let items = SearchableVec::new(vec!["React", "Vue", "Angular"]);
        cx.new(|cx| ComboboxState::new(items, vec![IndexPath::new(1)], window, cx))
    });
    let observer = cx.update(|_, cx| cx.new(|cx| TestComboboxObserver::new(&state, cx)));

    let before = cx.update(|_, cx| observer.read(cx).notifications.get());
    cx.update(|window, cx| {
        state.update(cx, |state, cx| {
            state.set_items(SearchableVec::new(vec!["Rust", "Vue", "Go"]), window, cx);
        });
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        let state = state.read(cx);
        assert_eq!(state.selected_values(), vec!["Vue"]);
        assert_eq!(
            state.state.list.read(cx).selected_index(),
            Some(IndexPath::new(1))
        );
        assert_eq!(
            state.state.list.read(cx).delegate().delegate.items_count(0),
            3
        );
        assert!(observer.read(cx).notifications.get() > before);
    });

    cx.update(|window, cx| {
        state.update(cx, |state, cx| {
            state.set_items(SearchableVec::new(vec!["Vue"]), window, cx);
        });
    });
    cx.update(|_, cx| {
        let state = state.read(cx);
        assert_eq!(state.selected_values(), vec!["Vue"]);
        assert_eq!(
            state.state.list.read(cx).selected_index(),
            Some(IndexPath::new(0))
        );
        assert_eq!(
            state.state.list.read(cx).delegate().delegate.items_count(0),
            1
        );
    });

    cx.update(|window, cx| {
        state.update(cx, |state, cx| {
            state.set_items(SearchableVec::new(vec!["Rust"]), window, cx);
        });
    });
    cx.update(|_, cx| {
        let state = state.read(cx);
        assert!(state.selection().is_empty());
        assert_eq!(state.state.list.read(cx).selected_index(), None);
        assert!(
            state
                .state
                .list
                .read(cx)
                .delegate()
                .selection_snapshot
                .is_empty()
        );
    });
}

#[gpui::test]
fn test_disabled_items_do_not_confirm_from_pointer_or_keyboard(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let cx = cx.add_empty_window();
    let state = cx.update(|window, cx| {
        let items = SearchableVec::new(vec![DisabledItem("Locked")]);
        cx.new(|cx| ComboboxState::new(items, vec![], window, cx))
    });

    cx.update(|window, cx| {
        state.update(cx, |state, cx| {
            state.set_open(true, cx);
            state.handle_item_select(IndexPath::new(0), window, cx);
        });
    });
    cx.update(|_, cx| {
        assert!(state.read(cx).selection().is_empty());
        assert!(state.read(cx).state.open);
    });

    cx.update(|window, cx| {
        state.update(cx, |state, cx| {
            state.state.list.update(cx, |list, cx| {
                list.set_selected_index(Some(IndexPath::new(0)), window, cx);
                list.delegate_mut().confirm(false, window, cx);
            });
        });
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        assert!(state.read(cx).selection().is_empty());
        assert!(state.read(cx).state.open);
    });
}

#[gpui::test]
fn test_disabling_open_combobox_closes_and_blocks_interaction(cx: &mut TestAppContext) {
    let window = cx.update(|cx| {
        crate::init(cx);
        cx.open_window(Default::default(), |window, cx| {
            let state = cx.new(|cx| {
                ComboboxState::new(SearchableVec::new(vec!["Rust", "Go"]), vec![], window, cx)
            });
            cx.new(|_| DisabledComboboxHarness {
                state,
                disabled: false,
            })
        })
        .unwrap()
    });
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    let harness = window.root(&mut cx).unwrap();
    let state = harness.read_with(&cx, |harness, _| harness.state.clone());

    state.update_in(&mut cx, |state, _, cx| state.set_open(true, cx));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    state.read_with(&cx, |state, _| assert!(state.state.open));

    harness.update_in(&mut cx, |harness, _, cx| {
        harness.disabled = true;
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    state.read_with(&cx, |state, _| {
        assert!(state.state.disabled);
        assert!(!state.state.open);
    });

    state.update_in(&mut cx, |state, window, cx| {
        state.down(&SelectDown, window, cx);
        state.handle_item_select(IndexPath::new(0), window, cx);
    });
    state.read_with(&cx, |state, _| {
        assert!(!state.state.open);
        assert!(state.selection().is_empty());
    });
}

#[gpui::test]
fn test_combo_box_escape_confirms_once_and_restores_focus(cx: &mut TestAppContext) {
    let parent_cancels = Rc::new(Cell::new(0));
    let window = cx.update(|cx| {
        crate::init(cx);
        cx.open_window(Default::default(), {
            let parent_cancels = parent_cancels.clone();
            move |window, cx| {
                let state = cx.new(|cx| {
                    ComboboxState::new(SearchableVec::new(vec!["Rust", "Go"]), vec![], window, cx)
                });
                cx.new(|_| ComboboxEscapeHarness {
                    state,
                    parent_cancels,
                })
            }
        })
        .unwrap()
    });
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    let harness = window.root(&mut cx).unwrap();
    let state = harness.read_with(&cx, |harness, _| harness.state.clone());
    let collector = cx.update(|_, cx| cx.new(|cx| TestComboboxEventCollector::new(&state, cx)));

    state.update_in(&mut cx, |state, window, cx| {
        state.set_open(true, cx);
        state.state.list.focus_handle(cx).focus(window, cx);
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));

    cx.dispatch_action(Cancel);
    cx.run_until_parked();
    state.read_with(&cx, |state, _| assert!(!state.state.open));
    cx.update(|window, cx| assert!(state.read(cx).state.focus_handle.is_focused(window)));
    assert_eq!(
        collector.read_with(&cx, |collector, _| collector.event_count.get()),
        1
    );
    assert_eq!(parent_cancels.get(), 0);

    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.dispatch_action(Cancel);
    cx.run_until_parked();
    assert_eq!(parent_cancels.get(), 1);
    assert_eq!(
        collector.read_with(&cx, |collector, _| collector.event_count.get()),
        1
    );
}

#[gpui::test]
fn test_escape_and_blur_commit_delegate_once(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let cx = cx.add_empty_window();
    let confirms = Rc::new(Cell::new(0));
    let state = cx.update(|window, cx| {
        cx.new(|cx| {
            ComboboxState::new(
                ConfirmTrackingDelegate::new(confirms.clone()),
                vec![],
                window,
                cx,
            )
        })
    });
    let collector = cx.update(|_, cx| cx.new(|cx| TrackedEventCollector::new(&state, cx)));

    cx.update(|window, cx| {
        state.update(cx, |state, cx| {
            state.set_open(true, cx);
            state.state.list.update(cx, |list, cx| {
                list.delegate_mut().cancel(window, cx);
            });
        });
    });
    cx.run_until_parked();
    assert_eq!(confirms.get(), 1);
    assert_eq!(
        collector.read_with(cx, |collector, _| collector.confirm_count.get()),
        1
    );

    cx.update(|window, cx| {
        window.blur();
        state.update(cx, |state, cx| {
            state.set_open(true, cx);
            state.on_blur(window, cx);
        });
    });
    assert_eq!(confirms.get(), 2);
    assert_eq!(
        collector.read_with(cx, |collector, _| collector.confirm_count.get()),
        2
    );
}

#[gpui::test]
fn test_trigger_and_pointer_close_commit_delegate_once(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let cx = cx.add_empty_window();
    let confirms = Rc::new(Cell::new(0));
    let state = cx.update(|window, cx| {
        cx.new(|cx| {
            ComboboxState::new(
                ConfirmTrackingDelegate::new(confirms.clone()),
                vec![],
                window,
                cx,
            )
            .multiple(true)
        })
    });
    let collector = cx.update(|_, cx| cx.new(|cx| TrackedEventCollector::new(&state, cx)));

    cx.update(|window, cx| {
        state.update(cx, |state, cx| {
            state.set_open(true, cx);
            state.toggle_menu(&ClickEvent::default(), window, cx);
            state.set_open(true, cx);
            state.dismiss(&left_press(point(px(1.), px(1.))), window, cx);
        });
    });

    assert_eq!(confirms.get(), 2);
    assert_eq!(
        collector.read_with(cx, |collector, _| collector.confirm_count.get()),
        2
    );
}

#[gpui::test]
fn test_item_confirm_commits_delegate_once(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let cx = cx.add_empty_window();
    let confirms = Rc::new(Cell::new(0));
    let state = cx.update(|window, cx| {
        cx.new(|cx| {
            ComboboxState::new(
                ConfirmTrackingDelegate::new(confirms.clone()),
                vec![],
                window,
                cx,
            )
        })
    });
    let collector = cx.update(|_, cx| cx.new(|cx| TrackedEventCollector::new(&state, cx)));

    cx.update(|window, cx| {
        state.update(cx, |state, cx| {
            state.set_open(true, cx);
            state.state.list.update(cx, |list, cx| {
                list.set_selected_index(Some(IndexPath::new(0)), window, cx);
                list.delegate_mut().confirm(false, window, cx);
            });
        });
    });
    cx.run_until_parked();

    assert_eq!(confirms.get(), 1);
    assert_eq!(
        collector.read_with(cx, |collector, _| collector.confirm_count.get()),
        1
    );
    assert!(!state.read_with(cx, |state, _| state.state.open));
}

#[gpui::test]
fn test_selected_item_confirm_closes_and_commits_once(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let cx = cx.add_empty_window();
    let confirms = Rc::new(Cell::new(0));
    let state = cx.update(|window, cx| {
        cx.new(|cx| {
            ComboboxState::new(
                ConfirmTrackingDelegate::new(confirms.clone()),
                vec![IndexPath::new(0)],
                window,
                cx,
            )
        })
    });
    let collector = cx.update(|_, cx| cx.new(|cx| TrackedEventCollector::new(&state, cx)));

    cx.update(|window, cx| {
        state.update(cx, |state, cx| {
            state.set_open(true, cx);
            state.handle_item_select(IndexPath::new(0), window, cx);
        });
    });

    assert_eq!(confirms.get(), 1);
    assert_eq!(
        collector.read_with(cx, |collector, _| collector.confirm_count.get()),
        1
    );
    assert_eq!(
        collector.read_with(cx, |collector, _| collector.change_count.get()),
        0
    );
    assert!(!state.read_with(cx, |state, _| state.state.open));
}

#[gpui::test]
fn test_same_index_value_change_emits_change(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let cx = cx.add_empty_window();
    let state = cx.update(|window, cx| {
        cx.new(|cx| {
            ComboboxState::new(
                SameIndexValueDelegate {
                    item: VersionedItem {
                        title: "Original",
                        value: "original",
                    },
                    replacement: VersionedItem {
                        title: "Replacement",
                        value: "replacement",
                    },
                },
                vec![IndexPath::new(0)],
                window,
                cx,
            )
        })
    });
    let collector = cx.update(|_, cx| cx.new(|cx| TrackedEventCollector::new(&state, cx)));

    cx.update(|window, cx| {
        state.update(cx, |state, cx| {
            state.set_open(true, cx);
            state.handle_item_select(IndexPath::new(0), window, cx);
        });
    });

    assert_eq!(
        state.read_with(cx, |state, _| state.selected_values()),
        ["replacement"]
    );
    assert_eq!(
        collector.read_with(cx, |collector, _| collector.change_count.get()),
        1
    );
    assert_eq!(
        collector.read_with(cx, |collector, _| collector.confirm_count.get()),
        1
    );
    assert!(!state.read_with(cx, |state, _| state.state.open));
}

#[gpui::test]
fn test_open_combobox_exposes_selected_and_disabled_options(cx: &mut TestAppContext) {
    let window = cx.update(|cx| {
        crate::init(cx);
        cx.open_window(Default::default(), |window, cx| {
            let state = cx.new(|cx| {
                ComboboxState::new(
                    SearchableVec::new(vec![
                        A11yItem {
                            title: "Selected option",
                            disabled: false,
                        },
                        A11yItem {
                            title: "Cursor option",
                            disabled: false,
                        },
                        A11yItem {
                            title: "Disabled option",
                            disabled: true,
                        },
                    ]),
                    vec![IndexPath::new(0)],
                    window,
                    cx,
                )
            });
            state.update(cx, |state, cx| {
                state.set_open(true, cx);
                state.state.list.update(cx, |list, cx| {
                    list.set_selected_index(Some(IndexPath::new(1)), window, cx);
                });
            });
            cx.new(|_| ComboboxA11yHarness { state })
        })
        .unwrap()
    });
    let mut cx = VisualTestContext::from_window(window.into(), cx);

    let update = cx.update(|window, cx| {
        window.set_a11y_active_for_test(true);
        window.draw(cx).clear(cx);
        window
            .last_a11y_tree_for_test()
            .cloned()
            .expect("open combobox should publish an accessibility tree")
    });
    let options = update
        .nodes
        .iter()
        .filter_map(|(_, node)| (node.role() == Role::ListBoxOption).then_some(node))
        .collect::<Vec<_>>();

    assert_eq!(options.len(), 3);
    assert_eq!(options[0].is_selected(), Some(true));
    assert_eq!(options[1].is_selected(), Some(false));
    assert!(
        options
            .iter()
            .any(|node| node.is_selected() == Some(true) && !node.is_disabled())
    );
    assert!(options.iter().any(|node| node.is_disabled()));
}

#[gpui::test]
fn test_multi_combo_box_search_selection_uses_value_identity(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let cx = cx.add_empty_window();
    cx.update(|window, cx| {
        let items = SearchableVec::new(vec!["React", "Vue", "Angular"]);
        let state = cx.new(|cx| ComboboxState::new(items, vec![], window, cx).multiple(true));

        state.update(cx, |s, cx| s.add_selected_index(IndexPath::new(0), cx));
        assert_eq!(state.read(cx).selected_values(), &["React"]);

        state.update(cx, |s, cx| {
            s.state.list.update(cx, |list, cx| {
                let _ = list
                    .delegate_mut()
                    .delegate
                    .perform_search("Vue", window, cx);
            });
        });

        state.read_with(cx, |s, cx| {
            let selection = s.state.selection.clone();
            let list = s.state.list.read(cx);
            let delegate = &list.delegate().delegate;
            let ix = IndexPath::new(0);
            let item = delegate.item(ix).expect("filtered item exists");

            assert_eq!(item.value(), &"Vue");
            assert!(
                !delegate.is_item_checked(ix, item, &selection, cx),
                "filtered row 0 should not inherit React's checked state",
            );
        });

        state.update(cx, |s, cx| {
            s.handle_item_select(IndexPath::new(0), window, cx);
        });
        assert_eq!(state.read(cx).selected_values(), &["React", "Vue"]);
    });
}

#[gpui::test]
fn test_multi_combo_box_search_deselects_by_value(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let cx = cx.add_empty_window();
    cx.update(|window, cx| {
        let items = SearchableVec::new(vec!["React", "Vue", "Angular"]);
        let state = cx.new(|cx| ComboboxState::new(items, vec![], window, cx).multiple(true));

        state.update(cx, |s, cx| s.add_selected_index(IndexPath::new(0), cx));

        state.update(cx, |s, cx| {
            s.state.list.update(cx, |list, cx| {
                let _ = list
                    .delegate_mut()
                    .delegate
                    .perform_search("React", window, cx);
            });
        });

        state.update(cx, |s, cx| {
            s.handle_item_select(IndexPath::new(0), window, cx);
        });
        assert!(state.read(cx).selected_values().is_empty());
    });
}

#[gpui::test]
fn test_searchable_list_default_change_uses_value_identity(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let cx = cx.add_empty_window();
    cx.update(|window, cx| {
        let mut delegate = SearchableVec::new(vec!["React", "Vue", "Angular"]);
        let mut selection = vec![(IndexPath::new(1), "Vue")];

        let _ = delegate.perform_search("Vue", window, cx);
        delegate.on_will_change(
            &mut selection,
            &[SearchableListChange::Deselect {
                index: IndexPath::new(0),
            }],
        );
        assert!(selection.is_empty());

        delegate.on_will_change(
            &mut selection,
            &[SearchableListChange::Select {
                index: IndexPath::new(0),
            }],
        );
        assert_eq!(selection, vec![(IndexPath::new(0), "Vue")]);
    });
}

#[gpui::test]
fn test_multi_combo_box_clear(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let cx = cx.add_empty_window();
    cx.update(|window, cx| {
        let items = SearchableVec::new(vec!["React", "Vue", "Angular"]);
        let state = cx.new(|cx| {
            ComboboxState::new(
                items,
                vec![IndexPath::new(0), IndexPath::new(1)],
                window,
                cx,
            )
            .multiple(true)
        });

        assert_eq!(state.read(cx).selected_values().len(), 2);
        state.update(cx, |s, cx| s.clear_selection(cx));
        assert!(state.read(cx).selected_values().is_empty());
    });
}

#[gpui::test]
fn test_single_combo_box_add_replaces_selection(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let cx = cx.add_empty_window();
    let state = cx.update(|window, cx| {
        let items = SearchableVec::new(vec!["Rust", "Go", "C++"]);
        cx.new(|cx| ComboboxState::new(items, vec![], window, cx))
    });
    let collector = cx.update(|_, cx| cx.new(|cx| TestComboboxEventCollector::new(&state, cx)));

    cx.update(|_, cx| {
        // Default mode is Single.
        assert!(state.update(cx, |s, cx| s.add_selected_index(IndexPath::new(0), cx)));
        assert_eq!(state.read(cx).selected_values(), &["Rust"]);

        assert!(state.update(cx, |s, cx| s.add_selected_index(IndexPath::new(1), cx)));
        assert!(!state.update(cx, |s, cx| s.add_selected_index(IndexPath::new(1), cx)));
        assert!(!state.update(cx, |s, cx| s.add_selected_index(IndexPath::new(99), cx)));
        assert_eq!(state.read(cx).selected_values(), &["Go"]);
        assert_eq!(
            state
                .read(cx)
                .state
                .list
                .read(cx)
                .delegate()
                .selection_snapshot
                .as_slice(),
            state.read(cx).selection(),
        );
        assert_eq!(collector.read(cx).event_count.get(), 0);
    });
}

#[gpui::test]
fn test_combobox_render_clears_omitted_empty_builder(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let cx = cx.add_empty_window();
    cx.update(|window, cx| {
        let state =
            cx.new(|cx| ComboboxState::new(SearchableVec::new(vec!["Rust"]), vec![], window, cx));

        _ = Combobox::new(&state).empty(|_, _| div()).render(window, cx);
        assert!(state.read(cx).state.empty.is_some());

        _ = Combobox::new(&state).render(window, cx);
        assert!(state.read(cx).state.empty.is_none());
    });
}

// Delegate that vetoes all selections via on_will_change by ignoring the changes.
struct VetoDelegate(SearchableVec<&'static str>);

impl SearchableListDelegate for VetoDelegate {
    type Item = &'static str;

    fn items_count(&self, section: usize) -> usize {
        self.0.items_count(section)
    }

    fn item(&self, ix: IndexPath) -> Option<&&'static str> {
        self.0.item(ix)
    }

    fn position<V>(&self, value: &V) -> Option<IndexPath>
    where
        &'static str: SearchableListItem<Value = V>,
        V: PartialEq,
    {
        self.0.position(value)
    }

    fn on_will_change(
        &mut self,
        _selection: &mut Vec<(IndexPath, &'static str)>,
        _changes: &[SearchableListChange],
    ) {
        // Leave selection unchanged — acts as a veto.
    }
}

#[gpui::test]
fn test_on_will_change_veto(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let cx = cx.add_empty_window();
    cx.update(|window, cx| {
        let delegate = VetoDelegate(SearchableVec::new(vec!["Rust", "Go", "C++"]));
        let state = cx.new(|cx| ComboboxState::new(delegate, vec![], window, cx));

        // Pre-select an item directly so we can verify veto prevents changes.
        state.update(cx, |s, cx| s.add_selected_index(IndexPath::new(0), cx));
        assert_eq!(state.read(cx).selected_values(), &["Rust"]);

        // Simulate a click on index 1 via handle_item_select; on_will_change vetoes it.
        state.update(cx, |s, cx| {
            s.handle_item_select(IndexPath::new(1), window, cx);
        });

        // Selection must remain unchanged because on_will_change left it unmodified.
        assert_eq!(state.read(cx).selected_values(), &["Rust"]);
    });
}

fn left_press(position: Point<Pixels>) -> MouseDownEvent {
    MouseDownEvent {
        button: MouseButton::Left,
        position,
        modifiers: Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    }
}

#[gpui::test]
fn test_combo_box_dismiss_ignores_press_on_trigger(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let cx = cx.add_empty_window();
    let state = cx.update(|window, cx| {
        let items = SearchableVec::new(vec!["React", "Vue", "Angular"]);
        cx.new(|cx| ComboboxState::new(items, vec![], window, cx).multiple(true))
    });
    let collector = cx.update(|_, cx| cx.new(|cx| TestComboboxEventCollector::new(&state, cx)));

    cx.update(|window, cx| {
        state.update(cx, |state, cx| {
            state.state.bounds = Bounds {
                origin: point(px(10.), px(10.)),
                size: size(px(200.), px(32.)),
            };
            state.set_open(true, cx);
            state.dismiss(&left_press(point(px(20.), px(20.))), window, cx);
        });
    });

    cx.update(|_, cx| {
        assert!(
            state.read(cx).state.open,
            "a press on the trigger must reach nested controls, so the menu stays open \
             and `toggle_menu` closes it on release instead",
        );
        assert_eq!(collector.read(cx).event_count.get(), 0);
    });

    cx.update(|window, cx| {
        state.update(cx, |state, cx| {
            state.dismiss(&left_press(point(px(20.), px(200.))), window, cx);
        });
    });

    cx.update(|_, cx| {
        assert!(!state.read(cx).state.open);
        assert_eq!(
            collector.read(cx).event_count.get(),
            1,
            "dismissing from outside the trigger emits Confirm",
        );
    });
}

// Suppress unused import warning for SearchableListState in test module.
#[allow(unused)]
fn _uses_state<D: SearchableListDelegate + 'static>(_: &SearchableListState<D>)
where
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
}
