use gpui::{
    App, Bounds, Element, ElementInputHandler, Entity, IntoElement, LayoutId, PaintQuad, Pixels,
    ShapedLine, SharedString, TextRun, Window, fill, hsla, point, px, relative, size,
};

use super::Editor;

// ---------------------------------------------------------------------------
// EditorText — the specialized renderer: shapes the text and paints the cursor.
// ---------------------------------------------------------------------------

pub(super) struct EditorText {
    pub(super) editor: Entity<Editor>,
}

pub(super) struct EditorTextPrepaint {
    lines: Vec<ShapedLine>,
    cursor: Option<PaintQuad>,
}

impl IntoElement for EditorText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for EditorText {
    type RequestLayoutState = ();
    type PrepaintState = EditorTextPrepaint;

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let editor = self.editor.read(cx);
        let content = editor.value.read(cx);
        let line_count = content.split('\n').count().max(1);
        let line_height = window.line_height();
        let mut style = gpui::Style::default();
        style.size.width = relative(1.).into();
        style.size.height = (line_height * line_count as f32).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let editor = self.editor.read(cx);
        let content = editor.value.read(cx).clone();
        let cursor_offset = editor.cursor;
        let cursor_visible = editor.cursor_visible;
        let is_focused = editor.focus_handle.is_focused(window);

        let style = window.text_style();
        let text_color = style.color;
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();

        let is_placeholder = content.is_empty();

        let lines: Vec<ShapedLine> = if is_placeholder {
            let placeholder: SharedString = "Type here...".into();
            let run = TextRun {
                len: placeholder.len(),
                font: style.font(),
                color: hsla(0., 0., 0.5, 0.5),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            vec![
                window
                    .text_system()
                    .shape_line(placeholder, font_size, &[run], None),
            ]
        } else {
            content
                .split('\n')
                .map(|line_str| {
                    let text: SharedString = SharedString::from(line_str.to_string());
                    let run = TextRun {
                        len: text.len(),
                        font: style.font(),
                        color: text_color,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    window
                        .text_system()
                        .shape_line(text, font_size, &[run], None)
                })
                .collect()
        };

        let cursor = if is_focused && cursor_visible && !is_placeholder {
            let (cursor_line, offset_in_line) = cursor_line_and_offset(&content, cursor_offset);
            let cursor_line = cursor_line.min(lines.len().saturating_sub(1));
            let cursor_x = lines[cursor_line].x_for_index(offset_in_line);
            Some(fill(
                Bounds::new(
                    point(
                        bounds.left() + cursor_x,
                        bounds.top() + line_height * cursor_line as f32,
                    ),
                    size(px(1.5), line_height),
                ),
                text_color,
            ))
        } else if is_focused && cursor_visible && is_placeholder {
            Some(fill(
                Bounds::new(
                    point(bounds.left(), bounds.top()),
                    size(px(1.5), line_height),
                ),
                text_color,
            ))
        } else {
            None
        };

        EditorTextPrepaint { lines, cursor }
    }

    fn paint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.editor.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.editor.clone()),
            cx,
        );

        let line_height = window.line_height();
        for (i, line) in prepaint.lines.iter().enumerate() {
            let origin = point(bounds.left(), bounds.top() + line_height * i as f32);
            line.paint(origin, line_height, gpui::TextAlign::Left, None, window, cx)
                .unwrap();
        }

        if let Some(cursor) = prepaint.cursor.take() {
            window.paint_quad(cursor);
        }
    }
}

fn cursor_line_and_offset(content: &str, cursor: usize) -> (usize, usize) {
    let cursor = cursor.min(content.len());
    let mut line_index = 0;
    let mut line_start = 0;
    for (i, ch) in content.char_indices() {
        if i >= cursor {
            break;
        }
        if ch == '\n' {
            line_index += 1;
            line_start = i + 1;
        }
    }
    (line_index, cursor - line_start)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_offset_past_content_clamps_to_the_last_line_end() {
        assert_eq!(cursor_line_and_offset("a\nβ", usize::MAX), (1, "β".len()));
    }
}
