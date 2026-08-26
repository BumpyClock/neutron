use gpui::{App, KeyBinding, SharedString};

use crate::{
    IndexPath, Size,
    actions::{Cancel, Confirm, SelectDown, SelectUp},
    searchable_list::{SearchableListChange, SearchableListDelegate},
};

mod component;
mod render;
mod state;

pub use component::Combobox;
pub use render::Caret;
pub use state::{ComboboxEvent, ComboboxState};

const CONTEXT: &str = "Combobox";

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("up", SelectUp, Some(CONTEXT)),
        KeyBinding::new("down", SelectDown, Some(CONTEXT)),
        KeyBinding::new("enter", Confirm { secondary: false }, Some(CONTEXT)),
        KeyBinding::new(
            "secondary-enter",
            Confirm { secondary: true },
            Some(CONTEXT),
        ),
        KeyBinding::new("escape", Cancel, Some(CONTEXT)),
    ])
}

/// Context passed to the `render_trigger` closure on [`Combobox`].
pub struct ComboboxTriggerCtx<'a, D: SearchableListDelegate + 'static> {
    pub selection: &'a [(IndexPath, D::Item)],
    pub placeholder: Option<&'a SharedString>,
    pub open: bool,
    pub disabled: bool,
    pub size: Size,
}

/// Back-compat alias — new code should use [`SearchableListChange`] directly.
pub type ComboboxChange = SearchableListChange;
