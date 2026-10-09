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

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use serde_json::Value;
use sprite_term::{CellStyle, CursorSnapshot, CursorStyle, Rgb, SnapshotColor, UnderlineStyle};

use crate::config::{Colors, HighlightStyle, Highlights};
use crate::grid::PositionedCell;
use crate::surface::Refusal;

/// Wide enough for any editor a person would run in a pane; a limit so a
/// misbehaving program cannot ask for a gigabyte of cells.
pub const MAX_COLS: u16 = 1024;
pub const MAX_ROWS: u16 = 1024;
/// How many highlight ids and group names one grid keeps. An editor's
/// adapter forwards ids as the editor allocates them and never retires one,
/// so these are generous: a refusal mid-session would leave that grid's
/// highlighting wrong for the rest of it.
pub const MAX_HIGHLIGHT_IDS: usize = 262_144;
pub const MAX_GROUP_NAMES: usize = 262_144;

#[cfg(test)]
thread_local! {
    /// Stored group entries a relink found and moved. Every stored name a
    /// relink reads must be counted here, so a test can tell work that
    /// grows with the message from work that grows with the grid.
    static RELINKED_ENTRIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

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
pub struct WireCell {
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
    pub cells: Vec<(WireCell, u32)>,
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
            WireCell {
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Cell {
    text: u32,
    pub hl: u32,
}

#[derive(Clone, Debug, PartialEq)]
struct TextEntry {
    text: Arc<str>,
    paint: gpui::SharedString,
    references: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct TextPool {
    entries: Vec<Option<TextEntry>>,
    ids: HashMap<Arc<str>, u32>,
    free: Vec<u32>,
}

impl TextPool {
    // A live cell owns one reference; retired paint rows own their text independently.
    fn intern(&mut self, text: &str) -> u32 {
        if let Some(&id) = self.ids.get(text) {
            self.retain(id);
            return id;
        }
        let id = self.free.pop().unwrap_or(self.entries.len() as u32);
        let text: Arc<str> = text.into();
        let entry = TextEntry {
            paint: gpui::SharedString::new(Arc::clone(&text)),
            text: text.clone(),
            references: 1,
        };
        self.ids.insert(text, id);
        if id as usize == self.entries.len() {
            self.entries.push(Some(entry));
        } else {
            self.entries[id as usize] = Some(entry);
        }
        id
    }

    fn retain(&mut self, id: u32) {
        self.entries[id as usize].as_mut().unwrap().references += 1;
    }

    fn release(&mut self, id: u32) {
        let entry = self.entries[id as usize].as_mut().unwrap();
        entry.references -= 1;
        if entry.references == 0 {
            self.ids.remove(entry.text.as_ref());
            self.entries[id as usize] = None;
            self.free.push(id);
        }
    }

    fn text(&self, id: u32) -> &gpui::SharedString {
        &self.entries[id as usize].as_ref().unwrap().paint
    }
}

/// A program's grid as Sprite holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct GridSurface {
    cols: u16,
    rows: u16,
    cells: Vec<Vec<Cell>>,
    texts: TextPool,
    dirty: Vec<bool>,
    theme: Option<Highlights>,
    attrs: HashMap<u32, Attrs>,
    /// Every group name an id has been given, in the order they arrived,
    /// keyed by an arrival stamp so one name can leave its id without the
    /// others moving.
    ///
    /// One id commonly stands for several names — an editor maps `Comment`,
    /// `@comment`, and `@comment.lua` to the same attrs — so keeping only the
    /// last would let one name shadow a theme entry written for another.
    groups: HashMap<u32, BTreeMap<u64, String>>,
    /// Where each group name sits now: its id and its stamp under that id.
    /// A relink finds the one entry it moves here, instead of searching
    /// every id's names.
    group_of: HashMap<String, (u32, u64)>,
    /// The stamp the next group name to arrive is given.
    next_group_stamp: u64,
    defaults: Defaults,
    cursor: Cursor,
    /// The rows as the painter wants them, rebuilt only when something
    /// changed: a frame that repaints an idle grid costs no layout.
    laid_out: crate::grid::PositionedRows,
}

impl GridSurface {
    pub fn new(cols: u16, rows: u16) -> Self {
        let mut texts = TextPool::default();
        // Slot zero has one permanent reference so blanking never needs an allocation.
        let blank = texts.intern(" ");
        texts.entries[blank as usize].as_mut().unwrap().references =
            usize::from(cols) * usize::from(rows) + 1;
        let empty_row = Arc::new(Vec::new());
        Self {
            cols,
            rows,
            cells: vec![vec![Cell { text: blank, hl: 0 }; usize::from(cols)]; usize::from(rows)],
            texts,
            dirty: vec![true; usize::from(rows)],
            theme: None,
            attrs: HashMap::new(),
            groups: HashMap::new(),
            group_of: HashMap::new(),
            next_group_stamp: 0,
            defaults: Defaults::default(),
            cursor: Cursor::default(),
            laid_out: vec![empty_row; usize::from(rows)].into(),
        }
    }

    fn replace(&mut self, row: usize, col: usize, cell: Cell) {
        let old = self.cells[row][col];
        if old != cell {
            self.texts.retain(cell.text);
            self.texts.release(old.text);
            self.cells[row][col] = cell;
            self.dirty[row] = true;
        }
    }

    fn blank(&self) -> Cell {
        Cell { text: 0, hl: 0 }
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    /// Applies operations in order and stops at the first bad one, which is
    /// refused; the ones before it stand, as a terminal's would. Inside a
    /// batch of several, the refusal names the operation's index so a client
    /// can tell which ones stood.
    pub fn apply_all(&mut self, ops: Vec<Op>) -> Result<(), Refusal> {
        let several = ops.len() > 1;
        for (index, op) in ops.into_iter().enumerate() {
            self.apply(op).map_err(|refusal| match (several, refusal) {
                (true, Refusal::Malformed(why)) => Refusal::Malformed(format!("op {index}: {why}")),
                // Unreachable in practice: `apply` only ever refuses with
                // `Malformed`. Kept so a future refusal kind passes through
                // rather than needing this match widened.
                (_, other) => other,
            })?;
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
                    let mut column = usize::from(chunk.col);
                    for (cell, repeat) in chunk.cells {
                        let text = self.texts.intern(&cell.text);
                        for _ in 0..repeat {
                            self.replace(
                                usize::from(chunk.row),
                                column,
                                Cell { text, hl: cell.hl },
                            );
                            column += 1;
                        }
                        self.texts.release(text);
                    }
                }
            }
            Op::Highlights { define, groups } => {
                // Checked before anything below runs, so an operation over a
                // cap changes nothing at all.
                self.highlights_fit(&define, &groups)?;
                let mut changed = false;
                for (id, attrs) in define {
                    changed |= self.attrs.get(&id) != Some(&attrs);
                    self.attrs.insert(id, attrs);
                }
                for (name, id) in groups {
                    changed |= self
                        .groups
                        .get(&id)
                        .and_then(|names| names.last_key_value())
                        .map(|(_, last)| last)
                        != Some(&name);
                    let stamp = self.next_group_stamp;
                    self.next_group_stamp += 1;
                    // A relink moves the name: an editor that now maps
                    // `Comment` to another attr id no longer means the old one
                    // by it. The index says where the name was, so a relink
                    // costs the same however many names the grid holds.
                    if let Some((old_id, old_stamp)) =
                        self.group_of.insert(name.clone(), (id, stamp))
                        && let Some(names) = self.groups.get_mut(&old_id)
                    {
                        #[cfg(test)]
                        RELINKED_ENTRIES.with(|count| count.set(count.get() + 1));
                        names.remove(&old_stamp);
                        // An id the relink emptied is dropped rather than kept
                        // with no names: `style_for` would look it up and find
                        // nothing to apply, and the entry would outlive the
                        // only reason it existed.
                        if names.is_empty() {
                            self.groups.remove(&old_id);
                        }
                    }
                    self.groups.entry(id).or_default().insert(stamp, name);
                }
                if changed {
                    self.invalidate();
                }
            }
            Op::Defaults(defaults) => {
                let old = self.defaults;
                if defaults.fg.is_some() {
                    self.defaults.fg = defaults.fg;
                }
                if defaults.bg.is_some() {
                    self.defaults.bg = defaults.bg;
                }
                if defaults.sp.is_some() {
                    self.defaults.sp = defaults.sp;
                }
                if old != self.defaults {
                    self.invalidate();
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
                if (cols, rows) != (self.cols, self.rows) {
                    let mut resized = Self::new(cols, rows);
                    for r in 0..usize::from(rows.min(self.rows)) {
                        for c in 0..usize::from(cols.min(self.cols)) {
                            let cell = self.cells[r][c];
                            let text = resized.texts.intern(self.texts.text(cell.text).as_str());
                            resized.replace(r, c, Cell { text, hl: cell.hl });
                            resized.texts.release(text);
                        }
                    }
                    if cols == self.cols {
                        let rows = Arc::make_mut(&mut resized.laid_out);
                        for (index, row) in rows.iter_mut().enumerate().take(usize::from(self.rows))
                        {
                            *row = self.laid_out[index].clone();
                            resized.dirty[index] = self.dirty[index];
                        }
                    }
                    self.cells = resized.cells;
                    self.texts = resized.texts;
                    self.dirty = resized.dirty;
                    self.laid_out = resized.laid_out;
                    self.cols = cols;
                    self.rows = rows;
                }
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
                for target in 0..height {
                    let r = if rows < 0 {
                        bot - 1 - target
                    } else {
                        top + target
                    };
                    for c in left..right {
                        let source = if distance >= height {
                            None
                        } else if rows > 0 {
                            (r + distance < bot).then_some(r + distance)
                        } else {
                            r.checked_sub(distance).filter(|source| *source >= top)
                        };
                        let cell =
                            source.map_or_else(|| self.blank(), |source| self.cells[source][c]);
                        self.replace(r, c, cell);
                    }
                }
            }
            Op::Clear => {
                for r in 0..usize::from(self.rows) {
                    for c in 0..usize::from(self.cols) {
                        self.replace(r, c, self.blank());
                    }
                }
            }
        }
        Ok(())
    }

    /// Refuses one highlights operation that would grow the grid past either
    /// cap. Ids and names are never forgotten, so a grid at its caps can still
    /// redefine and relink what it has, but not add to it.
    fn highlights_fit(
        &self,
        define: &[(u32, Attrs)],
        groups: &[(String, u32)],
    ) -> Result<(), Refusal> {
        let new_ids: HashSet<u32> = define
            .iter()
            .map(|(id, _)| *id)
            .filter(|id| !self.attrs.contains_key(id))
            .collect();
        let new_names: HashSet<&str> = groups
            .iter()
            .map(|(name, _)| name.as_str())
            .filter(|name| !self.group_of.contains_key(*name))
            .collect();
        if self.attrs.len() + new_ids.len() > MAX_HIGHLIGHT_IDS {
            return Err(malformed(format!(
                "a grid defines at most {MAX_HIGHLIGHT_IDS} highlight ids"
            )));
        }
        if self.group_of.len() + new_names.len() > MAX_GROUP_NAMES {
            return Err(malformed(format!(
                "a grid names at most {MAX_GROUP_NAMES} highlight groups"
            )));
        }
        Ok(())
    }

    pub fn invalidate(&mut self) {
        self.dirty.fill(true);
    }

    pub fn positioned_rows(&mut self, theme: &Highlights) -> crate::grid::PositionedRows {
        if self.theme.as_ref() != Some(theme) {
            self.invalidate();
            self.theme = Some(theme.clone());
        }
        for index in 0..self.cells.len() {
            if self.dirty[index] {
                let row = Arc::new(self.lay_out(&self.cells[index], theme));
                Arc::make_mut(&mut self.laid_out)[index] = row;
                self.dirty[index] = false;
            }
        }
        self.laid_out.clone()
    }

    fn lay_out(&self, row: &[Cell], theme: &Highlights) -> Vec<PositionedCell> {
        let mut placed = Vec::with_capacity(row.len());
        for (column, cell) in row.iter().enumerate() {
            // An empty cell is the second half of the wide character before it;
            // that character already covers this column.
            if self.texts.text(cell.text).is_empty() {
                continue;
            }
            let wide = row
                .get(column + 1)
                .is_some_and(|next| self.texts.text(next.text).is_empty());
            placed.push(PositionedCell {
                column: column as u16,
                columns: if wide { 2 } else { 1 },
                text: self.texts.text(cell.text).clone(),
                style: self.style_for(cell.hl, theme),
                selected: false,
                hovered_link: false,
            });
        }
        placed
    }

    /// The program's attrs for an id, with the theme's say over the group the
    /// id was named as, in the shape the painter reads for a terminal cell.
    fn style_for(&self, hl: u32, theme: &Highlights) -> CellStyle {
        let mut attrs = self.attrs.get(&hl).cloned().unwrap_or_default();
        // Every theme entry that names this id is applied, in the order the
        // names were received, so a later name layers over an earlier one for
        // the fields it sets and leaves the rest alone. Within one `highlights`
        // message that order is the JSON object's key order; across messages it
        // is the order the messages arrived.
        if let Some(names) = self.groups.get(&hl) {
            for style in names.values().filter_map(|name| theme.get(name)) {
                apply_theme(&mut attrs, style);
            }
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

    /// Whether this grid's cursor is asking the pane for a blink phase. A
    /// grid that fills its pane hides the terminal, so the pane's phase has
    /// to follow the grid's cursor or the grid's cursor never blinks.
    pub fn cursor_blinks(&self) -> bool {
        self.cursor.visible && self.cursor.blink
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

    /// The attr ids that currently have at least one group name, sorted.
    ///
    /// Test-only, and deliberately not part of the type's interface: nothing
    /// that draws needs the id list, only the names behind one id.
    #[cfg(test)]
    fn group_ids(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self.groups.keys().copied().collect();
        ids.sort_unstable();
        ids
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

    fn check_pool(grid: &GridSurface) {
        let mut refs = vec![0usize; grid.texts.entries.len()];
        refs[0] = 1;
        for row in &grid.cells {
            for cell in row {
                refs[cell.text as usize] += 1;
            }
        }
        for (id, entry) in grid.texts.entries.iter().enumerate() {
            assert_eq!(entry.as_ref().map_or(0, |entry| entry.references), refs[id]);
            if let Some(entry) = entry {
                assert_eq!(grid.texts.ids.get(entry.text.as_ref()), Some(&(id as u32)));
            }
        }
        assert!(grid.texts.entries.len() <= usize::from(grid.cols) * usize::from(grid.rows) + 2);
        let bound = usize::from(grid.cols) * usize::from(grid.rows) + 2;
        assert!(grid.texts.entries.capacity() <= bound.next_power_of_two().max(4));
        assert!(grid.texts.free.capacity() <= bound.next_power_of_two().max(4));
        assert!(grid.texts.ids.capacity() <= bound.next_power_of_two() * 2);
        assert_eq!(
            grid.texts.free.len() + grid.texts.ids.len(),
            grid.texts.entries.len()
        );
    }

    #[test]
    fn compact_cells_reclaim_unique_text_without_mutating_held_frames() {
        assert!(std::mem::size_of::<Cell>() <= 8);
        let mut grid = GridSurface::new(3, 2);
        let theme = Highlights::default();
        let mut held = Vec::new();
        for i in 0..10_000 {
            let text = format!("👩‍💻-{i}");
            grid.apply(Op::Rows(vec![RowChunk {
                row: 0,
                col: 0,
                cells: vec![(
                    WireCell {
                        text: text.clone(),
                        hl: i,
                    },
                    1,
                )],
            }]))
            .unwrap();
            let frame = grid.positioned_rows(&theme);
            if i % 1000 == 0 {
                held.push((frame, text));
            }
            check_pool(&grid);
            assert!(grid.texts.ids.len() <= 2);
        }
        for (frame, text) in held {
            assert_eq!(frame[0][0].text.as_str(), text);
        }
        grid.apply(Op::Clear).unwrap();
        check_pool(&grid);
        assert_eq!(grid.texts.ids.len(), 1);
        grid.apply(Op::Resize { cols: 1, rows: 1 }).unwrap();
        check_pool(&grid);
        assert_eq!(grid.texts.entries.len(), 1);
    }

    #[test]
    fn cached_rows_follow_actual_cell_and_theme_changes() {
        let mut grid = GridSurface::new(4, 3);
        let theme = Highlights::default();
        let before = grid.positioned_rows(&theme);
        grid.apply_all(ops(
            json!({"type":"rows","rows":[{"row":1,"cells":[["é", 2],["界"],[""]]}]}),
        ))
        .unwrap();
        let changed = grid.positioned_rows(&theme);
        assert!(Arc::ptr_eq(&before[0], &changed[0]));
        assert!(!Arc::ptr_eq(&before[1], &changed[1]));
        assert!(Arc::ptr_eq(&before[2], &changed[2]));
        assert_eq!(changed[1][0].text, "é");
        assert_eq!(changed[1][1].columns, 2);
        grid.apply_all(ops(
            json!({"type":"rows","rows":[{"row":1,"cells":[["é", 2],["界"],[""]]}]}),
        ))
        .unwrap();
        grid.apply_all(ops(json!({"type":"cursor","row":2,"col":3})))
            .unwrap();
        grid.apply(Op::Scroll {
            top: 0,
            bot: 3,
            left: 0,
            right: 4,
            rows: 0,
        })
        .unwrap();
        assert!(Arc::ptr_eq(&changed, &grid.positioned_rows(&theme)));
        let before_bad = grid.positioned_rows(&theme);
        assert!(grid.apply_all(ops(json!({"type":"rows","rows":[{"row":0,"cells":[["bad"]]},{"row":9,"cells":[["bad"]]}]}))).is_err());
        assert!(Arc::ptr_eq(&before_bad, &grid.positioned_rows(&theme)));
        grid.apply_all(ops(
            json!({"type":"highlights","define":{"2":{"bold":true}},"groups":{"Comment":2}}),
        ))
        .unwrap();
        let highlighted = grid.positioned_rows(&theme);
        assert!(highlighted[1][0].style.bold);
        assert!(!changed[1][0].style.bold);
        let theme = Highlights::from_groups(vec![(
            "Comment".into(),
            HighlightStyle {
                italic: Some(true),
                ..Default::default()
            },
        )]);
        let themed = grid.positioned_rows(&theme);
        assert!(themed[1][0].style.italic);
        assert!(!highlighted[1][0].style.italic);
        assert!(Arc::ptr_eq(&themed, &grid.positioned_rows(&theme)));
        grid.apply(Op::Resize { cols: 4, rows: 5 }).unwrap();
        let grown = grid.positioned_rows(&theme);
        for row in 0..3 {
            assert!(Arc::ptr_eq(&themed[row], &grown[row]));
        }
        grid.apply(Op::Resize { cols: 4, rows: 2 }).unwrap();
        let shrunk = grid.positioned_rows(&theme);
        for row in 0..2 {
            assert!(Arc::ptr_eq(&grown[row], &shrunk[row]));
        }
        check_pool(&grid);
    }

    #[test]
    fn generated_grid_transitions_match_a_frozen_copy_oracle() {
        for seed in 0..64u64 {
            let mut random = seed + 1;
            let mut next = || {
                random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                (random >> 32) as usize
            };
            let mut grid = GridSurface::new(7, 5);
            let mut expected = vec![vec![(" ".to_owned(), 0); 7]; 5];
            for step in 0..100 {
                let rows = expected.len();
                let cols = expected[0].len();
                match next() % 5 {
                    0 => {
                        let r = next() % rows;
                        let c = next() % cols;
                        let text = ["", "x", "界", "é", "👩‍💻"][next() % 5].to_owned();
                        let hl = next() as u32;
                        expected[r][c] = (text.clone(), hl);
                        grid.apply(Op::Rows(vec![RowChunk {
                            row: r as u16,
                            col: c as u16,
                            cells: vec![(WireCell { text, hl }, 1)],
                        }]))
                        .unwrap();
                    }
                    1 => {
                        let top = next() % rows;
                        let bot = top + 1 + next() % (rows - top);
                        let left = next() % cols;
                        let right = left + 1 + next() % (cols - left);
                        let distance = [i32::MIN, -3, -1, 0, 1, 3, i32::MAX][next() % 7];
                        let old = expected.clone();
                        for (r, row) in expected.iter_mut().enumerate().take(bot).skip(top) {
                            for (c, cell) in row.iter_mut().enumerate().take(right).skip(left) {
                                let source = r as i64 + i64::from(distance);
                                *cell = if source >= top as i64 && source < bot as i64 {
                                    old[source as usize][c].clone()
                                } else {
                                    (" ".into(), 0)
                                };
                            }
                        }
                        grid.apply(Op::Scroll {
                            top: top as u16,
                            bot: bot as u16,
                            left: left as u16,
                            right: right as u16,
                            rows: distance,
                        })
                        .unwrap();
                    }
                    2 => {
                        expected
                            .iter_mut()
                            .for_each(|row| row.fill((" ".into(), 0)));
                        grid.apply(Op::Clear).unwrap();
                    }
                    3 => {
                        let cols = 1 + next() % 9;
                        let rows = 1 + next() % 7;
                        for row in &mut expected {
                            row.resize(cols, (" ".into(), 0));
                        }
                        expected.resize(rows, vec![(" ".into(), 0); cols]);
                        grid.apply(Op::Resize {
                            cols: cols as u16,
                            rows: rows as u16,
                        })
                        .unwrap();
                    }
                    _ => {
                        let before = grid.cells.clone();
                        assert!(grid.apply(Op::Resize { cols: 0, rows: 1 }).is_err());
                        assert_eq!(grid.cells, before);
                    }
                }
                check_pool(&grid);
                for (r, row) in expected.iter().enumerate() {
                    for (c, (text, hl)) in row.iter().enumerate() {
                        let cell = grid.cells[r][c];
                        assert_eq!(
                            (grid.texts.text(cell.text).as_str(), cell.hl),
                            (text.as_str(), *hl),
                            "seed={seed} shortest failing prefix={}",
                            step + 1
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_refusal_inside_a_batch_names_the_operation_that_failed() {
        let mut grid = GridSurface::new(4, 2);
        let ops = parse_ops(&json!({ "type": "batch", "ops": [
            { "type": "clear" },
            { "type": "cursor", "row": 7, "col": 0 },
        ] }))
        .expect("parses");
        let refused = grid.apply_all(ops).expect_err("row 7 is outside");
        assert!(
            refused.reason().starts_with("malformed: op 1: "),
            "{}",
            refused.reason()
        );
        // A bare operation is refused without a prefix.
        let ops = parse_ops(&json!({ "type": "cursor", "row": 7, "col": 0 })).expect("parses");
        let refused = grid.apply_all(ops).expect_err("row 7 is outside");
        assert!(!refused.reason().contains("op "), "{}", refused.reason());
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

        let theme = Highlights::from_groups(vec![(
            "Comment".to_owned(),
            HighlightStyle {
                color: Some(unpack(0x00ff00)),
                bold: Some(true),
                italic: Some(false),
                underline: Some(UnderlineStyle::None),
                background: None,
            },
        )]);
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
    fn several_group_names_for_one_id_each_reach_the_theme_and_the_later_one_layers_over() {
        let mut grid = GridSurface::new(1, 1);
        // Two messages, so the order the names arrived in is the order written
        // here rather than the order a JSON object happens to enumerate.
        grid.apply_all(ops(json!({ "type": "batch", "ops": [
            { "type": "highlights", "define": { "1": { "italic": true } }, "groups": { "Comment": 1 } },
            { "type": "highlights", "groups": { "@comment.lua": 1 } },
            { "type": "rows", "rows": [{ "row": 0, "cells": [["a", 1]] }] }
        ] })))
        .expect("apply");

        let entry = |color: u32, bold: Option<bool>| HighlightStyle {
            color: Some(unpack(color)),
            bold,
            italic: None,
            underline: None,
            background: None,
        };
        // `Highlights::get` searches a sorted table, and '@' sorts before 'C'.
        let comment = ("Comment".to_owned(), entry(0x00ff00, Some(true)));
        let lua = ("@comment.lua".to_owned(), entry(0x0000ff, None));

        // A theme that names only the first of the two still reaches the cell.
        grid.invalidate();
        let styled =
            grid.positioned_rows(&Highlights::from_groups(vec![comment.clone()]))[0][0].style;
        assert_eq!(styled.foreground, SnapshotColor::Rgb(unpack(0x00ff00)));
        assert!(styled.bold);
        assert!(
            styled.italic,
            "the program's own italic stands where the theme says nothing"
        );

        // With both named, the later name wins the field they both set and
        // leaves the earlier one's other fields alone.
        grid.invalidate();
        let styled = grid.positioned_rows(&Highlights::from_groups(vec![lua, comment]))[0][0].style;
        assert_eq!(
            styled.foreground,
            SnapshotColor::Rgb(unpack(0x0000ff)),
            "@comment.lua was received second"
        );
        assert!(styled.bold, "and Comment's bold, which it does not set");
    }

    #[test]
    fn relinking_a_group_moves_its_name_to_the_new_id() {
        let mut grid = GridSurface::new(2, 1);
        let theme = Highlights::from_groups(vec![(
            "Comment".to_owned(),
            HighlightStyle {
                color: Some(Rgb {
                    r: 0xff,
                    g: 0,
                    b: 0,
                }),
                ..Default::default()
            },
        )]);
        grid.apply_all(
            parse_ops(
                &json!({ "type": "highlights", "define": { "1": {}, "2": {} }, "groups": { "Comment": 1 } }),
            )
            .expect("parses"),
        )
        .expect("applies");
        grid.apply_all(
            parse_ops(&json!({ "type": "highlights", "groups": { "Comment": 2 } }))
                .expect("parses"),
        )
        .expect("applies");
        assert_eq!(
            grid.group_ids(),
            vec![2],
            "id 1 kept no names, so it is not kept either"
        );
        grid.apply_all(
            parse_ops(
                &json!({ "type": "rows", "rows": [{ "row": 0, "cells": [["a", 1], ["b", 2]] }] }),
            )
            .expect("parses"),
        )
        .expect("applies");
        let row = &grid.positioned_rows(&theme)[0];
        assert_eq!(
            row[0].style.foreground,
            SnapshotColor::Default,
            "id 1 is no longer Comment"
        );
        assert_eq!(
            row[1].style.foreground,
            SnapshotColor::Rgb(Rgb {
                r: 0xff,
                g: 0,
                b: 0
            })
        );
    }

    /// The name index and the per-id lists describe the same thing: every
    /// name sits under exactly one id, at the stamp the index says, and no
    /// id is kept with no names.
    fn check_groups(grid: &GridSurface) {
        let mut seen = 0;
        for (id, names) in &grid.groups {
            assert!(!names.is_empty(), "id {id} is kept with no names");
            for (stamp, name) in names {
                assert_eq!(
                    grid.group_of.get(name),
                    Some(&(*id, *stamp)),
                    "{name} sits under id {id} but the index disagrees"
                );
                seen += 1;
            }
        }
        assert_eq!(
            seen,
            grid.group_of.len(),
            "the index names a group no id holds"
        );
    }

    #[test]
    fn the_name_index_follows_every_relink_and_a_moved_name_leaves_its_old_id() {
        let mut grid = GridSurface::new(1, 1);
        let names = |grid: &GridSurface, id: u32| -> Vec<String> {
            grid.groups[&id].values().cloned().collect()
        };
        grid.apply_all(ops(json!({ "type": "highlights",
            "define": { "1": {}, "2": {} },
            "groups": { "Comment": 1, "@comment": 1, "String": 2 } })))
            .expect("applies");
        check_groups(&grid);
        assert_eq!(names(&grid, 1), ["Comment", "@comment"]);

        grid.apply_all(ops(
            json!({ "type": "highlights", "groups": { "Comment": 2 } }),
        ))
        .expect("applies");
        check_groups(&grid);
        assert_eq!(names(&grid, 1), ["@comment"], "Comment left id 1");
        assert_eq!(names(&grid, 2), ["String", "Comment"]);

        // Naming an id it already has moves the name last, so it layers over
        // the others exactly as a fresh name would.
        grid.apply_all(ops(
            json!({ "type": "highlights", "groups": { "String": 2 } }),
        ))
        .expect("applies");
        check_groups(&grid);
        assert_eq!(names(&grid, 2), ["Comment", "String"]);

        grid.apply_all(ops(
            json!({ "type": "highlights", "groups": { "@comment": 2 } }),
        ))
        .expect("applies");
        check_groups(&grid);
        assert_eq!(
            grid.group_ids(),
            vec![2],
            "id 1 kept no names, so it is not kept"
        );
    }

    #[test]
    fn a_relink_touches_one_stored_entry_however_many_the_grid_holds() {
        let mut grid = GridSurface::new(1, 1);
        let names = |offset: u32| -> Vec<(String, u32)> {
            (0..50_000u32)
                .map(|n| (format!("group-{n}"), n + offset))
                .collect()
        };
        grid.apply(Op::Highlights {
            define: Vec::new(),
            groups: names(1),
        })
        .expect("applies");
        RELINKED_ENTRIES.with(|count| count.set(0));
        grid.apply(Op::Highlights {
            define: Vec::new(),
            groups: names(2),
        })
        .expect("applies");
        assert_eq!(
            RELINKED_ENTRIES.with(|count| count.get()),
            50_000,
            "each relink should touch only the entry it moves"
        );
        check_groups(&grid);
        assert_eq!(grid.group_ids().len(), 50_000);
    }

    /// A coarse guard against a quadratic relink, not a benchmark: the bound
    /// is generous enough for a debug build on a loaded machine, and the work
    /// runs on its own thread so a regression fails here instead of hanging.
    #[test]
    fn relinking_fifty_thousand_names_finishes_well_inside_a_generous_bound() {
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut grid = GridSurface::new(1, 1);
            for offset in [1, 2] {
                grid.apply(Op::Highlights {
                    define: Vec::new(),
                    groups: (0..50_000u32)
                        .map(|n| (format!("group-{n}"), n + offset))
                        .collect(),
                })
                .expect("applies");
            }
            let _ = done.send(grid.group_ids().len());
        });
        assert_eq!(
            finished.recv_timeout(std::time::Duration::from_secs(30)),
            Ok(50_000),
            "relinking 50k names took more than 30 s, or panicked"
        );
    }

    #[test]
    fn a_highlights_operation_past_either_cap_is_refused_before_it_changes_anything() {
        let mut grid = GridSurface::new(2, 1);
        grid.apply(Op::Highlights {
            define: (1..=MAX_HIGHLIGHT_IDS as u32)
                .map(|id| (id, Attrs::default()))
                .collect(),
            groups: Vec::new(),
        })
        .expect("exactly the id cap fits");
        grid.apply(Op::Highlights {
            define: Vec::new(),
            groups: (0..MAX_GROUP_NAMES).map(|n| (format!("g{n}"), 1)).collect(),
        })
        .expect("exactly the name cap fits");

        // On its own, an over-cap operation changes nothing, and a bare
        // operation's refusal carries no prefix.
        let before = grid.clone();
        for message in [
            json!({ "type": "highlights", "define": { "1": { "bold": true }, "262145": {} } }),
            json!({ "type": "highlights", "define": { "1": { "bold": true } },
                    "groups": { "g0": 2, "OneMore": 1 } }),
        ] {
            let refusal = grid
                .apply_all(ops(message.clone()))
                .expect_err("over a cap");
            assert!(
                matches!(&refusal, Refusal::Malformed(why) if !why.starts_with("op ")),
                "{message} was refused as {refusal:?}"
            );
            assert!(grid == before, "{message} changed the grid");
        }

        // Inside a batch it is refused like any other bad operation: the
        // operations before it stand, it changes nothing itself — not even
        // the id it would have redefined — and the reason names it.
        let refusal = grid
            .apply_all(ops(json!({ "type": "batch", "ops": [
                { "type": "rows", "rows": [{ "row": 0, "cells": [["x", 1]] }] },
                { "type": "highlights", "define": { "1": { "bold": true }, "262145": {} } }
            ] })))
            .expect_err("op 1 is over the id cap");
        assert!(
            refusal.reason().starts_with("malformed: op 1: "),
            "{}",
            refusal.reason()
        );
        assert_eq!(
            grid.texts.text(grid.cells[0][0].text).as_str(),
            "x",
            "op 0 stood"
        );
        assert!(
            grid.attrs == before.attrs,
            "the over-cap operation redefined an id"
        );

        let refusal = grid
            .apply_all(ops(json!({ "type": "batch", "ops": [
                { "type": "cursor", "row": 0, "col": 1 },
                { "type": "highlights", "groups": { "g0": 2, "OneMore": 1 } }
            ] })))
            .expect_err("op 1 is over the name cap");
        assert!(
            refusal.reason().starts_with("malformed: op 1: "),
            "{}",
            refusal.reason()
        );
        assert_eq!(grid.cursor.col, 1, "op 0 stood");
        assert!(
            grid.groups == before.groups && grid.group_of == before.group_of,
            "the over-cap operation relinked a name"
        );
        check_groups(&grid);

        // At the caps, redefining an id and relinking a name grow nothing,
        // so they still apply.
        grid.apply_all(ops(json!({ "type": "highlights",
            "define": { "1": { "bold": true } }, "groups": { "g0": 2 } })))
            .expect("no growth");
        check_groups(&grid);
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
    fn only_a_visible_blinking_cursor_asks_the_pane_for_a_blink_phase() {
        let mut grid = GridSurface::new(2, 1);
        assert!(
            !grid.cursor_blinks(),
            "a fresh grid's cursor is visible but steady"
        );
        grid.apply_all(ops(
            json!({ "type": "cursor", "row": 0, "col": 0, "blink": true, "visible": false }),
        ))
        .expect("apply");
        assert!(!grid.cursor_blinks(), "a hidden cursor blinks nothing");
        grid.apply_all(ops(
            json!({ "type": "cursor", "row": 0, "col": 0, "visible": true }),
        ))
        .expect("apply");
        assert!(grid.cursor_blinks());
        grid.apply_all(ops(
            json!({ "type": "cursor", "row": 0, "col": 0, "blink": false }),
        ))
        .expect("apply");
        assert!(!grid.cursor_blinks(), "visible but steady");
    }

    #[test]
    fn a_cursor_outside_the_grid_is_refused_and_the_cursor_stays_put() {
        let mut grid = GridSurface::new(4, 2);
        grid.apply_all(ops(
            json!({ "type": "cursor", "row": 1, "col": 3, "shape": "bar" }),
        ))
        .expect("apply");
        for message in [
            json!({ "type": "cursor", "row": 2, "col": 0 }),
            json!({ "type": "cursor", "row": 0, "col": 4 }),
        ] {
            let refusal = grid.apply_all(ops(message.clone()));
            assert!(
                matches!(&refusal, Err(Refusal::Malformed(why)) if why.contains("outside the grid")),
                "{message}: {refusal:?}"
            );
        }
        let cursor = grid.cursor_snapshot();
        assert_eq!((cursor.row, cursor.column), (1, 3));
        assert_eq!(cursor.style, CursorStyle::Bar);
    }

    #[test]
    fn a_cell_whose_highlight_was_never_defined_takes_the_grids_default_style() {
        let mut grid = GridSurface::new(2, 1);
        grid.apply_all(ops(json!({ "type": "batch", "ops": [
            { "type": "highlights", "define": { "1": { "bold": true } } },
            { "type": "rows", "rows": [{ "row": 0, "cells": [["a", 1], ["b", 99]] }] }
        ] })))
        .expect("apply");
        let rows = grid.positioned_rows(&Highlights::default());
        // Every field is Default or off: the painter fills the colours from
        // `default_colors`, exactly as it does for an unstyled terminal cell.
        assert_eq!(
            rows[0][1].style,
            CellStyle {
                foreground: SnapshotColor::Default,
                background: SnapshotColor::Default,
                underline_color: SnapshotColor::Default,
                bold: false,
                italic: false,
                faint: false,
                blink: false,
                inverse: false,
                invisible: false,
                strikethrough: false,
                overline: false,
                underline: UnderlineStyle::None,
            },
            "an undefined id is not the id before it, and not bold"
        );
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
