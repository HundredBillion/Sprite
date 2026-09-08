//! Draws a Surface Description: one GPUI element per described element,
//! built fresh on every frame the way GPUI's own views are. An `update` that
//! replaces the description therefore replaces the drawing, and nothing from
//! the previous one survives — there are no element ids to patch and none to
//! leak.

use std::sync::Arc;

use gpui::{
    AnyElement, ElementId, Image, ImageFormat, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, div, img, rgb,
};

use crate::grid_paint::pack;
use crate::surface::SurfaceId;
use crate::surface::channel::{SurfaceConnection, event_click};
use crate::surface::description::{Description, Element, Kind};
use crate::surface::style;
use crate::tokens::{Role, TokenRegistry};

pub(crate) fn render(
    description: &Description,
    surface: SurfaceId,
    registry: &TokenRegistry,
    connection: &Arc<SurfaceConnection>,
) -> AnyElement {
    let mut next = 0u64;
    element(&description.root, surface, registry, connection, &mut next)
}

fn element(
    node: &Element,
    surface: SurfaceId,
    registry: &TokenRegistry,
    connection: &Arc<SurfaceConnection>,
    next: &mut u64,
) -> AnyElement {
    // Numbered in tree order, so a clickable element's identity is stable for
    // as long as the description keeps its shape.
    let index = *next;
    *next += 1;

    if node.kind == Kind::Image {
        let bytes = node.svg.clone().unwrap_or_default().into_bytes();
        let picture = img(Arc::new(Image::from_bytes(ImageFormat::Svg, bytes)));
        return style::apply_all(picture, &node.style).into_any_element();
    }

    let mut boxed = div();
    if node.kind == Kind::List {
        boxed = boxed.flex().flex_col();
    }
    if node.kind == Kind::Button {
        boxed = boxed.cursor_pointer();
    }
    boxed = style::apply_all(boxed, &node.style);
    if let Some(color) = &node.color {
        boxed = boxed.text_color(rgb(pack(color.resolve(registry, Role::Text))));
    }
    if let Some(background) = &node.background {
        boxed = boxed.bg(rgb(pack(background.resolve(registry, Role::Fill))));
    }
    if let Some(border) = &node.border {
        boxed = boxed.border_color(rgb(pack(border.resolve(registry, Role::Fill))));
    }
    if let Some(text) = &node.text {
        boxed = boxed.child(SharedString::from(text.clone()));
    }
    let children: Vec<AnyElement> = node
        .children
        .iter()
        .map(|child| element(child, surface, registry, connection, next))
        .collect();
    boxed = boxed.children(children);

    match &node.on_click {
        None => boxed.into_any_element(),
        Some(name) => {
            let name = name.clone();
            let connection = Arc::clone(connection);
            boxed
                .id(ElementId::NamedInteger(
                    SharedString::from(format!("surface-{}", surface.0)),
                    index,
                ))
                .on_click(move |_event, _window, _cx| {
                    connection.send(&event_click(&name));
                })
                .into_any_element()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;

    use serde_json::json;

    use crate::config::Colors;
    use crate::surface::description;

    #[test]
    fn every_kind_becomes_an_element_without_a_window() {
        let (ours, _theirs) = UnixStream::pair().expect("socket pair");
        let connection = Arc::new(SurfaceConnection::new(&ours).expect("connection"));
        let registry = TokenRegistry::new(&Colors::default());
        let parsed = description::parse(
            &json!({
                "version": 1,
                "root": { "kind": "box", "style": "flex flex_col gap_2 p_3", "bg": "terminal.background",
                    "children": [
                        { "kind": "text", "text": "Files", "style": "text_sm font_bold", "color": "#c0caf5" },
                        { "kind": "list", "children": [
                            { "kind": "box", "style": "flex flex_row items_center gap_2", "on_click": "row-1", "children": [
                                { "kind": "image", "style": "w_4 h_4",
                                  "svg": "<svg xmlns='http://www.w3.org/2000/svg' width='16' height='16'><circle cx='8' cy='8' r='6' fill='#7aa2f7'/></svg>" },
                                { "kind": "text", "text": "src", "color": "ansi.4" }
                            ] }
                        ] },
                        { "kind": "button", "text": "Refresh", "on_click": "refresh", "border": "ansi.4", "style": "border_1 rounded_sm px_2" }
                    ] }
            }),
            &registry,
        )
        .expect("a valid description");

        // Building the element tree needs no window; that is the property
        // this test locks down, since every frame rebuilds it.
        let _element = render(&parsed.description, SurfaceId(1), &registry, &connection);
    }
}
