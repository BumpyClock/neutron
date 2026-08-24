use gpui::{
    AnyElement, App, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    Styled as _, Window, div, px,
};

use crate::{
    ActiveTheme as _, Disableable as _, Icon, IconName, IndexPath, Sizable as _, Size,
    StyleSized as _, flyout_secondary_foreground, h_flex,
    list::{ListDelegate, ListState},
};

use super::{
    delegate::{SearchableListDelegate, SearchableListItem as _},
    item::SearchableListItemElement,
};

/// Bridges a [`SearchableListDelegate`] into the [`ListDelegate`] protocol.
///
/// Parent states (`SelectState`, `ComboBoxState`) create one of these, supplying closures that
/// encode parent-specific confirm / cancel / empty-render behaviour. All state mutation stays in
/// the parent; the adapter only handles item layout.
pub(crate) struct SearchableListAdapter<D: SearchableListDelegate + 'static> {
    pub(crate) delegate: D,
    /// Keyboard cursor row — updated by `ListDelegate::set_selected_index`.
    selected_index: Option<IndexPath>,
    /// Snapshot of the parent's committed selection, kept in sync by the parent state after every
    /// selection change. `render_item` reads this directly so it never touches the parent entity
    /// (which would panic — the `ListState` entity is already locked during render).
    pub(crate) selection_snapshot: Vec<(IndexPath, D::Item)>,
    /// Called when the user confirms an item (click or Enter).
    on_confirm:
        Box<dyn Fn(Option<IndexPath>, bool, &mut Window, &mut Context<ListState<Self>>) + 'static>,
    /// Called when the user cancels (Escape) or focus leaves the dropdown.
    on_cancel: Box<dyn Fn(Option<IndexPath>, &mut Window, &mut Context<ListState<Self>>) + 'static>,
    /// Renders the empty-state placeholder.
    on_render_empty: Option<Box<dyn Fn(&mut Window, &mut App) -> Option<AnyElement> + 'static>>,
    pub(crate) size: Size,
    /// Override the trailing check icon; defaults to `IconName::Check`.
    pub(crate) check_icon: Option<Icon>,
}

impl<D: SearchableListDelegate + 'static> SearchableListAdapter<D> {
    pub(crate) fn new_with_empty(
        delegate: D,
        on_confirm: impl Fn(Option<IndexPath>, bool, &mut Window, &mut Context<ListState<Self>>)
        + 'static,
        on_cancel: impl Fn(Option<IndexPath>, &mut Window, &mut Context<ListState<Self>>) + 'static,
        on_render_empty: Option<Box<dyn Fn(&mut Window, &mut App) -> Option<AnyElement> + 'static>>,
    ) -> Self {
        Self {
            delegate,
            selected_index: None,
            selection_snapshot: Vec::new(),
            on_confirm: Box::new(on_confirm),
            on_cancel: Box::new(on_cancel),
            on_render_empty,
            size: Size::default(),
            check_icon: None,
        }
    }

    /// Replace the selection snapshot. Call this after every selection mutation so that
    /// `render_item` sees up-to-date check state without touching any external entity.
    pub(crate) fn update_selection_snapshot(&mut self, snapshot: Vec<(IndexPath, D::Item)>) {
        self.selection_snapshot = snapshot;
    }
}

impl<D: SearchableListDelegate + 'static> ListDelegate for SearchableListAdapter<D> {
    type Item = SearchableListItemElement;

    fn sections_count(&self, cx: &App) -> usize {
        self.delegate.sections_count(cx)
    }

    fn items_count(&self, section: usize, _: &App) -> usize {
        self.delegate.items_count(section)
    }

    fn render_section_header(
        &mut self,
        section: usize,
        window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<impl IntoElement> {
        if let Some(el) = self.delegate.render_section_header(section, window, cx) {
            return Some(el.into_any_element());
        }

        #[allow(deprecated)]
        let item = self.delegate.section(section)?;

        Some(
            div()
                .py_0p5()
                .px_2()
                .list_size(self.size)
                .text_sm()
                .text_color(flyout_secondary_foreground(cx))
                .child(item)
                .into_any_element(),
        )
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        use gpui::IntoElement as _;

        let item = self.delegate.item(ix)?;
        // Read check state from the snapshot — never from an external entity, which would panic
        // because the ListState entity is already locked for this render pass.
        let is_checked = self
            .delegate
            .is_item_checked(ix, item, &self.selection_snapshot, cx);
        let disabled = !self.delegate.is_item_enabled(ix, item, cx);
        let size = self.size;

        if let Some(el) = self.delegate.render_item(ix, item, is_checked, window, cx) {
            return Some(
                SearchableListItemElement::new(ix.row)
                    .checked(is_checked)
                    .hide_check_icon()
                    .disabled(disabled)
                    .with_size(size)
                    .child(el),
            );
        }

        let check_icon = self
            .check_icon
            .clone()
            .unwrap_or_else(|| Icon::new(IconName::Check));

        let content = div()
            .whitespace_nowrap()
            .child(item.render(window, cx).into_any_element());

        Some(
            SearchableListItemElement::new(ix.row)
                .checked(is_checked)
                .check_icon(check_icon)
                .disabled(disabled)
                .with_size(size)
                .child(content.into_any_element()),
        )
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<ListState<Self>>) {
        // List enables propagation before it delegates cancellation. A searchable-list host
        // consumes Escape while open, so stop it again before the host callback closes the popup.
        cx.stop_propagation();
        let saved = self.selected_index;
        (self.on_cancel)(saved, window, cx);
    }

    fn confirm(&mut self, secondary: bool, window: &mut Window, cx: &mut Context<ListState<Self>>) {
        let Some(index) = self.selected_index else {
            return;
        };
        let Some(item) = self.delegate.item(index) else {
            return;
        };
        if !self.delegate.is_item_enabled(index, item, cx) {
            return;
        }

        (self.on_confirm)(self.selected_index, secondary, window, cx);
    }

    fn perform_search(
        &mut self,
        query: &str,
        window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> gpui::Task<()> {
        self.delegate.perform_search_with_context(query, window, cx)
    }

    fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) {
        self.selected_index = ix.filter(|index| {
            self.delegate
                .item(*index)
                .is_some_and(|item| self.delegate.is_item_enabled(*index, item, cx))
        });
    }

    fn render_empty(
        &mut self,
        window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        self.on_render_empty
            .as_ref()
            .and_then(|render| render(window, cx))
            .unwrap_or_else(|| {
                h_flex()
                    .justify_center()
                    .py_6()
                    .text_color(cx.theme().muted_foreground.opacity(0.6))
                    .debug_selector(|| "searchable-list-empty-default".into())
                    .child(Icon::new(IconName::Inbox).size(px(28.)))
                    .into_any_element()
            })
    }
}

#[cfg(test)]
mod tests {
    use gpui::{
        AppContext as _, IntoElement as _, ParentElement as _, TestAppContext, Window, div,
    };

    use crate::{
        IndexPath,
        list::ListDelegate as _,
        list::ListState,
        searchable_list::{SearchableListAdapter, SearchableListDelegate, SearchableListItem},
    };

    #[derive(Clone)]
    struct CustomDelegate {
        items: Vec<&'static str>,
    }

    impl SearchableListDelegate for CustomDelegate {
        type Item = &'static str;

        fn items_count(&self, _: usize) -> usize {
            self.items.len()
        }

        fn item(&self, ix: IndexPath) -> Option<&Self::Item> {
            self.items.get(ix.row)
        }

        fn position<V>(&self, value: &V) -> Option<IndexPath>
        where
            Self::Item: SearchableListItem<Value = V>,
            V: PartialEq,
        {
            self.items
                .iter()
                .position(|item| item.value() == value)
                .map(IndexPath::new)
        }

        fn render_item(
            &self,
            _: IndexPath,
            item: &Self::Item,
            _: bool,
            _: &mut Window,
            _: &mut gpui::App,
        ) -> Option<gpui::AnyElement> {
            Some(div().child(item.title()).into_any_element())
        }
    }

    #[gpui::test]
    fn custom_rows_keep_checked_state_after_cursor_moves(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();

        cx.update(|window, cx| {
            let adapter = SearchableListAdapter::new_with_empty(
                CustomDelegate {
                    items: vec!["Selected", "Cursor"],
                },
                |_, _, _, _| {},
                |_, _, _| {},
                Some(Box::new(|_, _| Some(div().into_any_element()))),
            );
            let list = cx.new(|cx| ListState::new(adapter, window, cx));

            list.update(cx, |list, cx| {
                list.delegate_mut()
                    .update_selection_snapshot(vec![(IndexPath::new(0), "Selected")]);
                list.set_selected_index(Some(IndexPath::new(1)), window, cx);

                let row = list
                    .delegate_mut()
                    .render_item(IndexPath::new(0), window, cx)
                    .expect("custom row should render");
                assert!(row.is_checked());
            });
        });
    }
}
