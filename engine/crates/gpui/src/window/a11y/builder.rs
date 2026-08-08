use super::ROOT_NODE_ID;
use crate::{Bounds, FocusId, GlobalElementId, Pixels, SharedString};
use accesskit::{NodeId, TreeUpdate};
use collections::{FxHashMap, FxHashSet};
use smallvec::SmallVec;

#[derive(Clone)]
pub(crate) struct A11yNodeBuilder {
    ids_stack: SmallVec<[NodeId; 16]>,
    nodes_stack: SmallVec<[accesskit::Node; 16]>,
    suppression_stack: SmallVec<[bool; 16]>,
    ambient_suppression_depth: usize,
    all_nodes: Vec<(NodeId, accesskit::Node)>,
    emitted_node_indices: FxHashMap<NodeId, usize>,
    seen_ids: FxHashSet<NodeId>,
    focus: Option<NodeId>,
    active_descendant: Option<NodeId>,
}

pub(crate) struct A11yPrepaintSnapshot {
    pub(super) nodes: A11yNodeBuilder,
    pub(super) node_ids: FxHashMap<GlobalElementId, NodeId>,
    pub(super) visited_global_ids: FxHashSet<GlobalElementId>,
    pub(super) next_node_id: u64,
    pub(super) focus_ids: FxHashMap<NodeId, FocusId>,
    pub(super) node_bounds: FxHashMap<NodeId, Bounds<Pixels>>,
}

impl A11yNodeBuilder {
    pub(super) fn new() -> Self {
        Self {
            ids_stack: SmallVec::new(),
            nodes_stack: SmallVec::new(),
            suppression_stack: SmallVec::new(),
            ambient_suppression_depth: 0,
            all_nodes: Vec::new(),
            emitted_node_indices: FxHashMap::default(),
            seen_ids: FxHashSet::default(),
            focus: None,
            active_descendant: None,
        }
    }

    fn can_push(&mut self, id: NodeId) -> bool {
        debug_assert!(!self.ids_stack.is_empty(), "push called before begin_frame");

        if self.is_suppressed() {
            return false;
        }

        if !self.seen_ids.insert(id) {
            debug_assert!(
                false,
                "duplicate a11y node id: {id:?}; release builds discard this node"
            );
            return false;
        }

        true
    }

    pub(crate) fn push(&mut self, id: NodeId, node: accesskit::Node) -> bool {
        if !self.can_push(id) {
            return false;
        }

        self.ids_stack.push(id);
        self.nodes_stack.push(node);
        self.suppression_stack.push(false);
        true
    }

    pub(crate) fn push_leaf(&mut self, id: NodeId, node: accesskit::Node) -> bool {
        if !self.can_push(id) {
            return false;
        }

        let Some(parent) = self.nodes_stack.last_mut() else {
            return false;
        };
        parent.push_child(id);
        self.emitted_node_indices.insert(id, self.all_nodes.len());
        self.all_nodes.push((id, node));
        true
    }

    pub(crate) fn pop(&mut self) {
        debug_assert!(self.ids_stack.len() > 1, "pop would remove the root node");

        self.pop_any();
    }

    pub(super) fn begin_frame(&mut self, window_title: Option<&SharedString>) {
        self.all_nodes.clear();
        self.emitted_node_indices.clear();
        self.ids_stack.clear();
        self.nodes_stack.clear();
        self.suppression_stack.clear();
        self.ambient_suppression_depth = 0;
        self.seen_ids.clear();
        self.seen_ids.insert(ROOT_NODE_ID);
        self.ids_stack.push(ROOT_NODE_ID);
        let mut root = accesskit::Node::new(accesskit::Role::Window);
        if let Some(title) = window_title {
            root.set_label(title.to_string());
        }
        self.nodes_stack.push(root);
        self.suppression_stack.push(false);
        self.focus = None;
        self.active_descendant = None;
    }

    pub(crate) fn has_node(&self, id: NodeId) -> bool {
        id == ROOT_NODE_ID || self.seen_ids.contains(&id)
    }

    pub(crate) fn has_current_node(&self, id: NodeId) -> bool {
        self.ids_stack.last().copied() == Some(id) && !self.is_suppressed()
    }

    pub(crate) fn node_is_focused(&self, id: NodeId) -> bool {
        self.focus == Some(id)
    }

    pub(crate) fn focus_is_ancestor_of_current(&self) -> bool {
        let Some(focus) = self.focus else {
            return false;
        };

        let ancestor_count = self.ids_stack.len().saturating_sub(1);
        self.ids_stack[..ancestor_count].contains(&focus)
    }

    pub(crate) fn set_active_descendant(&mut self, id: NodeId) {
        if self
            .active_descendant
            .is_some_and(|existing| existing != id)
        {
            if cfg!(debug_assertions) {
                panic!("active descendant claimed by multiple nodes in one frame");
            } else {
                log::warn!(
                    "a11y: multiple nodes claimed the active descendant this frame; using last-wins ({id:?})"
                );
            }
        }
        self.active_descendant = Some(id);
    }

    pub(crate) fn set_focus(&mut self, id: NodeId) {
        if self.focus.is_some() {
            if cfg!(debug_assertions) {
                panic!("set_focus called more than once in a single frame");
            } else {
                log::warn!(
                    "a11y: set_focus called more than once in a single frame; using last-wins ({id:?})"
                );
            }
        }
        self.focus = Some(id);
    }

    pub(super) fn prepaint_snapshot(&self) -> A11yNodeBuilder {
        self.clone()
    }

    pub(super) fn restore_prepaint_snapshot(&mut self, snapshot: A11yNodeBuilder) {
        *self = snapshot;
    }

    pub(crate) fn update_current_node_bounds(
        &mut self,
        id: NodeId,
        bounds: Bounds<Pixels>,
        scale_factor: f32,
    ) -> bool {
        let Some(node) = self.current_node_mut_for_id(id) else {
            return false;
        };

        node.set_bounds(accesskit::Rect {
            x0: (bounds.origin.x.0 * scale_factor) as f64,
            y0: (bounds.origin.y.0 * scale_factor) as f64,
            x1: ((bounds.origin.x.0 + bounds.size.width.0) * scale_factor) as f64,
            y1: ((bounds.origin.y.0 + bounds.size.height.0) * scale_factor) as f64,
        });
        true
    }

    pub(crate) fn suppress_current_node(&mut self, id: NodeId) -> bool {
        if self.ids_stack.len() <= 1 {
            debug_assert!(false, "cannot suppress the root a11y node");
            return false;
        }

        if self.ids_stack.last().copied() != Some(id) {
            return false;
        }

        let Some(suppressed) = self.suppression_stack.last_mut() else {
            return false;
        };

        if *suppressed {
            return false;
        }

        *suppressed = true;
        self.prune_emitted_subtree(id);
        true
    }

    pub(crate) fn begin_suppressing_descendants(&mut self) {
        self.ambient_suppression_depth += 1;
    }

    pub(crate) fn end_suppressing_descendants(&mut self) {
        debug_assert!(
            self.ambient_suppression_depth > 0,
            "end_suppressing_descendants called without matching begin"
        );
        self.ambient_suppression_depth = self.ambient_suppression_depth.saturating_sub(1);
    }

    pub(super) fn finalize(&mut self) -> TreeUpdate {
        debug_assert_eq!(self.ids_stack.len(), 1);
        debug_assert_eq!(self.ids_stack[0], ROOT_NODE_ID);
        debug_assert_eq!(self.ambient_suppression_depth, 0);

        if self.ids_stack.len() != 1 {
            log::error!(
                "a11y: stack imbalance at end of frame: expected 1 (root), got {}",
                self.ids_stack.len()
            );
        }
        if self.ambient_suppression_depth != 0 {
            log::error!(
                "a11y: ambient suppression imbalance at end of frame: got {}",
                self.ambient_suppression_depth
            );
            self.ambient_suppression_depth = 0;
        }

        while !self.ids_stack.is_empty() {
            self.pop_any();
        }

        let focus = match self.active_descendant {
            Some(id) if self.has_node(id) => id,
            Some(id) => {
                if cfg!(debug_assertions) {
                    panic!("active_descendant set to {id:?}, which is not in the tree");
                } else {
                    log::warn!("active_descendant set to {id:?}, which is not in the tree");
                    self.focus.unwrap_or(ROOT_NODE_ID)
                }
            }
            None => self.focus.unwrap_or(ROOT_NODE_ID),
        };

        let nodes = std::mem::take(&mut self.all_nodes);
        self.emitted_node_indices.clear();

        let update = TreeUpdate {
            nodes,
            tree: Some(accesskit::Tree::new(ROOT_NODE_ID)),
            tree_id: accesskit::TreeId::ROOT,
            focus,
        };

        Self::repair_tree_update(update)
    }

    pub(crate) fn current_node_mut(&mut self) -> Option<&mut accesskit::Node> {
        if self.is_suppressed() {
            None
        } else {
            self.nodes_stack.last_mut()
        }
    }

    fn current_node_mut_for_id(&mut self, id: NodeId) -> Option<&mut accesskit::Node> {
        if self.ids_stack.len() <= 1
            || self.ids_stack.last().copied() != Some(id)
            || self.suppression_stack.last().copied().unwrap_or(true)
        {
            None
        } else {
            self.nodes_stack.last_mut()
        }
    }

    fn is_suppressed(&self) -> bool {
        self.ambient_suppression_depth > 0
            || self.suppression_stack.last().copied().unwrap_or_default()
    }

    fn prune_emitted_subtree(&mut self, id: NodeId) {
        let mut pruned_ids = FxHashSet::default();
        pruned_ids.insert(id);

        if let Some(current_node) = self.nodes_stack.last() {
            let mut pending = current_node.children().to_vec();
            if !pending.is_empty() {
                while let Some(child_id) = pending.pop() {
                    if !pruned_ids.insert(child_id) {
                        continue;
                    }

                    if let Some(index) = self.emitted_node_indices.get(&child_id).copied()
                        && let Some((_, child_node)) = self.all_nodes.get(index)
                    {
                        pending.extend(child_node.children().iter().copied());
                    }
                }
            }
        }

        for node_id in &pruned_ids {
            self.seen_ids.remove(node_id);
        }

        if self.focus.is_some_and(|focus| pruned_ids.contains(&focus)) {
            self.focus = None;
        }
        if self
            .active_descendant
            .is_some_and(|active| pruned_ids.contains(&active))
        {
            self.active_descendant = None;
        }

        self.all_nodes
            .retain(|(node_id, _)| !pruned_ids.contains(node_id));
        self.rebuild_emitted_node_indices();

        for (_, node) in &mut self.all_nodes {
            Self::remove_child_refs(node, &pruned_ids);
        }
        for node in &mut self.nodes_stack {
            Self::remove_child_refs(node, &pruned_ids);
        }
    }

    fn rebuild_emitted_node_indices(&mut self) {
        self.emitted_node_indices.clear();
        self.emitted_node_indices.extend(
            self.all_nodes
                .iter()
                .enumerate()
                .map(|(index, (node_id, _))| (*node_id, index)),
        );
    }

    fn remove_child_refs(node: &mut accesskit::Node, removed_ids: &FxHashSet<NodeId>) {
        if node
            .children()
            .iter()
            .any(|child_id| removed_ids.contains(child_id))
        {
            let children = node
                .children()
                .iter()
                .copied()
                .filter(|child_id| !removed_ids.contains(child_id))
                .collect::<Vec<_>>();
            node.set_children(children);
        }
    }

    fn pop_any(&mut self) {
        if let (Some(id), Some(node), Some(suppressed)) = (
            self.ids_stack.pop(),
            self.nodes_stack.pop(),
            self.suppression_stack.pop(),
        ) {
            if suppressed {
                return;
            }

            if let (Some(parent), Some(parent_suppressed)) =
                (self.nodes_stack.last_mut(), self.suppression_stack.last())
            {
                if !*parent_suppressed {
                    parent.push_child(id);
                }
            }
            self.emitted_node_indices.insert(id, self.all_nodes.len());
            self.all_nodes.push((id, node));
        }
    }

    fn repair_tree_update(mut update: TreeUpdate) -> TreeUpdate {
        let node_ids: FxHashSet<NodeId> = update.nodes.iter().map(|(id, _)| *id).collect();

        if !node_ids.contains(&update.focus) {
            log::error!(
                "a11y: focused node {:?} is not in the tree ({} nodes); falling back to root",
                update.focus,
                update.nodes.len()
            );
            update.focus = ROOT_NODE_ID;
        }

        for (id, node) in &mut update.nodes {
            let has_invalid_child = node
                .children()
                .iter()
                .any(|child_id| !node_ids.contains(child_id));
            if has_invalid_child {
                let children = node.children();
                let invalid_count = children
                    .iter()
                    .filter(|child_id| !node_ids.contains(child_id))
                    .count();
                log::error!(
                    "a11y: node {:?} references {} children not present in the tree; stripping invalid child references",
                    id,
                    invalid_count
                );
                let valid = children
                    .iter()
                    .copied()
                    .filter(|child_id| node_ids.contains(child_id))
                    .collect::<Vec<_>>();
                node.set_children(valid);
            }
        }

        update
    }
}

#[cfg(test)]
#[path = "builder_tests.rs"]
mod tests;
