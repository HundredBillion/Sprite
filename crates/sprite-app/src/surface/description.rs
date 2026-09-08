//! The Surface Description: the versioned document a program sends saying
//! what a Surface contains. Parsed once, when it arrives, into a tree Sprite
//! can draw on every frame without looking at JSON again.
//!
//! What is refused and what is merely warned about follows one rule: a
//! mistake that changes the *shape* of what is drawn (an unknown kind, an
//! unknown layout token, a missing required field) is refused, because
//! guessing a layout draws something the program did not mean; a mistake in
//! a *colour name* is warned about and drawn in the role's fallback, because
//! a typo should dim one label, never blank a plugin.

use serde_json::Value;
use sprite_term::Rgb;

use crate::config::Colors;
use crate::surface::Refusal;
use crate::surface::style;
use crate::tokens::{Role, TokenRegistry};

/// The description format this Sprite understands.
pub const VERSION: u64 = 1;
/// Enough for a file tree of a few hundred rows several times over; past it,
/// a program wants the grid widget, not an element tree.
pub const MAX_ELEMENTS: usize = 4096;
pub const MAX_DEPTH: usize = 32;

#[derive(Clone, Debug, PartialEq)]
pub struct Description {
    pub root: Element,
}

impl Description {
    /// The grid this description opens, when its root is one.
    pub fn grid(&self) -> Option<GridSize> {
        self.root.grid
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    Box,
    Text,
    List,
    Image,
    Button,
    Grid,
}

impl Kind {
    fn parse(name: &str) -> Option<Kind> {
        Some(match name {
            "box" => Kind::Box,
            "text" => Kind::Text,
            "list" => Kind::List,
            "image" => Kind::Image,
            "button" => Kind::Button,
            "grid" => Kind::Grid,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Box => "box",
            Kind::Text => "text",
            Kind::List => "list",
            Kind::Image => "image",
            Kind::Button => "button",
            Kind::Grid => "grid",
        }
    }
}

/// A colour as a description names it: by token, or literally as an escape
/// hatch the documentation discourages.
#[derive(Clone, Debug, PartialEq)]
pub enum ColorRef {
    Token(String),
    Literal(Rgb),
}

impl ColorRef {
    pub fn resolve(&self, registry: &TokenRegistry, role: Role) -> Rgb {
        match self {
            ColorRef::Token(name) => registry.resolve(name, role).color,
            ColorRef::Literal(color) => *color,
        }
    }
}

/// A grid's size in cells.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GridSize {
    pub cols: u16,
    pub rows: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub kind: Kind,
    /// Utility tokens, each already checked against the style table.
    pub style: Vec<String>,
    pub color: Option<ColorRef>,
    pub background: Option<ColorRef>,
    pub border: Option<ColorRef>,
    pub text: Option<String>,
    pub svg: Option<String>,
    /// The event name a click sends, when the element is clickable.
    pub on_click: Option<String>,
    pub children: Vec<Element>,
    /// The grid this element opens, set only for `Kind::Grid`.
    pub grid: Option<GridSize>,
}

/// A description that parsed, and the colour names in it that nobody knows.
#[derive(Clone, Debug, PartialEq)]
pub struct Parsed {
    pub description: Description,
    pub warnings: Vec<String>,
}

pub fn parse(value: &Value, registry: &TokenRegistry) -> Result<Parsed, Refusal> {
    let object = value
        .as_object()
        .ok_or_else(|| Refusal::Malformed("a description is a JSON object".to_owned()))?;
    if object.get("version").and_then(Value::as_u64) != Some(VERSION) {
        return Err(Refusal::UnsupportedVersion);
    }
    let root = object
        .get("root")
        .ok_or_else(|| Refusal::Malformed("a description needs a root element".to_owned()))?;
    let mut count = 0usize;
    let mut warnings = Vec::new();
    let root = element(root, 0, &mut count, registry, &mut warnings)?;
    Ok(Parsed {
        description: Description { root },
        warnings,
    })
}

fn element(
    value: &Value,
    depth: usize,
    count: &mut usize,
    registry: &TokenRegistry,
    warnings: &mut Vec<String>,
) -> Result<Element, Refusal> {
    if depth > MAX_DEPTH {
        return Err(Refusal::Malformed(format!(
            "elements nest deeper than {MAX_DEPTH}"
        )));
    }
    *count += 1;
    if *count > MAX_ELEMENTS {
        return Err(Refusal::Malformed(format!(
            "more than {MAX_ELEMENTS} elements"
        )));
    }
    let object = value
        .as_object()
        .ok_or_else(|| Refusal::Malformed("an element is a JSON object".to_owned()))?;
    let kind_name = object
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| Refusal::Malformed("an element needs a kind".to_owned()))?;
    let kind = Kind::parse(kind_name).ok_or_else(|| Refusal::UnknownKind(kind_name.to_owned()))?;

    if kind == Kind::Grid && depth > 0 {
        return Err(Refusal::Malformed(
            "a grid is the root element; it cannot sit inside a box".to_owned(),
        ));
    }

    let style = match object.get("style") {
        None => Vec::new(),
        Some(Value::String(text)) => {
            let mut tokens = Vec::new();
            for token in text.split_whitespace() {
                if !style::is_known(token) {
                    return Err(Refusal::UnknownStyle(token.to_owned()));
                }
                tokens.push(token.to_owned());
            }
            tokens
        }
        Some(_) => {
            return Err(Refusal::Malformed(
                "style is a string of utility tokens".to_owned(),
            ));
        }
    };

    let color = color_ref(object, "color", Role::Text, registry, warnings)?;
    let background = color_ref(object, "bg", Role::Fill, registry, warnings)?;
    let border = color_ref(object, "border", Role::Fill, registry, warnings)?;
    let text = string_field(object, "text")?;
    let svg = string_field(object, "svg")?;
    let on_click = string_field(object, "on_click")?;

    match kind {
        Kind::Text | Kind::Button if text.is_none() => {
            return Err(Refusal::Malformed(format!("a {} needs text", kind.name())));
        }
        Kind::Image if svg.is_none() => {
            return Err(Refusal::Malformed("an image needs svg".to_owned()));
        }
        Kind::Image if on_click.is_some() => {
            return Err(Refusal::Malformed(
                "an image is not clickable; put it in a box with on_click".to_owned(),
            ));
        }
        _ => {}
    }

    let grid = if kind == Kind::Grid {
        if on_click.is_some() {
            return Err(Refusal::Malformed(
                "a grid has no on_click; it receives input as a whole".to_owned(),
            ));
        }
        if text.is_some() || svg.is_some() {
            return Err(Refusal::Malformed(
                "a grid has no text or svg; its cells arrive as rows".to_owned(),
            ));
        }
        let dimension = |key: &str, max: u16| -> Result<u16, Refusal> {
            object
                .get(key)
                .and_then(Value::as_u64)
                .and_then(|value| u16::try_from(value).ok())
                .filter(|value| (1..=max).contains(value))
                .ok_or_else(|| Refusal::Malformed(format!("a grid needs {key} from 1 to {max}")))
        };
        Some(GridSize {
            cols: dimension("cols", crate::surface::grid::MAX_COLS)?,
            rows: dimension("rows", crate::surface::grid::MAX_ROWS)?,
        })
    } else {
        None
    };

    let children = match object.get("children") {
        None => Vec::new(),
        Some(Value::Array(items)) if matches!(kind, Kind::Box | Kind::List) => items
            .iter()
            .map(|item| element(item, depth + 1, count, registry, warnings))
            .collect::<Result<Vec<Element>, Refusal>>()?,
        Some(Value::Array(_)) => {
            return Err(Refusal::Malformed(format!(
                "a {} has no children",
                kind.name()
            )));
        }
        Some(_) => {
            return Err(Refusal::Malformed(
                "children is an array of elements".to_owned(),
            ));
        }
    };

    Ok(Element {
        kind,
        style,
        color,
        background,
        border,
        text,
        svg,
        on_click,
        children,
        grid,
    })
}

fn string_field(
    object: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Option<String>, Refusal> {
    match object.get(key) {
        None => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(Refusal::Malformed(format!("{key} is a string"))),
    }
}

fn color_ref(
    object: &serde_json::Map<String, Value>,
    key: &str,
    role: Role,
    registry: &TokenRegistry,
    warnings: &mut Vec<String>,
) -> Result<Option<ColorRef>, Refusal> {
    let Some(value) = object.get(key) else {
        return Ok(None);
    };
    let text = value
        .as_str()
        .ok_or_else(|| Refusal::Malformed(format!("{key} is a token name or a #rrggbb colour")))?;
    if text.starts_with('#') {
        return Colors::parse_hex(text)
            .map(|color| Some(ColorRef::Literal(color)))
            .ok_or_else(|| Refusal::Malformed(format!("{key} {text:?} is not a #rrggbb colour")));
    }
    if !registry.is_known(text) {
        warnings.push(format!("unknown token {text}; using {}", role.fallback()));
    }
    Ok(Some(ColorRef::Token(text.to_owned())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Colors;
    use crate::surface::Refusal;
    use crate::tokens::TokenRegistry;
    use serde_json::json;

    fn registry() -> TokenRegistry {
        TokenRegistry::new(&Colors::default())
    }

    fn parsed(value: serde_json::Value) -> Parsed {
        parse(&value, &registry()).expect("a valid description")
    }

    fn refused(value: serde_json::Value) -> Refusal {
        parse(&value, &registry()).expect_err("an invalid description")
    }

    #[test]
    fn every_kind_parses_with_its_fields() {
        let parsed = parsed(json!({
            "version": 1,
            "root": { "kind": "box", "style": "flex flex_col gap_2", "bg": "terminal.background",
                "children": [
                    { "kind": "text", "text": "Files", "color": "#c0caf5", "style": "text_sm font_bold" },
                    { "kind": "list", "children": [
                        { "kind": "box", "on_click": "row-1", "children": [
                            { "kind": "image", "svg": "<svg/>", "style": "w_4 h_4" },
                            { "kind": "text", "text": "src" }
                        ] }
                    ] },
                    { "kind": "button", "text": "Refresh", "on_click": "refresh", "border": "ansi.4" }
                ] }
        }));
        assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
        let root = &parsed.description.root;
        assert_eq!(root.kind, Kind::Box);
        assert_eq!(root.style, vec!["flex", "flex_col", "gap_2"]);
        assert_eq!(
            root.background,
            Some(ColorRef::Token("terminal.background".into()))
        );
        assert_eq!(root.children.len(), 3);
        assert_eq!(root.children[0].kind, Kind::Text);
        assert_eq!(
            root.children[0].color,
            Some(ColorRef::Literal(sprite_term::Rgb {
                r: 0xc0,
                g: 0xca,
                b: 0xf5
            }))
        );
        let list = &root.children[1];
        assert_eq!(list.kind, Kind::List);
        let row = &list.children[0];
        assert_eq!(row.on_click.as_deref(), Some("row-1"));
        assert_eq!(row.children[0].kind, Kind::Image);
        assert_eq!(row.children[0].svg.as_deref(), Some("<svg/>"));
        let button = &root.children[2];
        assert_eq!(button.kind, Kind::Button);
        assert_eq!(button.text.as_deref(), Some("Refresh"));
        assert_eq!(button.border, Some(ColorRef::Token("ansi.4".into())));
    }

    #[test]
    fn an_unknown_kind_an_unknown_style_token_and_an_unsupported_version_are_distinct_refusals() {
        assert_eq!(
            refused(json!({ "version": 1, "root": { "kind": "blob" } })),
            Refusal::UnknownKind("blob".into())
        );
        assert_eq!(
            refused(json!({ "version": 1, "root": { "kind": "box", "style": "flex sparkle" } })),
            Refusal::UnknownStyle("sparkle".into())
        );
        assert_eq!(
            refused(json!({ "version": 2, "root": { "kind": "box" } })),
            Refusal::UnsupportedVersion
        );
        assert_eq!(
            refused(json!({ "root": { "kind": "box" } })),
            Refusal::UnsupportedVersion
        );
    }

    #[test]
    fn a_description_missing_what_its_kind_needs_is_malformed() {
        assert!(matches!(
            refused(json!({ "version": 1 })),
            Refusal::Malformed(_)
        ));
        assert!(matches!(refused(json!([1, 2])), Refusal::Malformed(_)));
        assert!(matches!(
            refused(json!({ "version": 1, "root": { "kind": "text" } })),
            Refusal::Malformed(why) if why.contains("text")
        ));
        assert!(matches!(
            refused(json!({ "version": 1, "root": { "kind": "image" } })),
            Refusal::Malformed(why) if why.contains("svg")
        ));
        assert!(matches!(
            refused(json!({ "version": 1, "root": { "kind": "text", "text": "x", "children": [] } })),
            Refusal::Malformed(why) if why.contains("children")
        ));
        assert!(matches!(
            refused(json!({ "version": 1, "root": { "kind": "box", "bg": "#12" } })),
            Refusal::Malformed(why) if why.contains("#rrggbb")
        ));
        assert!(matches!(
            refused(json!({ "version": 1, "root": { "kind": "box", "style": 7 } })),
            Refusal::Malformed(_)
        ));
    }

    #[test]
    fn an_unknown_colour_token_is_a_warning_that_names_the_fallback_not_a_refusal() {
        let parsed = parsed(json!({
            "version": 1,
            "root": { "kind": "text", "text": "x", "color": "svgtree.iconn", "bg": "nope.fill" }
        }));
        assert_eq!(
            parsed.warnings,
            vec![
                "unknown token svgtree.iconn; using terminal.foreground".to_owned(),
                "unknown token nope.fill; using terminal.background".to_owned(),
            ]
        );
        let registry = registry();
        let root = &parsed.description.root;
        assert_eq!(
            root.color
                .as_ref()
                .expect("color")
                .resolve(&registry, crate::tokens::Role::Text),
            crate::tokens::unpack(crate::tokens::DEFAULT_FOREGROUND)
        );
    }

    #[test]
    fn a_grid_is_a_root_with_a_size_and_nothing_inside() {
        let grid = parsed(
            json!({ "version": 1, "root": { "kind": "grid", "cols": 80, "rows": 24, "style": "p_1", "bg": "terminal.background" } }),
        );
        assert_eq!(grid.description.root.kind, Kind::Grid);
        assert_eq!(
            grid.description.grid(),
            Some(GridSize { cols: 80, rows: 24 })
        );
        // A grid root's style and bg are kept, not dropped for being a grid:
        // they dress the wrapper the cell box sits in.
        assert_eq!(grid.description.root.style, vec!["p_1"]);
        assert_eq!(
            grid.description.root.background,
            Some(ColorRef::Token("terminal.background".into()))
        );

        let no_grid = parsed(json!({ "version": 1, "root": { "kind": "box" } }));
        assert_eq!(no_grid.description.grid(), None);

        for (root, needle) in [
            (json!({ "kind": "grid", "rows": 24 }), "cols"),
            (json!({ "kind": "grid", "cols": 0, "rows": 24 }), "cols"),
            (json!({ "kind": "grid", "cols": 80, "rows": 5000 }), "rows"),
            (
                json!({ "kind": "grid", "cols": 80, "rows": 24, "children": [] }),
                "children",
            ),
            (
                json!({ "kind": "grid", "cols": 80, "rows": 24, "on_click": "x" }),
                "on_click",
            ),
            (
                json!({ "kind": "box", "children": [{ "kind": "grid", "cols": 8, "rows": 2 }] }),
                "root",
            ),
        ] {
            let refusal = refused(json!({ "version": 1, "root": root }));
            assert!(
                matches!(&refusal, Refusal::Malformed(why) if why.contains(needle)),
                "{refusal:?} should mention {needle}"
            );
        }
    }

    #[test]
    fn too_many_or_too_deep_elements_are_malformed() {
        let mut deep = json!({ "kind": "box" });
        for _ in 0..(MAX_DEPTH + 1) {
            deep = json!({ "kind": "box", "children": [deep] });
        }
        assert!(matches!(
            refused(json!({ "version": 1, "root": deep })),
            Refusal::Malformed(why) if why.contains("deep")
        ));

        let many: Vec<serde_json::Value> = (0..MAX_ELEMENTS)
            .map(|_| json!({ "kind": "box" }))
            .collect();
        assert!(matches!(
            refused(json!({ "version": 1, "root": { "kind": "box", "children": many } })),
            Refusal::Malformed(why) if why.contains("elements")
        ));
    }
}
