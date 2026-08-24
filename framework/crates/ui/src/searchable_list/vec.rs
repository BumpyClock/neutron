use gpui::{App, SharedString, Task, Window};

use crate::IndexPath;

use super::delegate::{SearchableListDelegate, SearchableListItem};

// MARK: Primitive impls

impl SearchableListItem for String {
    type Value = Self;

    fn title(&self) -> SharedString {
        SharedString::from(self.clone())
    }

    fn value(&self) -> &Self::Value {
        self
    }
}

impl SearchableListItem for SharedString {
    type Value = Self;

    fn title(&self) -> SharedString {
        self.clone()
    }

    fn value(&self) -> &Self::Value {
        self
    }
}

impl SearchableListItem for &'static str {
    type Value = Self;

    fn title(&self) -> SharedString {
        SharedString::from(*self)
    }

    fn value(&self) -> &Self::Value {
        self
    }
}

// MARK: Vec delegate

impl<T: SearchableListItem + 'static> SearchableListDelegate for Vec<T> {
    type Item = T;

    fn items_count(&self, _: usize) -> usize {
        self.len()
    }

    fn item(&self, ix: IndexPath) -> Option<&Self::Item> {
        self.as_slice().get(ix.row)
    }

    fn position<V>(&self, value: &V) -> Option<IndexPath>
    where
        Self::Item: SearchableListItem<Value = V>,
        V: PartialEq,
    {
        self.iter()
            .position(|v| v.value() == value)
            .map(|ix| IndexPath::default().row(ix))
    }
}

// MARK: SearchableVec

/// A vector of items that supports incremental filtering.
///
/// On each `perform_search` call the `matched_items` view is rebuilt by filtering
/// the full `items` list.  Use this as a delegate when all data is already in memory.
#[derive(Debug, Clone)]
pub struct SearchableVec<T> {
    items: Vec<T>,
    matched_items: Vec<T>,
}

impl<T: Clone> SearchableVec<T> {
    /// Create a new `SearchableVec` from an initial list of items.
    pub fn new(items: impl Into<Vec<T>>) -> Self {
        let items = items.into();

        Self {
            items: items.clone(),
            matched_items: items,
        }
    }

    /// Append an item to both the master list and the current filtered view.
    pub fn push(&mut self, item: T) {
        self.items.push(item.clone());
        self.matched_items.push(item);
    }

    pub(crate) fn all_items(&self) -> &[T] {
        &self.items
    }

    pub(crate) fn matched_items(&self) -> &[T] {
        &self.matched_items
    }

    pub(crate) fn replace_matched_items(&mut self, items: Vec<T>) {
        self.matched_items = items;
    }

    pub(crate) fn filter_items(&mut self, predicate: impl Fn(&T) -> bool) {
        self.replace_matched_items(
            self.all_items()
                .iter()
                .filter(|item| predicate(item))
                .cloned()
                .collect(),
        );
    }

    pub(crate) fn find_position<V>(
        &self,
        value: &V,
        value_of: impl Fn(&T) -> &V,
    ) -> Option<IndexPath>
    where
        V: PartialEq,
    {
        self.matched_items()
            .iter()
            .position(|item| value_of(item) == value)
            .map(IndexPath::new)
    }

    pub(crate) fn filter_groups(
        &mut self,
        group_matches: impl Fn(&T) -> bool,
        mut filter_group: impl FnMut(&mut T) -> bool,
    ) where
        T: Clone,
    {
        self.replace_matched_items(
            self.all_items()
                .iter()
                .filter_map(|group| {
                    if group_matches(group) {
                        return Some(group.clone());
                    }

                    let mut group = group.clone();
                    filter_group(&mut group).then_some(group)
                })
                .collect(),
        );
    }

    pub(crate) fn find_group_position<I, V>(
        &self,
        value: &V,
        items_of: impl Fn(&T) -> &[I],
        value_of: impl Fn(&I) -> &V,
    ) -> Option<IndexPath>
    where
        V: PartialEq,
    {
        self.matched_items()
            .iter()
            .enumerate()
            .find_map(|(section, group)| {
                items_of(group)
                    .iter()
                    .position(|item| value_of(item) == value)
                    .map(|row| IndexPath::default().section(section).row(row))
            })
    }
}

impl<T: Clone> From<Vec<T>> for SearchableVec<T> {
    fn from(items: Vec<T>) -> Self {
        Self {
            items: items.clone(),
            matched_items: items,
        }
    }
}

impl<I: SearchableListItem + 'static> SearchableListDelegate for SearchableVec<I> {
    type Item = I;

    fn items_count(&self, _: usize) -> usize {
        self.matched_items().len()
    }

    fn item(&self, ix: IndexPath) -> Option<&Self::Item> {
        self.matched_items().get(ix.row)
    }

    fn position<V>(&self, value: &V) -> Option<IndexPath>
    where
        Self::Item: SearchableListItem<Value = V>,
        V: PartialEq,
    {
        self.find_position(value, |item| item.value())
    }

    fn perform_search(&mut self, query: &str, _: &mut Window, _: &mut App) -> Task<()> {
        self.filter_items(|item| item.matches(query));

        Task::ready(())
    }
}

// MARK: SearchableGroup

/// A named group of items used for sectioned lists.
#[derive(Debug, Clone)]
pub struct SearchableGroup<I> {
    pub title: SharedString,
    pub items: Vec<I>,
}

impl<I> SearchableGroup<I> {
    /// Create an empty group with the given section title.
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            items: vec![],
        }
    }

    /// Append a single item to this group.
    pub fn item(mut self, item: I) -> Self {
        self.items.push(item);
        self
    }

    /// Append multiple items to this group.
    pub fn items(mut self, items: impl IntoIterator<Item = I>) -> Self {
        self.items.extend(items);
        self
    }

    pub(crate) fn retain_items(&mut self, mut f: impl FnMut(&I) -> bool) {
        self.items.retain(|item| f(item));
    }
}

impl<I: SearchableListItem + 'static> SearchableListDelegate for SearchableVec<SearchableGroup<I>> {
    type Item = I;

    fn sections_count(&self, _: &App) -> usize {
        self.matched_items().len()
    }

    fn items_count(&self, section: usize) -> usize {
        self.matched_items()
            .get(section)
            .map_or(0, |group| group.items.len())
    }

    fn section(&self, section: usize) -> Option<gpui::AnyElement> {
        use gpui::IntoElement as _;

        Some(
            self.matched_items
                .get(section)?
                .title
                .clone()
                .into_any_element(),
        )
    }

    fn item(&self, ix: IndexPath) -> Option<&Self::Item> {
        let section = self.matched_items().get(ix.section)?;

        section.items.get(ix.row)
    }

    fn position<V>(&self, value: &V) -> Option<IndexPath>
    where
        Self::Item: SearchableListItem<Value = V>,
        V: PartialEq,
    {
        self.find_group_position(value, |group| group.items.as_slice(), |item| item.value())
    }

    fn perform_search(&mut self, query: &str, _: &mut Window, _: &mut App) -> Task<()> {
        let normalized_query = query.to_lowercase();
        self.filter_groups(
            |group| group.title.to_lowercase().contains(&normalized_query),
            |group| {
                group.retain_items(|item| item.matches(query));
                !group.items.is_empty()
            },
        );

        Task::ready(())
    }
}

#[cfg(test)]
mod tests {
    use gpui::{SharedString, TestAppContext};

    use super::{SearchableGroup, SearchableListDelegate as _, SearchableVec};

    #[derive(Clone)]
    struct CaseSensitiveItem(&'static str);

    impl super::SearchableListItem for CaseSensitiveItem {
        type Value = &'static str;

        fn title(&self) -> SharedString {
            self.0.into()
        }

        fn value(&self) -> &Self::Value {
            &self.0
        }

        fn matches(&self, query: &str) -> bool {
            self.0.contains(query)
        }
    }

    #[gpui::test]
    fn group_search_preserves_children_when_title_matches(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();

        cx.update(|window, cx| {
            let mut groups = SearchableVec::new(vec![
                SearchableGroup::new("Frontend").items(["React", "Vue"]),
                SearchableGroup::new("Backend").item("Rust"),
            ]);

            let _ = groups.perform_search("front", window, cx);

            assert_eq!(groups.sections_count(cx), 1);
            assert_eq!(groups.items_count(0), 2);
            assert_eq!(groups.item(crate::IndexPath::new(0)), Some(&"React"));
            assert_eq!(groups.item(crate::IndexPath::new(1)), Some(&"Vue"));
        });
    }

    #[gpui::test]
    fn group_search_removes_empty_nonmatching_groups(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();

        cx.update(|window, cx| {
            let mut groups = SearchableVec::new(vec![
                SearchableGroup::new("Frontend").item("React"),
                SearchableGroup::new("Backend").item("Rust"),
            ]);

            let _ = groups.perform_search("python", window, cx);

            assert_eq!(groups.sections_count(cx), 0);
        });
    }

    #[gpui::test]
    fn group_search_preserves_original_query_for_custom_matchers(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();

        cx.update(|window, cx| {
            let mut groups = SearchableVec::new(vec![
                SearchableGroup::new("Other").item(CaseSensitiveItem("Rust")),
            ]);

            let _ = groups.perform_search("R", window, cx);

            assert_eq!(groups.sections_count(cx), 1);
            assert_eq!(groups.items_count(0), 1);
            assert_eq!(groups.item(crate::IndexPath::new(0)).unwrap().0, "Rust");
        });
    }
}
