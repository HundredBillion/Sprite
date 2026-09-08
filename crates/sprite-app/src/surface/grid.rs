//! The grid Surface: a program's screen of cells, each carrying a highlight
//! id, streamed row by row and drawn by the same painter as the terminal.
//!
//! Sprite holds the whole grid so the program need not: an editor's adapter
//! forwards each redraw as it arrives and keeps nothing but the batch in
//! flight. Highlight ids resolve to attributes the program defined, then the
//! theme's `[highlights]` entry for the group name the id was given, then the
//! grid's default colours, then the pane's. A cell with no colour of its own
//! stays `Default` here and is filled by the painter, exactly as a terminal
//! cell is, so the two paths cannot drift apart.

use std::collections::HashMap;

use serde_json::Value;
use sprite_term::{CellStyle, CursorSnapshot, CursorStyle, Rgb, SnapshotColor, UnderlineStyle};

use crate::config::{Colors, HighlightStyle, Highlights};
use crate::grid::PositionedCell;
use crate::surface::Refusal;

/// Wide enough for any editor a person would run in a pane; a limit so a
/// misbehaving program cannot ask for a gigabyte of cells.
pub const MAX_COLS: u16 = 1024;
pub const MAX_ROWS: u16 = 1024;

/// What a highlight id means: Neovim's `hl_attr_define`, with colours as
/// `#rrggbb` because that is how every colour on this channel is written.
#[derive(Clone, Debug, PartialEq)]
pub struct Attrs {
    pub fg: Option<Rgb>,
    pub bg: Option<Rgb>,
    /// The "special" colour: underlines and undercurls.
    pub sp: Option<Rgb>,
    pub bold: bool,
    pub italic: bool,
    pub reverse: bool,
    pub strikethrough: bool,
    pub underline: UnderlineStyle,
}

impl Default for Attrs {
    fn default() -> Self {
        Self {
            fg: None,
            bg: None,
            sp: None,
            bold: false,
            italic: false,
            reverse: false,
            strikethrough: false,
            underline: UnderlineStyle::None,
        }
    }
}

/// The colours a cell falls back to when its highlight sets none.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Defaults {
    pub fg: Option<Rgb>,
    pub bg: Option<Rgb>,
    pub sp: Option<Rgb>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cursor {
    pub row: u16,
    pub col: u16,
    pub shape: CursorStyle,
    pub visible: bool,
    pub blink: bool,
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            row: 0,
            col: 0,
            shape: CursorStyle::Block,
            visible: true,
            blink: false,
        }
    }
}

/// A cursor message: position always, the rest only when the program says.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CursorOp {
    pub row: u16,
    pub col: u16,
    pub shape: Option<CursorStyle>,
    pub visible: Option<bool>,
    pub blink: Option<bool>,
}

/// One cell as the program sent it. An empty `text` is the second column of
/// the wide character before it, which is how Neovim spells width.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Cell {
    pub text: String,
    pub hl: u32,
}

/// A run of cells written from `col` on `row`, with the carried highlight
/// filled in but repeats still counted rather than expanded, so nothing
/// downstream re-reads the wire rules.
///
/// The repeat stays a count until the operation is known to fit: a message of
/// `["", 0, 1024]` chunks would otherwise demand gigabytes of cells at parse
/// time and only then be refused for running past the grid's edge.
#[derive(Clone, Debug, PartialEq)]
pub struct RowChunk {
    pub row: u16,
    pub col: u16,
    /// Each cell and how many columns it fills; every count is at least one.
    pub cells: Vec<(Cell, u32)>,
}

impl RowChunk {
    /// The column just past the chunk's last, in `u64` so repeats that sum
    /// beyond `u16` still compare against the grid instead of wrapping.
    fn end_col(&self) -> u64 {
        u64::from(self.col)
            + self
                .cells
                .iter()
                .map(|(_, repeat)| u64::from(*repeat))
                .sum::<u64>()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Rows(Vec<RowChunk>),
    Highlights {
        define: Vec<(u32, Attrs)>,
        groups: Vec<(String, u32)>,
    },
    Defaults(Defaults),
    Cursor(CursorOp),
    Resize {
        cols: u16,
        rows: u16,
    },
    Scroll {
        top: u16,
        bot: u16,
        left: u16,
        right: u16,
        rows: i32,
    },
    Clear,
}

/// Whether a message `type` is a grid operation (or a batch of them).
pub fn is_op(kind: &str) -> bool {
    matches!(
        kind,
        "rows" | "highlights" | "defaults" | "cursor" | "resize" | "scroll" | "clear" | "batch"
    )
}

/// The operations a message carries: one, or a batch's in order.
pub fn parse_ops(message: &Value) -> Result<Vec<Op>, Refusal> {
    match message.get("type").and_then(Value::as_str) {
        Some("batch") => {
            let ops = message
                .get("ops")
                .and_then(Value::as_array)
                .ok_or_else(|| malformed("a batch needs ops, an array of operations"))?;
            ops.iter()
                .map(|op| {
                    if op.get("type").and_then(Value::as_str) == Some("batch") {
                        return Err(malformed("a batch does not nest another batch"));
                    }
                    parse_op(op)
                })
                .collect()
        }
        _ => Ok(vec![parse_op(message)?]),
    }
}

fn malformed(why: impl Into<String>) -> Refusal {
    Refusal::Malformed(why.into())
}

fn parse_op(message: &Value) -> Result<Op, Refusal> {
    let object = message
        .as_object()
        .ok_or_else(|| malformed("an operation is a JSON object"))?;
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| malformed("an operation needs a type"))?;
    match kind {
        "rows" => {
            let chunks = object
                .get("rows")
                .and_then(Value::as_array)
                .ok_or_else(|| malformed("rows needs rows, an array of row chunks"))?;
            Ok(Op::Rows(
                chunks.iter().map(parse_chunk).collect::<Result<_, _>>()?,
            ))
        }
        "highlights" => {
            let mut define = Vec::new();
            if let Some(entries) = object.get("define") {
                let entries = entries
                    .as_object()
                    .ok_or_else(|| malformed("highlights.define is an object of id to attrs"))?;
                for (id, attrs) in entries {
                    let id: u32 = id
                        .parse()
                        .map_err(|_| malformed(format!("highlight id {id:?} is not a number")))?;
                    if id == 0 {
                        return Err(malformed(
                            "highlight 0 is the default and cannot be defined",
                        ));
                    }
                    define.push((id, parse_attrs(attrs)?));
                }
            }
            let mut groups = Vec::new();
            if let Some(entries) = object.get("groups") {
                let entries = entries
                    .as_object()
                    .ok_or_else(|| malformed("highlights.groups is an object of name to id"))?;
                for (name, id) in entries {
                    let id = id
                        .as_u64()
                        .and_then(|id| u32::try_from(id).ok())
                        .ok_or_else(|| malformed(format!("group {name:?} needs a numeric id")))?;
                    groups.push((name.clone(), id));
                }
            }
            Ok(Op::Highlights { define, groups })
        }
        "defaults" => Ok(Op::Defaults(Defaults {
            fg: color_field(object, "fg")?,
            bg: color_field(object, "bg")?,
            sp: color_field(object, "sp")?,
        })),
        "cursor" => Ok(Op::Cursor(CursorOp {
            row: cell_index(object, "row")?,
            col: cell_index(object, "col")?,
            shape: match object.get("shape").and_then(Value::as_str) {
                None => None,
                Some("block") => Some(CursorStyle::Block),
                Some("bar") => Some(CursorStyle::Bar),
                Some("underline") => Some(CursorStyle::Underline),
                Some("hollow") => Some(CursorStyle::BlockHollow),
                Some(other) => {
                    return Err(malformed(format!(
                        "cursor shape {other:?} is not block, bar, underline, or hollow"
                    )));
                }
            },
            visible: flag_field(object, "visible")?,
            blink: flag_field(object, "blink")?,
        })),
        "resize" => Ok(Op::Resize {
            cols: cell_index(object, "cols")?,
            rows: cell_index(object, "rows")?,
        }),
        "scroll" => Ok(Op::Scroll {
            top: cell_index(object, "top")?,
            bot: cell_index(object, "bot")?,
            left: cell_index(object, "left")?,
            right: cell_index(object, "right")?,
            rows: object
                .get("rows")
                .and_then(Value::as_i64)
                .and_then(|rows| i32::try_from(rows).ok())
                .ok_or_else(|| malformed("scroll needs rows, a signed count"))?,
        }),
        "clear" => Ok(Op::Clear),
        other => Err(malformed(format!("{other} is not a grid operation"))),
    }
}

fn parse_chunk(value: &Value) -> Result<RowChunk, Refusal> {
    let object = value
        .as_object()
        .ok_or_else(|| malformed("a row chunk is a JSON object"))?;
    let row = cell_index(object, "row")?;
    let col = match object.get("col") {
        None => 0,
        Some(_) => cell_index(object, "col")?,
    };
    let items = object
        .get("cells")
        .and_then(Value::as_array)
        .ok_or_else(|| malformed("a row chunk needs cells"))?;
    let mut cells = Vec::with_capacity(items.len());
    let mut hl = 0u32;
    for item in items {
        let parts = item
            .as_array()
            .ok_or_else(|| malformed("a cell is [text], [text, hl], or [text, hl, repeat]"))?;
        let text = parts
            .first()
            .and_then(Value::as_str)
            .ok_or_else(|| malformed("a cell's text is a string"))?;
        if let Some(given) = parts.get(1) {
            hl = given
                .as_u64()
                .and_then(|hl| u32::try_from(hl).ok())
                .ok_or_else(|| malformed("a cell's hl is a number"))?;
        }
        let repeat = match parts.get(2) {
            None => 1u32,
            Some(count) => count
                .as_u64()
                .filter(|count| (1..=u64::from(MAX_COLS)).contains(count))
                .map(|count| count as u32)
                .ok_or_else(|| malformed(format!("a cell's repeat is 1 to {MAX_COLS}")))?,
        };
        cells.push((
            Cell {
                text: text.to_owned(),
                hl,
            },
            repeat,
        ));
    }
    Ok(RowChunk { row, col, cells })
}

fn parse_attrs(value: &Value) -> Result<Attrs, Refusal> {
    let object = value
        .as_object()
        .ok_or_else(|| malformed("highlight attrs are a JSON object"))?;
    Ok(Attrs {
        fg: color_field(object, "fg")?,
        bg: color_field(object, "bg")?,
        sp: color_field(object, "sp")?,
        bold: flag_field(object, "bold")?.unwrap_or(false),
        italic: flag_field(object, "italic")?.unwrap_or(false),
        reverse: flag_field(object, "reverse")?.unwrap_or(false),
        strikethrough: flag_field(object, "strikethrough")?.unwrap_or(false),
        underline: match object.get("underline") {
            None | Some(Value::Bool(false)) => UnderlineStyle::None,
            Some(Value::Bool(true)) => UnderlineStyle::Single,
            Some(Value::String(kind)) => Highlights::parse_underline(kind).ok_or_else(|| {
                malformed(format!(
                    "underline {kind:?} is not single, double, curly, dotted, or dashed"
                ))
            })?,
            Some(_) => return Err(malformed("underline is false or a kind name")),
        },
    })
}

fn color_field(object: &serde_json::Map<String, Value>, key: &str) -> Result<Option<Rgb>, Refusal> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Colors::parse_hex(text)
            .map(Some)
            .ok_or_else(|| malformed(format!("{key} {text:?} is not a #rrggbb colour"))),
        Some(_) => Err(malformed(format!("{key} is a #rrggbb colour"))),
    }
}

fn flag_field(object: &serde_json::Map<String, Value>, key: &str) -> Result<Option<bool>, Refusal> {
    match object.get(key) {
        None => Ok(None),
        Some(Value::Bool(flag)) => Ok(Some(*flag)),
        Some(_) => Err(malformed(format!("{key} is true or false"))),
    }
}

fn cell_index(object: &serde_json::Map<String, Value>, key: &str) -> Result<u16, Refusal> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .ok_or_else(|| malformed(format!("{key} is a whole number of cells")))
}

/// A program's grid as Sprite holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct GridSurface {
    cols: u16,
    rows: u16,
    cells: Vec<Vec<Cell>>,
    attrs: HashMap<u32, Attrs>,
    groups: HashMap<u32, String>,
    defaults: Defaults,
    cursor: Cursor,
    /// The rows as the painter wants them, rebuilt only when something
    /// changed: a frame that repaints an idle grid costs no layout.
    laid_out: Option<Vec<Vec<PositionedCell>>>,
}

fn blank() -> Cell {
    Cell {
        text: " ".to_owned(),
        hl: 0,
    }
}

impl GridSurface {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            cols,
            rows,
            cells: vec![vec![blank(); usize::from(cols)]; usize::from(rows)],
            attrs: HashMap::new(),
            groups: HashMap::new(),
            defaults: Defaults::default(),
            cursor: Cursor::default(),
            laid_out: None,
        }
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    /// Applies operations in order and stops at the first bad one, which is
    /// refused; the ones before it stand, as a terminal's would.
    pub fn apply_all(&mut self, ops: Vec<Op>) -> Result<(), Refusal> {
        for op in ops {
            self.apply(op)?;
        }
        Ok(())
    }

    pub fn apply(&mut self, op: Op) -> Result<(), Refusal> {
        match op {
            Op::Rows(chunks) => {
                // Every chunk is checked before any is written, so a bad one
                // late in the operation leaves the grid as it was — and no
                // repeat is expanded until the whole operation is known to fit.
                for chunk in &chunks {
                    if chunk.row >= self.rows {
                        return Err(malformed(format!(
                            "row {} is past the grid's {} rows",
                            chunk.row, self.rows
                        )));
                    }
                    if chunk.end_col() > u64::from(self.cols) {
                        return Err(malformed(format!(
                            "the chunk at row {} col {} runs past the grid's {} columns",
                            chunk.row, chunk.col, self.cols
                        )));
                    }
                }
                for chunk in chunks {
                    let row = &mut self.cells[usize::from(chunk.row)];
                    let mut column = usize::from(chunk.col);
                    for (cell, repeat) in chunk.cells {
                        for _ in 0..repeat {
                            row[column] = cell.clone();
                            column += 1;
                        }
                    }
                }
            }
            Op::Highlights { define, groups } => {
                for (id, attrs) in define {
                    self.attrs.insert(id, attrs);
                }
                for (name, id) in groups {
                    self.groups.insert(id, name);
                }
            }
            Op::Defaults(defaults) => {
                if defaults.fg.is_some() {
                    self.defaults.fg = defaults.fg;
                }
                if defaults.bg.is_some() {
                    self.defaults.bg = defaults.bg;
                }
                if defaults.sp.is_some() {
                    self.defaults.sp = defaults.sp;
                }
            }
            Op::Cursor(cursor) => {
                if cursor.row >= self.rows || cursor.col >= self.cols {
                    return Err(malformed(format!(
                        "the cursor at row {} col {} is outside the grid",
                        cursor.row, cursor.col
                    )));
                }
                self.cursor.row = cursor.row;
                self.cursor.col = cursor.col;
                if let Some(shape) = cursor.shape {
                    self.cursor.shape = shape;
                }
                if let Some(visible) = cursor.visible {
                    self.cursor.visible = visible;
                }
                if let Some(blink) = cursor.blink {
                    self.cursor.blink = blink;
                }
            }
            Op::Resize { cols, rows } => {
                if !(1..=MAX_COLS).contains(&cols) || !(1..=MAX_ROWS).contains(&rows) {
                    return Err(malformed(format!(
                        "a grid is 1 to {MAX_COLS} columns by 1 to {MAX_ROWS} rows, not {cols} by {rows}"
                    )));
                }
                for row in &mut self.cells {
                    row.resize(usize::from(cols), blank());
                }
                self.cells
                    .resize(usize::from(rows), vec![blank(); usize::from(cols)]);
                self.cols = cols;
                self.rows = rows;
                self.cursor.row = self.cursor.row.min(rows - 1);
                self.cursor.col = self.cursor.col.min(cols - 1);
            }
            Op::Scroll {
                top,
                bot,
                left,
                right,
                rows,
            } => {
                if top >= bot || bot > self.rows || left >= right || right > self.cols {
                    return Err(malformed(format!(
                        "the scroll region rows {top}..{bot} cols {left}..{right} is not inside the grid"
                    )));
                }
                let (top, bot, left, right) = (
                    usize::from(top),
                    usize::from(bot),
                    usize::from(left),
                    usize::from(right),
                );
                let height = bot - top;
                let distance = rows.unsigned_abs() as usize;
                if distance >= height {
                    for row in &mut self.cells[top..bot] {
                        row[left..right].fill(blank());
                    }
                } else if rows > 0 {
                    // Content moves up: row r takes row r + distance.
                    for r in top..bot - distance {
                        let (upper, lower) = self.cells.split_at_mut(r + distance);
                        upper[r][left..right].clone_from_slice(&lower[0][left..right]);
                    }
                    for row in &mut self.cells[bot - distance..bot] {
                        row[left..right].fill(blank());
                    }
                } else if rows < 0 {
                    // Content moves down: row r takes row r - distance.
                    for r in (top + distance..bot).rev() {
                        let (upper, lower) = self.cells.split_at_mut(r);
                        lower[0][left..right].clone_from_slice(&upper[r - distance][left..right]);
                    }
                    for row in &mut self.cells[top..top + distance] {
                        row[left..right].fill(blank());
                    }
                }
            }
            Op::Clear => {
                for row in &mut self.cells {
                    row.fill(blank());
                }
            }
        }
        self.laid_out = None;
        Ok(())
    }

    /// Forgets the laid-out rows, for when the theme changed under them.
    pub fn invalidate(&mut self) {
        self.laid_out = None;
    }

    /// The rows as the painter takes them, laid out on demand.
    pub fn positioned_rows(&mut self, theme: &Highlights) -> &[Vec<PositionedCell>] {
        if self.laid_out.is_none() {
            let rows = self
                .cells
                .iter()
                .map(|row| self.lay_out(row, theme))
                .collect();
            self.laid_out = Some(rows);
        }
        self.laid_out.as_deref().expect("laid out just above")
    }

    fn lay_out(&self, row: &[Cell], theme: &Highlights) -> Vec<PositionedCell> {
        let mut placed = Vec::with_capacity(row.len());
        for (column, cell) in row.iter().enumerate() {
            // An empty cell is the second half of the wide character before it;
            // that character already covers this column.
            if cell.text.is_empty() {
                continue;
            }
            let wide = row.get(column + 1).is_some_and(|next| next.text.is_empty());
            placed.push(PositionedCell {
                column: column as u16,
                columns: if wide { 2 } else { 1 },
                text: cell.text.clone(),
                style: self.style_for(cell.hl, theme),
                selected: false,
            });
        }
        placed
    }

    /// The program's attrs for an id, with the theme's say over the group the
    /// id was named as, in the shape the painter reads for a terminal cell.
    fn style_for(&self, hl: u32, theme: &Highlights) -> CellStyle {
        let mut attrs = self.attrs.get(&hl).cloned().unwrap_or_default();
        if let Some(style) = self.groups.get(&hl).and_then(|name| theme.get(name)) {
            apply_theme(&mut attrs, style);
        }
        let color = |value: Option<Rgb>| value.map_or(SnapshotColor::Default, SnapshotColor::Rgb);
        CellStyle {
            foreground: color(attrs.fg),
            background: color(attrs.bg),
            underline_color: color(attrs.sp.or(self.defaults.sp)),
            bold: attrs.bold,
            italic: attrs.italic,
            faint: false,
            blink: false,
            inverse: attrs.reverse,
            invisible: false,
            strikethrough: attrs.strikethrough,
            overline: false,
            underline: attrs.underline,
        }
    }

    pub fn cursor_snapshot(&self) -> CursorSnapshot {
        CursorSnapshot {
            row: self.cursor.row,
            column: self.cursor.col,
            visible: self.cursor.visible,
            blinking: self.cursor.blink,
            style: self.cursor.shape,
        }
    }

    /// The grid's default colours, falling back to the pane's.
    pub fn default_colors(&self, fallback: (Rgb, Rgb)) -> (Rgb, Rgb) {
        (
            self.defaults.fg.unwrap_or(fallback.0),
            self.defaults.bg.unwrap_or(fallback.1),
        )
    }
}

/// Lays a theme's highlight-group override over a program's attrs.
fn apply_theme(attrs: &mut Attrs, style: &HighlightStyle) {
    if let Some(color) = style.color {
        attrs.fg = Some(color);
    }
    if let Some(color) = style.background {
        attrs.bg = Some(color);
    }
    if let Some(bold) = style.bold {
        attrs.bold = bold;
    }
    if let Some(italic) = style.italic {
        attrs.italic = italic;
    }
    if let Some(underline) = style.underline {
        attrs.underline = underline;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use crate::config::{HighlightStyle, Highlights};
    use crate::tokens::unpack;

    fn ops(message: serde_json::Value) -> Vec<Op> {
        parse_ops(&message).expect("valid operations")
    }

    fn refused(message: serde_json::Value) -> Refusal {
        parse_ops(&message).expect_err("invalid operations")
    }

    fn text_of(row: &[PositionedCell]) -> String {
        row.iter().map(|cell| cell.text.as_str()).collect()
    }

    #[test]
    fn a_rows_chunk_carries_the_highlight_forward_and_expands_repeats() {
        let mut grid = GridSurface::new(8, 2);
        grid.apply_all(ops(json!({
            "type": "rows",
            "rows": [{ "row": 1, "col": 1, "cells": [["H", 3], ["i"], ["!", 0, 2], ["x", 5]] }]
        })))
        .expect("apply");
        let rows = grid.positioned_rows(&Highlights::default());
        assert_eq!(text_of(&rows[1]), " Hi!!x  ");
        let cells = &grid.cells[1];
        assert_eq!(cells[1].hl, 3);
        assert_eq!(cells[2].hl, 3, "a missing hl repeats the previous cell's");
        assert_eq!(cells[3].hl, 0);
        assert_eq!(cells[4].hl, 0);
        assert_eq!(cells[5].hl, 5);
        assert_eq!(
            cells[0].hl, 0,
            "the first cell of a chunk defaults to 0 only when it omits hl"
        );
    }

    #[test]
    fn an_empty_cell_after_another_makes_it_wide() {
        let mut grid = GridSurface::new(4, 1);
        grid.apply_all(ops(
            json!({ "type": "rows", "rows": [{ "row": 0, "cells": [["界", 1], [""], ["b", 1]] }] }),
        ))
        .expect("apply");
        let rows = grid.positioned_rows(&Highlights::default());
        assert_eq!(
            rows[0].len(),
            3,
            "the tail draws nothing of its own: 界, b, and the trailing blank"
        );
        assert_eq!(rows[0][0].text, "界");
        assert_eq!(rows[0][0].columns, 2);
        assert_eq!(rows[0][1].column, 2);
        assert_eq!(rows[0][1].text, "b");
    }

    #[test]
    fn a_chunk_past_the_grid_is_refused_not_clipped() {
        let mut grid = GridSurface::new(4, 2);
        let past = grid.apply_all(ops(
            json!({ "type": "rows", "rows": [{ "row": 0, "col": 3, "cells": [["a"], ["b"]] }] }),
        ));
        assert!(matches!(past, Err(Refusal::Malformed(why)) if why.contains("past the grid")));
        let below = grid.apply_all(ops(
            json!({ "type": "rows", "rows": [{ "row": 2, "cells": [["a"]] }] }),
        ));
        assert!(matches!(below, Err(Refusal::Malformed(why)) if why.contains("row 2")));
        assert_eq!(
            text_of(&grid.positioned_rows(&Highlights::default())[0]),
            "    "
        );
    }

    #[test]
    fn repeats_that_sum_past_the_grid_are_refused_before_anything_is_expanded() {
        let mut grid = GridSurface::new(8, 1);
        let parsed = ops(json!({ "type": "rows", "rows": [
            { "row": 0, "cells": [["a", 0, 4]] },
            { "row": 0, "col": 4, "cells": [["b", 0, 1024], ["c", 0, 1024]] }
        ] }));
        let Op::Rows(chunks) = &parsed[0] else {
            panic!("a rows operation");
        };
        // The cost of parsing is the number of entries the message wrote, not
        // the number of columns they claim: 2048 repeats are still two cells.
        assert_eq!(chunks[0].cells.len(), 1);
        assert_eq!(chunks[1].cells.len(), 2);
        assert_eq!(chunks[1].cells[0].1, 1024);

        let refusal = grid.apply_all(parsed);
        assert!(
            matches!(&refusal, Err(Refusal::Malformed(why)) if why.contains("past the grid")),
            "{refusal:?}"
        );
        assert_eq!(
            text_of(&grid.positioned_rows(&Highlights::default())[0]),
            "        ",
            "the first chunk's repeats never reached the grid either"
        );
    }

    #[test]
    fn highlights_define_ids_and_the_theme_overrides_by_group_name() {
        let mut grid = GridSurface::new(2, 1);
        grid.apply_all(ops(json!({
            "type": "batch",
            "ops": [
                { "type": "highlights",
                  "define": { "1": { "fg": "#6c7086", "italic": true, "underline": "curly" },
                              "2": { "bg": "#ff0000", "reverse": true, "strikethrough": true } },
                  "groups": { "Comment": 1 } },
                { "type": "rows", "rows": [{ "row": 0, "cells": [["a", 1], ["b", 2]] }] }
            ]
        })))
        .expect("apply");

        let plain = grid.positioned_rows(&Highlights::default()).to_vec();
        assert_eq!(
            plain[0][0].style.foreground,
            SnapshotColor::Rgb(unpack(0x6c7086))
        );
        assert!(plain[0][0].style.italic);
        assert_eq!(plain[0][0].style.underline, UnderlineStyle::Curly);
        assert_eq!(
            plain[0][1].style.background,
            SnapshotColor::Rgb(unpack(0xff0000))
        );
        assert!(plain[0][1].style.inverse);
        assert!(plain[0][1].style.strikethrough);

        let theme = Highlights {
            groups: vec![(
                "Comment".to_owned(),
                HighlightStyle {
                    color: Some(unpack(0x00ff00)),
                    bold: Some(true),
                    italic: Some(false),
                    underline: Some(UnderlineStyle::None),
                    background: None,
                },
            )],
        };
        grid.invalidate();
        let themed = grid.positioned_rows(&theme);
        assert_eq!(
            themed[0][0].style.foreground,
            SnapshotColor::Rgb(unpack(0x00ff00))
        );
        assert!(themed[0][0].style.bold);
        assert!(!themed[0][0].style.italic, "the theme turned italic off");
        assert_eq!(themed[0][0].style.underline, UnderlineStyle::None);
        // Id 2 has no group name, so the theme cannot reach it.
        assert_eq!(
            themed[0][1].style.background,
            SnapshotColor::Rgb(unpack(0xff0000))
        );
    }

    #[test]
    fn defaults_feed_the_default_colours_and_fall_back_to_the_panes() {
        let mut grid = GridSurface::new(1, 1);
        let pane = (unpack(0xd8d8e0), unpack(0x101014));
        assert_eq!(grid.default_colors(pane), pane);
        grid.apply_all(ops(json!({ "type": "defaults", "fg": "#ffffff" })))
            .expect("apply");
        assert_eq!(
            grid.default_colors(pane),
            (unpack(0xffffff), unpack(0x101014))
        );
        grid.apply_all(ops(
            json!({ "type": "defaults", "bg": "#000000", "sp": "#ff0000" }),
        ))
        .expect("apply");
        assert_eq!(
            grid.default_colors(pane),
            (unpack(0xffffff), unpack(0x000000))
        );
        // A cell with no attr of its own is Default, which the painter fills
        // from default_colors: the grid never bakes the defaults into cells.
        let rows = grid.positioned_rows(&Highlights::default());
        assert_eq!(rows[0][0].style.foreground, SnapshotColor::Default);
    }

    #[test]
    fn scroll_moves_a_region_and_blanks_what_it_vacates() {
        let mut grid = GridSurface::new(3, 4);
        grid.apply_all(ops(json!({ "type": "rows", "rows": [
            { "row": 0, "cells": [["a"], ["a"], ["a"]] },
            { "row": 1, "cells": [["b"], ["b"], ["b"]] },
            { "row": 2, "cells": [["c"], ["c"], ["c"]] },
            { "row": 3, "cells": [["d"], ["d"], ["d"]] }
        ] })))
        .expect("apply");
        grid.apply_all(ops(
            json!({ "type": "scroll", "top": 0, "bot": 3, "left": 0, "right": 3, "rows": 1 }),
        ))
        .expect("apply");
        let rows: Vec<String> = grid
            .positioned_rows(&Highlights::default())
            .iter()
            .map(|row| text_of(row))
            .collect();
        assert_eq!(
            rows,
            vec!["bbb", "ccc", "   ", "ddd"],
            "up by one inside rows 0..3; row 3 untouched"
        );
        grid.apply_all(ops(
            json!({ "type": "scroll", "top": 0, "bot": 4, "left": 1, "right": 3, "rows": -2 }),
        ))
        .expect("apply");
        let rows: Vec<String> = grid
            .positioned_rows(&Highlights::default())
            .iter()
            .map(|row| text_of(row))
            .collect();
        assert_eq!(
            rows,
            vec!["b  ", "c  ", " bb", "dcc"],
            "down by two inside cols 1..3"
        );
        assert!(matches!(
            grid.apply_all(ops(
                json!({ "type": "scroll", "top": 0, "bot": 9, "left": 0, "right": 3, "rows": 1 })
            )),
            Err(Refusal::Malformed(_))
        ));
    }

    #[test]
    fn resize_keeps_what_fits_and_blanks_the_rest() {
        let mut grid = GridSurface::new(3, 2);
        grid.apply_all(ops(json!({ "type": "rows", "rows": [{ "row": 0, "cells": [["a"], ["b"], ["c"]] }, { "row": 1, "cells": [["d"], ["e"], ["f"]] }] })))
            .expect("apply");
        grid.apply_all(ops(json!({ "type": "resize", "cols": 2, "rows": 3 })))
            .expect("apply");
        assert_eq!((grid.cols(), grid.rows()), (2, 3));
        let rows: Vec<String> = grid
            .positioned_rows(&Highlights::default())
            .iter()
            .map(|row| text_of(row))
            .collect();
        assert_eq!(rows, vec!["ab", "de", "  "]);
        assert!(matches!(
            grid.apply_all(ops(json!({ "type": "resize", "cols": 0, "rows": 3 }))),
            Err(Refusal::Malformed(_))
        ));
        assert!(matches!(
            grid.apply_all(ops(json!({ "type": "resize", "cols": 2, "rows": 5000 }))),
            Err(Refusal::Malformed(_))
        ));
    }

    #[test]
    fn clear_resets_every_cell_and_the_cursor_keeps_what_a_partial_message_leaves_out() {
        let mut grid = GridSurface::new(2, 1);
        grid.apply_all(ops(json!({ "type": "batch", "ops": [
            { "type": "highlights", "define": { "1": { "bold": true } } },
            { "type": "rows", "rows": [{ "row": 0, "cells": [["a", 1], ["b", 1]] }] },
            { "type": "cursor", "row": 0, "col": 1, "shape": "bar", "blink": true }
        ] })))
        .expect("apply");
        let cursor = grid.cursor_snapshot();
        assert_eq!((cursor.row, cursor.column), (0, 1));
        assert_eq!(cursor.style, CursorStyle::Bar);
        assert!(cursor.visible && cursor.blinking);

        grid.apply_all(ops(json!({ "type": "cursor", "row": 0, "col": 0 })))
            .expect("apply");
        let cursor = grid.cursor_snapshot();
        assert_eq!(cursor.column, 0);
        assert_eq!(
            cursor.style,
            CursorStyle::Bar,
            "shape kept when the message leaves it out"
        );
        assert!(cursor.blinking);

        grid.apply_all(ops(json!({ "type": "clear" })))
            .expect("apply");
        let rows = grid.positioned_rows(&Highlights::default());
        assert_eq!(text_of(&rows[0]), "  ");
        assert!(!rows[0][0].style.bold, "clear resets highlights to 0");
    }

    #[test]
    fn a_batch_stops_at_its_first_bad_operation_and_says_which() {
        let mut grid = GridSurface::new(2, 1);
        let result = grid.apply_all(ops(json!({ "type": "batch", "ops": [
            { "type": "rows", "rows": [{ "row": 0, "cells": [["a"]] }] },
            { "type": "rows", "rows": [{ "row": 7, "cells": [["b"]] }] },
            { "type": "rows", "rows": [{ "row": 0, "col": 1, "cells": [["c"]] }] }
        ] })));
        assert!(matches!(result, Err(Refusal::Malformed(why)) if why.contains("row 7")));
        assert_eq!(
            text_of(&grid.positioned_rows(&Highlights::default())[0]),
            "a ",
            "the first stood, the third never ran"
        );
    }

    #[test]
    fn every_operation_parses_and_an_unknown_or_misshapen_one_is_malformed() {
        assert_eq!(ops(json!({ "type": "clear" })), vec![Op::Clear]);
        assert!(matches!(
            ops(json!({ "type": "resize", "cols": 3, "rows": 4 }))[0],
            Op::Resize { cols: 3, rows: 4 }
        ));
        assert!(matches!(
            ops(json!({ "type": "batch", "ops": [{ "type": "clear" }, { "type": "clear" }] }))
                .len(),
            2
        ));
        for (message, needle) in [
            (json!({ "type": "rows" }), "rows"),
            (json!({ "type": "rows", "rows": [{ "cells": [] }] }), "row"),
            (
                json!({ "type": "rows", "rows": [{ "row": 0, "cells": [[7]] }] }),
                "text",
            ),
            (
                json!({ "type": "rows", "rows": [{ "row": 0, "cells": [["a", "x"]] }] }),
                "hl",
            ),
            (
                json!({ "type": "highlights", "define": { "zero": {} } }),
                "id",
            ),
            (json!({ "type": "highlights", "define": { "0": {} } }), "0"),
            (
                json!({ "type": "highlights", "define": { "1": { "fg": "red" } } }),
                "fg",
            ),
            (
                json!({ "type": "highlights", "define": { "1": { "underline": "wavy" } } }),
                "underline",
            ),
            (json!({ "type": "cursor", "col": 1 }), "row"),
            (
                json!({ "type": "cursor", "row": 0, "col": 1, "shape": "blob" }),
                "shape",
            ),
            (json!({ "type": "scroll", "top": 0 }), "bot"),
            (json!({ "type": "batch" }), "ops"),
            (
                json!({ "type": "batch", "ops": [{ "type": "batch", "ops": [] }] }),
                "batch",
            ),
            (json!({ "type": "sparkle" }), "sparkle"),
        ] {
            let refusal = refused(message.clone());
            assert!(
                matches!(&refusal, Refusal::Malformed(why) if why.contains(needle)),
                "{message}: {refusal:?} should mention {needle:?}"
            );
        }
        for kind in [
            "rows",
            "highlights",
            "defaults",
            "cursor",
            "resize",
            "scroll",
            "clear",
            "batch",
        ] {
            assert!(is_op(kind), "{kind}");
        }
        assert!(!is_op("update"));
    }
}
