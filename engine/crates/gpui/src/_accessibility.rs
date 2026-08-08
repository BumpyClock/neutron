//! # Accessibility in GPUI
//!
//! "Accessibility" refers to the ability of your application to be used by all
//! users, regardless of disability status. There are many aspects, all important, including:
//! - Ensuring sufficient text contrast.
//! - Providing a mechanism to disable animations.
//! - Providing a mechanism to increase text sizes.
//! - etc.
//!
//! This guide is focused on **programmatic accessibility**. This allows
//! assistive technology, such as screen readers or Braille displays, to inspect
//! and interact with your app.
//!
//! GPUI integrates with [AccessKit] to provide programmatic accessibility
//! features (referred to as simply "accessibility" for the rest of this guide).
//!
//! A minimal example can be found in `examples/a11y.rs`.
//!
//! ## Background
//!
//! Accessibility support is based on two key capabilities:
//! - Exposing information about the current UI state to assistive technology.
//! - Responding to actions requested by assistive technology.
//!
//! ### IDs in GPUI - [`ElementId`] and [`GlobalElementId`]
//!
//! In GPUI, each [`Element`] can have an [`id`][Element::id]. [`Element`]s with
//! IDs are also assigned a [`GlobalElementId`], formed by composing all
//! non-`None` IDs of its ancestors. These IDs should be unique per frame.
//!
//! ### IDs and accessibility
//!
//! When GPUI renders a frame, it walks your UI tree and informs assistive
//! technology about nodes with global IDs and non-`None`
//! [`role`][Element::a11y_role] values. Use
//! [`div().id(...).role()`][StatefulInteractiveElement::role] to set a role.
//!
//! Nodes with the same global ID across frames are considered to be the same
//! node. If a node's ID changes, assistive technology treats it as one node
//! being removed and another being added, which can be disorienting for users.
//!
//! #### IDs and text
//!
//! GPUI provides the [`text!`] macro, which wraps strings in the [`Text`] type
//! and automatically derives an ID from the source location of the macro
//! invocation. This means repeated calls through the same helper function can
//! produce duplicate text IDs. Set IDs explicitly with [`Text::with_id`] or wrap
//! each text element in a parent with a unique ID when rendering collections.
//!
//! Occasionally, you will need to create a [`Text`] element with no ID. You can
//! achieve this with [`Text::new_inaccessible`]. If you are creating a custom UI
//! component, this can avoid duplicating text that is already represented on a
//! parent node's accessible label.
//!
//! ### Handling actions
//!
//! Assistive technology can dispatch actions to a specific node. AccessKit
//! exposes [`accesskit::Action`], re-exported by GPUI as [`AccessibleAction`].
//! Respond to actions with
//! [`div().on_a11y_action()`][StatefulInteractiveElement::on_a11y_action].
//!
//! Some common actions are registered automatically. For example,
//! [`.on_click()`][StatefulInteractiveElement::on_click] adds an
//! [`AccessibleAction::Click`] handler that calls the click handler.
//!
//! ## Synthetic children
//!
//! A custom [`Element`] can represent accessibility nodes that do not each
//! correspond to a GPUI element by implementing
//! [`Element::a11y_synthetic_children`]. The callback receives an
//! [`A11ySubtreeBuilder`] after prepaint, so it may use prepaint state to append
//! synthetic leaf nodes or update the parent node. Use
//! [`A11ySubtreeBuilder::synthetic_node_id`] with keys that are unique among a
//! parent's synthetic children to derive IDs that remain stable across frames.
//!
//! ## Further reading
//!
//! - [AccessKit]: The cross-platform accessibility toolkit GPUI uses
//!   internally.
//! - [MDN WAI-ARIA basics][mdn-aria]: Introduction to roles, properties, and
//!   states.
//! - [ARIA Authoring Practices Guide][apg]: W3C patterns for accessible
//!   widgets.
//!
//! [AccessKit]: https://accesskit.dev/
//! [mdn-aria]: https://developer.mozilla.org/en-US/docs/Learn_web_development/Core/Accessibility/WAI-ARIA_basics
//! [apg]: https://www.w3.org/WAI/ARIA/apg/

#[cfg(doc)]
use crate::*;
