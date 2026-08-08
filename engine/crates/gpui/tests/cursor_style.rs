use gpui::{CursorStyle, Styled, div};

#[test]
fn cursor_none_helper_sets_none_cursor_style() {
    let mut element = div().cursor_none(CursorStyle::Arrow);

    assert_eq!(element.style().mouse_cursor, Some(CursorStyle::None));
}

#[test]
fn cursor_style_accepts_none() {
    let mut element = div().cursor(CursorStyle::None);

    assert_eq!(element.style().mouse_cursor, Some(CursorStyle::None));
}
