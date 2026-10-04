# Private type contracts

`cargo test -p sprite-app --test type_invariants --locked --offline` compiles these
examples against the production declarations. The runner includes `grid.rs` and
`box_drawing.rs` unchanged. For Element it takes the declaration section of
`description.rs` and removes the color-resolution method, which needs the live
token registry but does not define an Element shape. Utility types come from the
unchanged production `style.rs`. No subject type is recreated in this fixture.

The extraction ends at the exact `/// A description that parsed` marker and
removes only the span from `impl ColorRef {` to `/// A grid's size in cells.`.
Missing markers abort the test. Unused imports for the omitted parser and
resolver are removed explicitly. Each dependency candidate must compile with
the active Rust compiler before selection; paired cases reuse the same externs.

Each negative snippet has the same imports as a compiling positive control.
The runner requires the indicated Rust diagnostic and its subject, so a missing
module or dependency cannot count as proof. These are internal examples because
Surface descriptions and grid painting are private application implementation.
Public ownership and terminal size contracts also have paired rustdoc tests.

## Element payloads

```rust
use crate::surface::description::{Element, ElementStyle};
let image = Element::Image { style: ElementStyle::default(), svg: "<svg/>".into() };
let text = Element::Text { style: ElementStyle::default(), text: "label".into(), on_click: None };
```

```compile_fail,E0559,on_click
use crate::surface::description::{Element, ElementStyle};
let image = Element::Image { style: ElementStyle::default(), svg: "<svg/>".into(), on_click: Some("click".into()) };
```

```compile_fail,E0559,children
use crate::surface::description::{Element, ElementStyle};
let text = Element::Text { style: ElementStyle::default(), text: "label".into(), on_click: None, children: vec![] };
```

```compile_fail,E0063,svg
use crate::surface::description::{Element, ElementStyle};
let image = Element::Image { style: ElementStyle::default() };
```

## Snapped box edges

```rust
use crate::{box_drawing::Cell, grid::{column_edge, row_edge, Col, Row}};
use gpui::px;
let left = column_edge(px(0.3), px(8.4), Col(0), 1.25);
let right = column_edge(px(0.3), px(8.4), Col(2), 1.25);
let top = row_edge(px(0.2), px(16.8), Row(0), 1.25);
let bottom = row_edge(px(0.2), px(16.8), Row(1), 1.25);
let cell = Cell::new(left, top, right, bottom);
```

```compile_fail,E0308,Snapped
use crate::{box_drawing::Cell, grid::{column_edge, row_edge, Col, Row}};
use gpui::px;
let cell = Cell::new(px(0.3), px(0.2), px(17.1), px(17.0));
```

```compile_fail,E0308,Col
use crate::{box_drawing::Cell, grid::{column_edge, row_edge, Col, Row}};
use gpui::px;
let edge = column_edge(px(0.3), px(8.4), Row(0), 1.25);
```
