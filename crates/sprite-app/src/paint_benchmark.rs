//! Headless access to the terminal's live row preparation and cell decisions.
//! Window layout, image elements, glyph rasterisation and GPU submission are
//! excluded. Shaping is counted through the live shape cache but not timed:
//! GPUI shapes text only through a window, and none can be opened headless.

use gpui::px;
use sprite_term::{
    CellStyle, CellWidth, CursorSnapshot, CursorStyle, HyperlinkSpan, RenderCell, RenderRow,
    RenderSnapshot, Rgb, SnapshotColor, TerminalSize, UnderlineStyle, Viewport,
};

use crate::grid::prepare_rows;
use crate::grid_paint::{GridPaint, GridPaintSpec, RowPass};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scenario {
    FirstFrame,
    SameGenerationBlink,
    Hover,
    OneRowChange,
}

impl Scenario {
    pub const ALL: [Self; 4] = [
        Self::FirstFrame,
        Self::SameGenerationBlink,
        Self::Hover,
        Self::OneRowChange,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::FirstFrame => "first_frame",
            Self::SameGenerationBlink => "same_generation_blink",
            Self::Hover => "hover",
            Self::OneRowChange => "one_row_change",
        }
    }
}

/// What one transition asks of the text system.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ShapingSample {
    /// Cells whose text reaches the text system: every one of them was shaped
    /// on every frame before the shape cache existed.
    pub glyph_cells: usize,
    /// Of those, how many the shape cache had to shape.
    pub shape_calls: usize,
}

/// The display scale shaping is counted at; any fixed value will do, since a
/// sample never changes it.
const BENCHMARK_SCALE: f32 = 2.0;

pub struct PaintBenchmark {
    snapshot: RenderSnapshot,
    changed: RenderSnapshot,
    cache: crate::grid::LayoutCache,
    shapes: std::rc::Rc<std::cell::RefCell<crate::grid_paint::ShapeCache>>,
}

impl Default for PaintBenchmark {
    fn default() -> Self {
        Self::new()
    }
}

impl PaintBenchmark {
    /// Fixture creation is outside the allocation and timing measurement.
    pub fn new() -> Self {
        let snapshot = fixture();
        let mut changed = snapshot.clone();
        changed.generation += 1;
        std::sync::Arc::make_mut(&mut changed.rows[30]).cells[10].text = "Z".into();
        Self {
            snapshot,
            changed,
            cache: Default::default(),
            shapes: Default::default(),
        }
    }

    pub fn run(&mut self, scenario: Scenario, split: bool) {
        let (background, text) = self.prepare(scenario, split);
        background.benchmark_draw_decisions();
        if let Some(text) = &text {
            text.benchmark_draw_decisions();
        }
        std::hint::black_box((background, text));
    }

    /// Counts, for one transition, the glyphs live painting hands to the text
    /// system and how many of them the shape cache has to shape.
    pub fn shaping(&mut self, scenario: Scenario, split: bool) -> ShapingSample {
        let (background, text) = self.prepare(scenario, split);
        let pass = text.as_ref().unwrap_or(&background);
        let (glyph_cells, shape_calls) = pass.benchmark_shaping(BENCHMARK_SCALE);
        ShapingSample {
            glyph_cells,
            shape_calls,
        }
    }

    pub(crate) fn prepare(
        &mut self,
        scenario: Scenario,
        split: bool,
    ) -> (GridPaint, Option<GridPaint>) {
        let snapshot = match scenario {
            Scenario::OneRowChange => &self.changed,
            _ => &self.snapshot,
        };
        let hover = (scenario == Scenario::Hover).then_some((
            snapshot.generation,
            HyperlinkSpan {
                row: 12,
                start_column: 20,
                end_column: 48,
            },
        ));
        let rows = prepare_rows(&mut self.cache, Some(snapshot), hover);
        GridPaint::prepare_spec(
            GridPaintSpec {
                rows,
                pass: RowPass::Whole,
                cursor: Some(snapshot.cursor)
                    .filter(|cursor| scenario != Scenario::SameGenerationBlink || !cursor.blinking),
                cursor_color: snapshot.cursor_color,
                palette: Some(snapshot.palette.clone()),
                default_fg: snapshot.default_foreground,
                default_bg: snapshot.default_background,
                cell_width: px(8.4),
                cell_height: px(18.0),
                font_family: "monospace".into(),
                font_size: px(14.0),
                shapes: std::rc::Rc::clone(&self.shapes),
                focused: true,
            },
            split,
        )
    }
}

pub(crate) fn fixture() -> RenderSnapshot {
    let rows = (0..60)
        .map(|row| {
            std::sync::Arc::new(RenderRow {
                cells: (0..200)
                    .map(|column| {
                        let (text, width) = match column % 20 {
                            0..=5 => ("a", CellWidth::Narrow),
                            6..=11 => (" ", CellWidth::Narrow),
                            12 => ("é", CellWidth::Narrow),
                            13 => ("e\u{301}", CellWidth::Narrow),
                            14 => ("界", CellWidth::Wide),
                            15 => ("", CellWidth::SpacerTail),
                            16 => ("│", CellWidth::Narrow),
                            17 => ("█", CellWidth::Narrow),
                            _ => ("x", CellWidth::Narrow),
                        };
                        RenderCell {
                            text: text.into(),
                            width,
                            style: CellStyle {
                                foreground: SnapshotColor::Palette((row + column) as u8),
                                background: if column % 7 == 0 {
                                    SnapshotColor::Palette(4)
                                } else {
                                    SnapshotColor::Default
                                },
                                underline_color: SnapshotColor::Default,
                                bold: column % 11 == 0,
                                italic: column % 13 == 0,
                                faint: column % 17 == 0,
                                blink: false,
                                inverse: column % 19 == 0,
                                invisible: false,
                                strikethrough: column % 23 == 0,
                                overline: false,
                                underline: if column % 5 == 0 {
                                    UnderlineStyle::Single
                                } else {
                                    UnderlineStyle::None
                                },
                            },
                            selected: (20..=22).contains(&row) && (10..80).contains(&column),
                        }
                    })
                    .collect(),
                wrapped: false,
            })
        })
        .collect();
    RenderSnapshot {
        generation: 1,
        size: sprite_term::ValidTerminalSize::new(
            TerminalSize {
                rows: 60,
                cols: 200,
                cell_width_px: 8,
                cell_height_px: 18,
            },
            "resize",
        )
        .expect("valid terminal size"),
        viewport: Viewport {
            total_rows: 60,
            offset: 0,
            visible_rows: 60,
        },
        mouse_tracking: false,
        rows,
        cursor: CursorSnapshot {
            row: 21,
            column: 14,
            visible: true,
            blinking: true,
            style: CursorStyle::Block,
        },
        default_foreground: Rgb {
            r: 220,
            g: 220,
            b: 220,
        },
        default_background: Rgb {
            r: 20,
            g: 20,
            b: 20,
        },
        palette: std::sync::Arc::new(std::array::from_fn(|index| Rgb {
            r: index as u8,
            g: (index as u8).wrapping_mul(3),
            b: (index as u8).wrapping_mul(7),
        })),
        cursor_color: Some(Rgb {
            r: 240,
            g: 210,
            b: 80,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_covers_text_width_selection_palette_and_cursor() {
        let benchmark = PaintBenchmark::new();
        let snapshot = &benchmark.snapshot;
        assert_eq!(snapshot.rows.len(), 60);
        assert!(snapshot.rows.iter().all(|row| row.cells.len() == 200));
        let cells = &snapshot.rows[21].cells;
        assert_eq!(cells[0].text, "a");
        assert_eq!(cells[6].text, " ");
        assert_eq!(cells[12].text, "é");
        assert_eq!(cells[13].text, "e\u{301}");
        assert_eq!(cells[14].width, CellWidth::Wide);
        assert_eq!(cells[15].width, CellWidth::SpacerTail);
        assert!(cells[14].selected);
        assert!(matches!(
            cells[14].style.foreground,
            SnapshotColor::Palette(_)
        ));
        assert!(snapshot.cursor.visible && snapshot.cursor.blinking);
        assert_eq!(benchmark.changed.generation, snapshot.generation + 1);
        let changed_rows: Vec<_> = snapshot
            .rows
            .iter()
            .zip(&benchmark.changed.rows)
            .enumerate()
            .filter_map(|(index, (before, after))| (before != after).then_some(index))
            .collect();
        assert_eq!(changed_rows, [30]);
    }

    /// Each transition, counted through the live shape cache. The fixture has
    /// 110 cells per row that reach the text system (6,600 on screen); before
    /// the cache every one of them was shaped on every frame.
    #[test]
    fn shaping_counts_follow_the_shape_cache_through_each_transition() {
        for split in [false, true] {
            let mut benchmark = PaintBenchmark::new();
            let first = benchmark.shaping(Scenario::FirstFrame, split);
            assert_eq!(first.glyph_cells, 6_600);
            assert!(first.shape_calls > 0 && first.shape_calls <= first.glyph_cells);
            assert_eq!(
                benchmark.shaping(Scenario::FirstFrame, split),
                ShapingSample {
                    glyph_cells: 6_600,
                    shape_calls: 0
                },
                "an unchanged frame shapes nothing"
            );
            let blink = benchmark.shaping(Scenario::SameGenerationBlink, split);
            assert!(
                blink.shape_calls <= 1,
                "a blink shapes at most the cursor's cell"
            );
            let hover = benchmark.shaping(Scenario::Hover, split);
            assert!(
                (1..=110).contains(&hover.shape_calls),
                "hover reshapes only within row 12: {}",
                hover.shape_calls
            );
            let changed = benchmark.shaping(Scenario::OneRowChange, split);
            assert_eq!(changed.glyph_cells, 6_601, "row 30 gained a glyph");
            assert!(
                (1..=111).contains(&changed.shape_calls),
                "a one-row change reshapes only row 30: {}",
                changed.shape_calls
            );
        }
    }
}
