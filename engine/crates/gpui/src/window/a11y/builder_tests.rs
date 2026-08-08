use super::*;
use crate::{point, px, size};

fn node(update: &TreeUpdate, id: NodeId) -> &accesskit::Node {
    update
        .nodes
        .iter()
        .find_map(|(node_id, node)| (*node_id == id).then_some(node))
        .unwrap()
}

fn has_update_node(update: &TreeUpdate, id: NodeId) -> bool {
    update.nodes.iter().any(|(node_id, _)| *node_id == id)
}

#[test]
fn preserves_unsuppressed_tree_shape() {
    let mut builder = A11yNodeBuilder::new();
    builder.begin_frame(None);

    assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::Button)));
    assert!(builder.push(NodeId(2), accesskit::Node::new(accesskit::Role::Label)));
    builder.pop();
    builder.pop();

    let update = builder.finalize();
    assert_eq!(node(&update, ROOT_NODE_ID).children(), &[NodeId(1)]);
    assert_eq!(node(&update, NodeId(1)).children(), &[NodeId(2)]);
    assert_eq!(node(&update, NodeId(2)).children(), &[]);
}

#[test]
fn updates_current_non_root_node_bounds() {
    let mut builder = A11yNodeBuilder::new();
    builder.begin_frame(None);

    assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::Button)));
    assert!(builder.update_current_node_bounds(
        NodeId(1),
        Bounds::new(point(px(2.), px(3.)), size(px(4.), px(5.))),
        2.,
    ));
    builder.pop();

    let update = builder.finalize();
    assert_eq!(
        node(&update, NodeId(1)).bounds(),
        Some(accesskit::Rect {
            x0: 4.,
            y0: 6.,
            x1: 12.,
            y1: 16.,
        })
    );
}

#[test]
fn suppresses_current_node_before_finalization() {
    let mut builder = A11yNodeBuilder::new();
    builder.begin_frame(None);

    assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::Button)));
    assert!(builder.suppress_current_node(NodeId(1)));
    assert!(!builder.has_node(NodeId(1)));
    builder.pop();

    let update = builder.finalize();
    assert_eq!(update.nodes.len(), 1);
    assert_eq!(node(&update, ROOT_NODE_ID).children(), &[]);
}

#[test]
fn skips_descendants_while_current_node_is_suppressed() {
    let mut builder = A11yNodeBuilder::new();
    builder.begin_frame(None);

    assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::Button)));
    assert!(builder.suppress_current_node(NodeId(1)));
    assert!(!builder.push(NodeId(2), accesskit::Node::new(accesskit::Role::Label)));
    builder.pop();

    assert!(builder.push(NodeId(2), accesskit::Node::new(accesskit::Role::Label)));
    builder.pop();

    let update = builder.finalize();
    assert_eq!(update.nodes.len(), 2);
    assert_eq!(node(&update, ROOT_NODE_ID).children(), &[NodeId(2)]);
    assert_eq!(node(&update, NodeId(2)).children(), &[]);
}

#[test]
fn ambient_descendant_suppression_under_root_skips_child_push_without_suppressing_root() {
    let mut builder = A11yNodeBuilder::new();
    builder.begin_frame(None);

    builder.begin_suppressing_descendants();
    assert!(!builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::Button)));
    builder.end_suppressing_descendants();

    assert!(builder.has_node(ROOT_NODE_ID));
    assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::Button)));
    builder.pop();

    let update = builder.finalize();
    assert_eq!(update.nodes.len(), 2);
    assert_eq!(node(&update, ROOT_NODE_ID).children(), &[NodeId(1)]);
}

#[test]
fn id_specific_update_requires_current_node() {
    let mut builder = A11yNodeBuilder::new();
    builder.begin_frame(None);

    assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::Button)));
    assert!(!builder.update_current_node_bounds(
        NodeId(2),
        Bounds::new(point(px(2.), px(3.)), size(px(4.), px(5.))),
        2.,
    ));
    builder.pop();

    let update = builder.finalize();
    assert_eq!(node(&update, NodeId(1)).bounds(), None);
}

#[test]
fn id_specific_suppress_requires_current_node() {
    let mut builder = A11yNodeBuilder::new();
    builder.begin_frame(None);

    assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::Button)));
    assert!(!builder.suppress_current_node(NodeId(2)));
    assert!(builder.has_node(NodeId(1)));
    assert!(builder.push(NodeId(2), accesskit::Node::new(accesskit::Role::Label)));
    builder.pop();
    builder.pop();

    let update = builder.finalize();
    assert_eq!(update.nodes.len(), 3);
    assert_eq!(node(&update, ROOT_NODE_ID).children(), &[NodeId(1)]);
    assert_eq!(node(&update, NodeId(1)).children(), &[NodeId(2)]);
}

#[test]
fn suppressing_focused_current_node_removes_focus_from_tree() {
    let mut builder = A11yNodeBuilder::new();
    builder.begin_frame(None);

    assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::Button)));
    builder.set_focus(NodeId(1));
    assert!(builder.suppress_current_node(NodeId(1)));
    builder.pop();

    let update = builder.finalize();
    assert_eq!(update.focus, ROOT_NODE_ID);
    assert_eq!(update.nodes.len(), 1);
}

#[test]
fn suppressing_current_node_prunes_already_emitted_descendants() {
    let mut builder = A11yNodeBuilder::new();
    builder.begin_frame(None);

    assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::Button)));
    assert!(builder.push(NodeId(2), accesskit::Node::new(accesskit::Role::Label)));
    assert!(builder.push(NodeId(3), accesskit::Node::new(accesskit::Role::TextInput)));
    builder.set_focus(NodeId(3));
    builder.pop();
    builder.pop();

    assert!(builder.suppress_current_node(NodeId(1)));
    assert!(!builder.has_node(NodeId(1)));
    assert!(!builder.has_node(NodeId(2)));
    assert!(!builder.has_node(NodeId(3)));
    builder.pop();

    let update = builder.finalize();
    assert_eq!(update.nodes.len(), 1);
    assert_eq!(update.focus, ROOT_NODE_ID);
    assert_eq!(node(&update, ROOT_NODE_ID).children(), &[]);
    assert!(!has_update_node(&update, NodeId(1)));
    assert!(!has_update_node(&update, NodeId(2)));
    assert!(!has_update_node(&update, NodeId(3)));
}

#[test]
fn parent_children_do_not_reference_suppressed_descendants() {
    let mut builder = A11yNodeBuilder::new();
    builder.begin_frame(None);

    assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::Button)));
    assert!(builder.push(NodeId(2), accesskit::Node::new(accesskit::Role::Label)));
    assert!(builder.suppress_current_node(NodeId(2)));
    builder.pop();

    assert!(builder.push(NodeId(3), accesskit::Node::new(accesskit::Role::Label)));
    builder.pop();
    builder.pop();

    let update = builder.finalize();
    assert_eq!(update.nodes.len(), 3);
    assert_eq!(node(&update, ROOT_NODE_ID).children(), &[NodeId(1)]);
    assert_eq!(node(&update, NodeId(1)).children(), &[NodeId(3)]);
}

#[test]
fn restores_prepaint_snapshot_for_builder_state() {
    let mut builder = A11yNodeBuilder::new();
    builder.begin_frame(None);

    assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::Button)));
    let snapshot = builder.prepaint_snapshot();

    assert!(builder.push(NodeId(2), accesskit::Node::new(accesskit::Role::Label)));
    builder.pop();
    builder.begin_suppressing_descendants();
    assert!(!builder.push(NodeId(3), accesskit::Node::new(accesskit::Role::Label)));
    builder.set_focus(NodeId(1));

    builder.restore_prepaint_snapshot(snapshot);

    assert!(builder.push(NodeId(2), accesskit::Node::new(accesskit::Role::Label)));
    builder.set_focus(NodeId(2));
    builder.pop();
    builder.pop();

    let update = builder.finalize();
    assert_eq!(update.nodes.len(), 3);
    assert_eq!(update.focus, NodeId(2));
    assert_eq!(node(&update, ROOT_NODE_ID).children(), &[NodeId(1)]);
    assert_eq!(node(&update, NodeId(1)).children(), &[NodeId(2)]);
}

#[test]
fn restored_snapshot_keeps_emitted_node_index_consistent() {
    let mut builder = A11yNodeBuilder::new();
    builder.begin_frame(None);

    assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::Button)));
    assert!(builder.push(NodeId(2), accesskit::Node::new(accesskit::Role::Label)));
    assert!(builder.push(NodeId(3), accesskit::Node::new(accesskit::Role::Label)));
    builder.pop();
    builder.pop();
    let snapshot = builder.prepaint_snapshot();

    assert!(builder.push(NodeId(4), accesskit::Node::new(accesskit::Role::Label)));
    builder.pop();
    builder.restore_prepaint_snapshot(snapshot);

    assert!(builder.suppress_current_node(NodeId(1)));
    builder.pop();

    let update = builder.finalize();
    assert_eq!(update.nodes.len(), 1);
    assert_eq!(node(&update, ROOT_NODE_ID).children(), &[]);
    assert!(!has_update_node(&update, NodeId(1)));
    assert!(!has_update_node(&update, NodeId(2)));
    assert!(!has_update_node(&update, NodeId(3)));
    assert!(!has_update_node(&update, NodeId(4)));
}

#[test]
fn labels_root_with_window_title() {
    let mut builder = A11yNodeBuilder::new();
    let title = SharedString::from("Inspector");
    builder.begin_frame(Some(&title));

    let update = builder.finalize();

    assert_eq!(node(&update, ROOT_NODE_ID).label(), Some("Inspector"));
}

#[test]
fn synthetic_leaf_is_inserted_under_current_parent() {
    let mut builder = A11yNodeBuilder::new();
    builder.begin_frame(None);
    assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::TextInput)));

    let mut child = accesskit::Node::new(accesskit::Role::TextRun);
    child.set_value("hello");
    assert!(builder.push_leaf(NodeId(2), child));
    builder.pop();

    let update = builder.finalize();
    assert_eq!(node(&update, NodeId(1)).children(), &[NodeId(2)]);
    assert_eq!(node(&update, NodeId(2)).value(), Some("hello"));
}

#[test]
#[cfg_attr(debug_assertions, should_panic(expected = "duplicate a11y node id"))]
fn duplicate_synthetic_leaf_is_rejected() {
    let mut builder = A11yNodeBuilder::new();
    builder.begin_frame(None);
    assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::TextInput)));
    assert!(builder.push_leaf(NodeId(2), accesskit::Node::new(accesskit::Role::TextRun)));
    assert!(!builder.push_leaf(NodeId(2), accesskit::Node::new(accesskit::Role::TextRun)));
}

#[test]
fn synthetic_leaf_is_rejected_while_parent_is_suppressed() {
    let mut builder = A11yNodeBuilder::new();
    builder.begin_frame(None);
    assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::TextInput)));
    assert!(builder.suppress_current_node(NodeId(1)));
    assert!(!builder.push_leaf(NodeId(2), accesskit::Node::new(accesskit::Role::TextRun)));
    builder.pop();

    let update = builder.finalize();
    assert!(!has_update_node(&update, NodeId(1)));
    assert!(!has_update_node(&update, NodeId(2)));
}

#[test]
fn snapshot_rollback_discards_synthetic_leaf_and_restores_parent() {
    let mut builder = A11yNodeBuilder::new();
    builder.begin_frame(None);
    assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::TextInput)));
    let snapshot = builder.prepaint_snapshot();

    assert!(builder.push_leaf(NodeId(2), accesskit::Node::new(accesskit::Role::TextRun)));
    builder.current_node_mut().unwrap().set_label("rejected");
    builder.restore_prepaint_snapshot(snapshot);
    builder.current_node_mut().unwrap().set_label("accepted");
    builder.pop();

    let update = builder.finalize();
    assert_eq!(node(&update, NodeId(1)).label(), Some("accepted"));
    assert_eq!(node(&update, NodeId(1)).children(), &[]);
    assert!(!has_update_node(&update, NodeId(2)));
}

#[test]
fn active_descendant_honored_when_container_focused() {
    let mut builder = A11yNodeBuilder::new();
    builder.begin_frame(None);
    assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::ListBox)));
    builder.set_focus(NodeId(1));
    assert!(builder.push(
        NodeId(2),
        accesskit::Node::new(accesskit::Role::ListBoxOption)
    ));
    assert!(builder.focus_is_ancestor_of_current());
    builder.set_active_descendant(NodeId(2));
    builder.pop();
    builder.pop();

    assert_eq!(builder.finalize().focus, NodeId(2));
}

#[test]
fn active_descendant_honored_for_deep_descendant() {
    let mut builder = A11yNodeBuilder::new();
    builder.begin_frame(None);
    assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::ListBox)));
    builder.set_focus(NodeId(1));
    assert!(builder.push(NodeId(2), accesskit::Node::new(accesskit::Role::Group)));
    assert!(builder.push(
        NodeId(3),
        accesskit::Node::new(accesskit::Role::ListBoxOption)
    ));
    assert!(builder.focus_is_ancestor_of_current());
    builder.set_active_descendant(NodeId(3));
    builder.pop();
    builder.pop();
    builder.pop();

    assert_eq!(builder.finalize().focus, NodeId(3));
}

#[test]
fn active_descendant_ignored_in_unfocused_subtree() {
    let mut builder = A11yNodeBuilder::new();
    builder.begin_frame(None);
    assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::Button)));
    builder.set_focus(NodeId(1));
    builder.pop();
    assert!(builder.push(NodeId(2), accesskit::Node::new(accesskit::Role::ListBox)));
    assert!(builder.push(
        NodeId(3),
        accesskit::Node::new(accesskit::Role::ListBoxOption)
    ));
    assert!(!builder.focus_is_ancestor_of_current());
    builder.pop();
    builder.pop();

    assert_eq!(builder.finalize().focus, NodeId(1));
}

#[test]
fn active_descendant_ignored_when_nothing_is_focused() {
    let mut builder = A11yNodeBuilder::new();
    builder.begin_frame(None);
    assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::ListBox)));
    assert!(builder.push(
        NodeId(2),
        accesskit::Node::new(accesskit::Role::ListBoxOption)
    ));
    assert!(!builder.focus_is_ancestor_of_current());
    builder.pop();
    builder.pop();

    assert_eq!(builder.finalize().focus, ROOT_NODE_ID);
}

#[test]
fn focus_ancestor_check_excludes_self() {
    let mut builder = A11yNodeBuilder::new();
    builder.begin_frame(None);
    assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::ListBox)));
    builder.set_focus(NodeId(1));
    assert!(!builder.focus_is_ancestor_of_current());
    assert!(builder.push(
        NodeId(2),
        accesskit::Node::new(accesskit::Role::ListBoxOption)
    ));
    assert!(builder.focus_is_ancestor_of_current());
}

#[test]
#[cfg_attr(
    debug_assertions,
    should_panic(expected = "active descendant claimed by multiple nodes")
)]
fn multiple_active_descendant_claims_fail_in_debug() {
    let mut builder = A11yNodeBuilder::new();
    builder.begin_frame(None);
    builder.set_active_descendant(NodeId(1));
    builder.set_active_descendant(NodeId(2));
}

#[test]
#[cfg_attr(
    debug_assertions,
    should_panic(expected = "set_focus called more than once")
)]
fn multiple_focus_claims_fail_in_debug() {
    let mut builder = A11yNodeBuilder::new();
    builder.begin_frame(None);
    builder.set_focus(NodeId(1));
    builder.set_focus(NodeId(2));
}
