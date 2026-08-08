//! Accessibility support, provided by [AccessKit][accesskit].
//!
//! User-facing guide-level docs live in [`crate::_accessibility`].

use crate::{App, Bounds, FocusId, GlobalElementId, Pixels, SharedString, Window};
use accesskit::{Action, NodeId, TreeUpdate};
use collections::{FxHashMap, FxHashSet};
use std::{
    hash::{Hash, Hasher},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

mod builder;

pub(crate) use builder::{A11yNodeBuilder, A11yPrepaintSnapshot};

/// The fixed AccessKit node ID used for the root of every window's a11y tree.
pub(crate) const ROOT_NODE_ID: NodeId = NodeId(0);

/// A listener for an accessibility action on a specific node.
pub(crate) type A11yActionListener =
    Box<dyn FnMut(Option<&accesskit::ActionData>, &mut Window, &mut App) + 'static>;

/// Per-window accessibility state.
pub(crate) struct A11y {
    force_disabled: bool,
    active_flag: Arc<AtomicBool>,
    active_this_frame: bool,
    node_ids: FxHashMap<GlobalElementId, NodeId>,
    visited_global_ids: FxHashSet<GlobalElementId>,
    next_node_id: u64,
    pub(crate) nodes: A11yNodeBuilder,
    pub(crate) focus_ids: FxHashMap<NodeId, FocusId>,
    pub(crate) node_bounds: FxHashMap<NodeId, Bounds<Pixels>>,
    pub(crate) action_listeners: FxHashMap<NodeId, Vec<(Action, A11yActionListener)>>,
    window_title: Option<SharedString>,
}

impl A11y {
    pub(crate) fn new(
        active_flag: Arc<AtomicBool>,
        force_disabled: bool,
        window_title: Option<SharedString>,
    ) -> Self {
        Self {
            force_disabled,
            active_flag,
            active_this_frame: false,
            node_ids: FxHashMap::default(),
            visited_global_ids: FxHashSet::default(),
            next_node_id: ROOT_NODE_ID.0 + 1,
            nodes: A11yNodeBuilder::new(),
            focus_ids: FxHashMap::default(),
            node_bounds: FxHashMap::default(),
            action_listeners: FxHashMap::default(),
            window_title,
        }
    }

    pub(crate) fn set_window_title(&mut self, title: impl Into<SharedString>) {
        self.window_title = Some(title.into());
    }

    pub(crate) fn sync_active_flag(&mut self) {
        self.active_this_frame = !self.force_disabled && self.active_flag.load(Ordering::SeqCst);
    }

    pub(crate) fn is_active(&self) -> bool {
        self.active_this_frame
    }

    pub(crate) fn set_focusable(&mut self, node_id: NodeId, focus_id: FocusId) {
        self.focus_ids.insert(node_id, focus_id);
    }

    pub(crate) fn set_focus(&mut self, node_id: NodeId) {
        if !self.focus_ids.contains_key(&node_id) {
            if cfg!(debug_assertions) {
                panic!("set_focus called for a node that was not registered with set_focusable");
            } else {
                log::warn!(
                    "a11y: set_focus called for a node that was not registered with set_focusable ({node_id:?})"
                );
            }
        }
        if self.nodes.has_node(node_id) {
            self.nodes.set_focus(node_id);
        }
    }

    pub(crate) fn set_active_descendant(&mut self, node_id: NodeId) {
        if self.nodes.node_is_focused(node_id) {
            if cfg!(debug_assertions) {
                panic!("set_active_descendant called on the focused node");
            } else {
                log::warn!("a11y: set_active_descendant called on the focused node ({node_id:?})");
            }
            return;
        }
        if self.nodes.has_node(node_id) && self.nodes.focus_is_ancestor_of_current() {
            self.nodes.set_active_descendant(node_id);
        }
    }

    /// Force accessibility active/inactive for tests without going through a platform adapter.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn set_active_for_test(&mut self, active: bool) {
        self.active_flag.store(active, Ordering::SeqCst);
        self.sync_active_flag();
    }

    pub(crate) fn begin_frame(&mut self) {
        self.focus_ids.clear();
        self.node_bounds.clear();
        self.action_listeners.clear();
        self.visited_global_ids.clear();
        self.nodes.begin_frame(self.window_title.as_ref());
    }

    pub(crate) fn node_id_for(&mut self, global_id: &GlobalElementId) -> NodeId {
        self.visited_global_ids.insert(global_id.clone());

        if let Some(node_id) = self.node_ids.get(global_id) {
            return *node_id;
        }

        let node_id = NodeId(self.next_node_id);
        debug_assert_ne!(node_id, ROOT_NODE_ID);
        self.next_node_id += 1;
        self.node_ids.insert(global_id.clone(), node_id);
        node_id
    }

    pub(crate) fn node_id_for_existing(&self, global_id: &GlobalElementId) -> Option<NodeId> {
        self.node_ids.get(global_id).copied()
    }

    pub(crate) fn end_frame(&mut self) -> TreeUpdate {
        let update = self.nodes.finalize();
        let live_node_ids = update
            .nodes
            .iter()
            .map(|(node_id, _)| *node_id)
            .collect::<FxHashSet<_>>();

        self.node_ids
            .retain(|global_id, _| self.visited_global_ids.contains(global_id));
        self.focus_ids
            .retain(|node_id, _| live_node_ids.contains(node_id));
        self.node_bounds
            .retain(|node_id, _| live_node_ids.contains(node_id));
        self.action_listeners
            .retain(|node_id, _| live_node_ids.contains(node_id));

        update
    }

    pub(crate) fn prepaint_snapshot(&self) -> A11yPrepaintSnapshot {
        A11yPrepaintSnapshot {
            nodes: self.nodes.prepaint_snapshot(),
            node_ids: self.node_ids.clone(),
            visited_global_ids: self.visited_global_ids.clone(),
            next_node_id: self.next_node_id,
            focus_ids: self.focus_ids.clone(),
            node_bounds: self.node_bounds.clone(),
        }
    }

    pub(crate) fn restore_prepaint_snapshot(&mut self, snapshot: A11yPrepaintSnapshot) {
        self.nodes.restore_prepaint_snapshot(snapshot.nodes);
        self.node_ids = snapshot.node_ids;
        self.visited_global_ids = snapshot.visited_global_ids;
        self.next_node_id = snapshot.next_node_id;
        self.focus_ids = snapshot.focus_ids;
        self.node_bounds = snapshot.node_bounds;
        // `action_listeners` are populated during paint, while prepaint transactions only
        // roll back prepaint side effects.
    }
}

/// Builder API for accessibility nodes that do not correspond to GPUI elements.
pub struct A11ySubtreeBuilder<'a> {
    parent_id: NodeId,
    nodes: &'a mut A11yNodeBuilder,
}

impl<'a> A11ySubtreeBuilder<'a> {
    pub(crate) fn new(parent_id: NodeId, nodes: &'a mut A11yNodeBuilder) -> Self {
        Self { parent_id, nodes }
    }

    /// Derive a stable synthetic child ID from this parent and a caller-provided key.
    pub fn synthetic_node_id(&self, key: impl Hash) -> NodeId {
        let mut hasher = std::hash::DefaultHasher::default();
        self.parent_id.0.hash(&mut hasher);
        key.hash(&mut hasher);
        NodeId(hasher.finish())
    }

    /// Append a synthetic leaf node as a child of this element's accessibility node.
    pub fn push_child(&mut self, id: NodeId, node: accesskit::Node) -> bool {
        self.nodes.push_leaf(id, node)
    }

    /// Mutate the accessibility node belonging to the parent element.
    pub fn parent_node(&mut self) -> &mut accesskit::Node {
        self.nodes
            .current_node_mut()
            .expect("A11ySubtreeBuilder exists only while its element node is current")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ElementId, point, px, size};
    use slotmap::KeyData;
    use std::sync::{Arc, atomic::AtomicBool};

    fn global_id(id: impl Into<ElementId>) -> GlobalElementId {
        GlobalElementId(Arc::from([id.into()]))
    }

    fn has_update_node(update: &TreeUpdate, id: NodeId) -> bool {
        update.nodes.iter().any(|(node_id, _)| *node_id == id)
    }

    #[test]
    fn restores_prepaint_snapshot_for_a11y_maps() {
        let mut a11y = A11y::new(Arc::new(AtomicBool::new(false)), false, None);
        a11y.begin_frame();

        let focus_id = FocusId::from(KeyData::from_ffi(1));
        let original_bounds = Bounds::new(point(px(1.), px(2.)), size(px(3.), px(4.)));
        a11y.focus_ids.insert(NodeId(1), focus_id);
        a11y.node_bounds.insert(NodeId(1), original_bounds);
        let snapshot = a11y.prepaint_snapshot();

        a11y.focus_ids.remove(&NodeId(1));
        a11y.focus_ids
            .insert(NodeId(2), FocusId::from(KeyData::from_ffi(2)));
        a11y.node_bounds.insert(
            NodeId(1),
            Bounds::new(point(px(5.), px(6.)), size(px(7.), px(8.))),
        );
        a11y.node_bounds.insert(
            NodeId(2),
            Bounds::new(point(px(9.), px(10.)), size(px(11.), px(12.))),
        );

        a11y.restore_prepaint_snapshot(snapshot);

        assert_eq!(a11y.focus_ids.len(), 1);
        assert_eq!(a11y.focus_ids.get(&NodeId(1)), Some(&focus_id));
        assert_eq!(a11y.node_bounds.len(), 1);
        assert_eq!(a11y.node_bounds.get(&NodeId(1)), Some(&original_bounds));
    }

    #[test]
    fn allocates_stable_unique_node_ids_for_global_element_ids() {
        let mut a11y = A11y::new(Arc::new(AtomicBool::new(false)), false, None);
        let first_id = global_id("first");
        let second_id = global_id("second");

        let first_node_id = a11y.node_id_for(&first_id);
        let second_node_id = a11y.node_id_for(&second_id);
        a11y.begin_frame();

        assert_ne!(first_node_id, ROOT_NODE_ID);
        assert_ne!(second_node_id, ROOT_NODE_ID);
        assert_ne!(first_node_id, second_node_id);
        assert_eq!(a11y.node_id_for(&first_id), first_node_id);
        assert_eq!(a11y.node_id_for_existing(&second_id), Some(second_node_id));
        assert_eq!(a11y.node_id_for_existing(&global_id("missing")), None);
    }

    #[test]
    fn retains_visited_suppressed_node_id_and_sweeps_omitted_node_id() {
        let mut a11y = A11y::new(Arc::new(AtomicBool::new(false)), false, None);
        let hidden_id = global_id("hidden");

        a11y.begin_frame();
        let hidden_node_id = a11y.node_id_for(&hidden_id);
        assert!(a11y.nodes.push(
            hidden_node_id,
            accesskit::Node::new(accesskit::Role::Button)
        ));
        assert!(a11y.nodes.suppress_current_node(hidden_node_id));
        a11y.nodes.pop();
        let update = a11y.end_frame();

        assert!(!has_update_node(&update, hidden_node_id));
        assert_eq!(a11y.node_id_for_existing(&hidden_id), Some(hidden_node_id));

        a11y.begin_frame();
        a11y.end_frame();

        assert_eq!(a11y.node_id_for_existing(&hidden_id), None);
    }

    #[test]
    fn sweeps_per_node_maps_by_live_emitted_nodes_after_end_frame() {
        let mut a11y = A11y::new(Arc::new(AtomicBool::new(false)), false, None);
        let live_bounds = Bounds::new(point(px(1.), px(2.)), size(px(3.), px(4.)));
        let stale_bounds = Bounds::new(point(px(5.), px(6.)), size(px(7.), px(8.)));
        let live_focus = FocusId::from(KeyData::from_ffi(1));
        let stale_focus = FocusId::from(KeyData::from_ffi(2));

        a11y.begin_frame();
        assert!(
            a11y.nodes
                .push(NodeId(1), accesskit::Node::new(accesskit::Role::Button))
        );
        a11y.nodes.pop();
        a11y.focus_ids.insert(NodeId(1), live_focus);
        a11y.focus_ids.insert(NodeId(2), stale_focus);
        a11y.node_bounds.insert(NodeId(1), live_bounds);
        a11y.node_bounds.insert(NodeId(2), stale_bounds);
        a11y.action_listeners
            .insert(NodeId(1), vec![(Action::Click, Box::new(|_, _, _| {}))]);
        a11y.action_listeners
            .insert(NodeId(2), vec![(Action::Focus, Box::new(|_, _, _| {}))]);

        let update = a11y.end_frame();

        assert!(has_update_node(&update, NodeId(1)));
        assert!(!has_update_node(&update, NodeId(2)));
        assert_eq!(a11y.focus_ids.len(), 1);
        assert_eq!(a11y.focus_ids.get(&NodeId(1)), Some(&live_focus));
        assert_eq!(a11y.node_bounds.len(), 1);
        assert_eq!(a11y.node_bounds.get(&NodeId(1)), Some(&live_bounds));
        assert_eq!(a11y.action_listeners.len(), 1);
        assert!(a11y.action_listeners.contains_key(&NodeId(1)));
    }

    #[test]
    fn restores_prepaint_snapshot_for_node_id_allocator() {
        let mut a11y = A11y::new(Arc::new(AtomicBool::new(false)), false, None);
        let accepted_id = global_id("accepted");
        let rejected_id = global_id("rejected");
        let next_id = global_id("next");

        let accepted_node_id = a11y.node_id_for(&accepted_id);
        let snapshot = a11y.prepaint_snapshot();
        let rejected_node_id = a11y.node_id_for(&rejected_id);

        a11y.restore_prepaint_snapshot(snapshot);
        let next_node_id = a11y.node_id_for(&next_id);

        assert_eq!(
            a11y.node_id_for_existing(&accepted_id),
            Some(accepted_node_id)
        );
        assert_eq!(a11y.node_id_for_existing(&rejected_id), None);
        assert_eq!(next_node_id, rejected_node_id);
    }

    #[test]
    fn restores_prepaint_snapshot_for_visited_globals() {
        let mut a11y = A11y::new(Arc::new(AtomicBool::new(false)), false, None);
        let accepted_id = global_id("accepted");
        let rejected_id = global_id("rejected");

        a11y.begin_frame();
        let accepted_node_id = a11y.node_id_for(&accepted_id);
        let snapshot = a11y.prepaint_snapshot();
        let rejected_node_id = a11y.node_id_for(&rejected_id);
        assert!(a11y.nodes.push(
            accepted_node_id,
            accesskit::Node::new(accesskit::Role::Button)
        ));
        a11y.nodes.pop();

        a11y.restore_prepaint_snapshot(snapshot);
        let update = a11y.end_frame();

        assert!(!has_update_node(&update, accepted_node_id));
        assert!(!has_update_node(&update, rejected_node_id));
        assert_eq!(
            a11y.node_id_for_existing(&accepted_id),
            Some(accepted_node_id)
        );
        assert_eq!(a11y.node_id_for_existing(&rejected_id), None);
    }

    #[test]
    fn window_title_labels_initial_and_updated_root() {
        let mut a11y = A11y::new(
            Arc::new(AtomicBool::new(false)),
            false,
            Some("Initial".into()),
        );
        a11y.begin_frame();
        let initial = a11y.end_frame();
        assert_eq!(
            initial
                .nodes
                .iter()
                .find(|(id, _)| *id == ROOT_NODE_ID)
                .unwrap()
                .1
                .label(),
            Some("Initial")
        );

        a11y.set_window_title("Updated");
        a11y.begin_frame();
        let updated = a11y.end_frame();
        assert_eq!(
            updated
                .nodes
                .iter()
                .find(|(id, _)| *id == ROOT_NODE_ID)
                .unwrap()
                .1
                .label(),
            Some("Updated")
        );
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "was not registered with set_focusable")
    )]
    fn set_focus_requires_focusable_registration() {
        let mut a11y = A11y::new(Arc::new(AtomicBool::new(true)), false, None);
        a11y.begin_frame();
        assert!(
            a11y.nodes
                .push(NodeId(1), accesskit::Node::new(accesskit::Role::Button))
        );
        a11y.set_focus(NodeId(1));
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "on the focused node"))]
    fn active_descendant_cannot_be_focused_node() {
        let mut a11y = A11y::new(Arc::new(AtomicBool::new(true)), false, None);
        a11y.begin_frame();
        let focus_id = FocusId::from(KeyData::from_ffi(1));
        assert!(
            a11y.nodes
                .push(NodeId(1), accesskit::Node::new(accesskit::Role::ListBox))
        );
        a11y.set_focusable(NodeId(1), focus_id);
        a11y.set_focus(NodeId(1));
        a11y.set_active_descendant(NodeId(1));
    }

    #[test]
    fn active_descendant_in_unfocused_subtree_keeps_real_focus() {
        let mut a11y = A11y::new(Arc::new(AtomicBool::new(true)), false, None);
        a11y.begin_frame();
        let focus_id = FocusId::from(KeyData::from_ffi(1));
        assert!(
            a11y.nodes
                .push(NodeId(1), accesskit::Node::new(accesskit::Role::Button))
        );
        a11y.set_focusable(NodeId(1), focus_id);
        a11y.set_focus(NodeId(1));
        a11y.nodes.pop();
        assert!(
            a11y.nodes
                .push(NodeId(2), accesskit::Node::new(accesskit::Role::ListBox))
        );
        assert!(a11y.nodes.push(
            NodeId(3),
            accesskit::Node::new(accesskit::Role::ListBoxOption)
        ));
        a11y.set_active_descendant(NodeId(3));
        a11y.nodes.pop();
        a11y.nodes.pop();

        assert_eq!(a11y.end_frame().focus, NodeId(1));
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "active descendant claimed by multiple nodes")
    )]
    fn sibling_active_descendant_claims_fail_in_debug() {
        let mut a11y = A11y::new(Arc::new(AtomicBool::new(true)), false, None);
        a11y.begin_frame();
        assert!(
            a11y.nodes
                .push(NodeId(1), accesskit::Node::new(accesskit::Role::ListBox))
        );
        a11y.set_focusable(NodeId(1), FocusId::from(KeyData::from_ffi(1)));
        a11y.set_focus(NodeId(1));

        assert!(a11y.nodes.push(
            NodeId(2),
            accesskit::Node::new(accesskit::Role::ListBoxOption)
        ));
        a11y.set_active_descendant(NodeId(2));
        a11y.nodes.pop();
        assert!(a11y.nodes.push(
            NodeId(3),
            accesskit::Node::new(accesskit::Role::ListBoxOption)
        ));
        a11y.set_active_descendant(NodeId(3));
    }

    #[test]
    fn synthetic_ids_are_stable_per_parent_and_key() {
        let mut nodes = A11yNodeBuilder::new();
        nodes.begin_frame(None);
        assert!(nodes.push(NodeId(1), accesskit::Node::new(accesskit::Role::TextInput)));
        let first = A11ySubtreeBuilder::new(NodeId(1), &mut nodes).synthetic_node_id("run");
        let same = A11ySubtreeBuilder::new(NodeId(1), &mut nodes).synthetic_node_id("run");
        let other = A11ySubtreeBuilder::new(NodeId(2), &mut nodes).synthetic_node_id("run");
        assert_eq!(first, same);
        assert_ne!(first, other);
    }
}
