use super::*;
use crate::tokens::unpack;

/// A view that paints one fixture snapshot through the live painter, so a
/// test can count what a real frame asks the text system for.
struct ShapeProbe {
    snapshot: RenderSnapshot,
    layout: crate::grid::LayoutCache,
    shapes: Rc<RefCell<ShapeCache>>,
    blink_on: bool,
    font_size: Pixels,
}

impl gpui::Render for ShapeProbe {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        use gpui::{ParentElement, Styled};
        let rows = crate::grid::prepare_rows(&mut self.layout, Some(&self.snapshot), None);
        let cursor = Some(self.snapshot.cursor).filter(|cursor| self.blink_on || !cursor.blinking);
        let (paint, _) = GridPaint::prepare_spec(
            GridPaintSpec {
                rows,
                pass: RowPass::Whole,
                cursor,
                cursor_color: self.snapshot.cursor_color,
                default_fg: self.snapshot.default_foreground,
                default_bg: self.snapshot.default_background,
                palette: Some(Arc::clone(&self.snapshot.palette)),
                cell_width: px(8.4),
                cell_height: px(18.0),
                font_family: "monospace".into(),
                font_size: self.font_size,
                focused: true,
                shapes: Rc::clone(&self.shapes),
            },
            false,
        );
        gpui::div().size_full().child(paint)
    }
}

/// Asks the cache for one cell and reports whether it had to shape.
fn shapes_anew(cache: &mut ShapeCache, cells: &Arc<Vec<PositionedCell>>, color: Rgba) -> bool {
    let mut shaped = false;
    let _ = cache.shaped(0, cells, 0, color, || {
        shaped = true;
        ShapedLine::default()
    });
    shaped
}

#[test]
fn benchmark_samples_prepare_real_blink_hover_and_one_row_transitions() {
    use crate::paint_benchmark::{PaintBenchmark, Scenario};

    for split in [false, true] {
        let mut benchmark = PaintBenchmark::new();
        let (first, first_text) = benchmark.prepare(Scenario::FirstFrame, split);
        assert!(first.cursor.is_some());
        assert_eq!(first_text.is_some(), split);
        let (blink, _) = benchmark.prepare(Scenario::SameGenerationBlink, split);
        assert!(blink.cursor.is_none());
        assert_eq!(first.rows, blink.rows);
        assert!(Arc::ptr_eq(&first.rows, &blink.rows));
        assert!(Arc::ptr_eq(
            first.palette.as_ref().unwrap(),
            blink.palette.as_ref().unwrap()
        ));
        if let Some(text) = &first_text {
            assert!(Arc::ptr_eq(&first.rows, &text.rows));
        }
        let (hover, _) = benchmark.prepare(Scenario::Hover, split);
        assert!(!Arc::ptr_eq(&first.rows[12], &hover.rows[12]));
        assert!(
            first
                .rows
                .iter()
                .zip(hover.rows.iter())
                .enumerate()
                .all(|(index, (before, after))| index == 12 || Arc::ptr_eq(before, after))
        );
        assert!(hover.rows[12].iter().any(|cell| cell.hovered_link));
        assert!(
            hover
                .rows
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != 12)
                .all(|(_, row)| row.iter().all(|cell| !cell.hovered_link))
        );
        let (unhover, _) = benchmark.prepare(Scenario::FirstFrame, split);
        assert_eq!(first.rows, unhover.rows);
        let (changed, _) = benchmark.prepare(Scenario::OneRowChange, split);
        let changed_rows: Vec<_> = first
            .rows
            .iter()
            .zip(changed.rows.iter())
            .enumerate()
            .filter_map(|(index, (before, after))| (before != after).then_some(index))
            .collect();
        assert_eq!(changed_rows, [30]);
        assert!(
            unhover
                .rows
                .iter()
                .zip(changed.rows.iter())
                .enumerate()
                .all(|(index, (before, after))| Arc::ptr_eq(before, after) == (index != 30))
        );
        assert_eq!(changed.rows[30][10].text, "Z");
        let (fresh, _) = PaintBenchmark::new().prepare(Scenario::FirstFrame, split);
        assert_eq!(first.rows, fresh.rows);
        assert_eq!(first.cursor, fresh.cursor);
    }
}

/// Preparing a frame copies no cell text: a laid-out cell keeps the
/// terminal's compact text, and the text system's string is made only
/// when a shape is. A quarter of the benchmark fixture's cells are
/// non-ASCII, and none of them may cost an allocation, so a frame costs
/// only the rows it lays out: each row's vector and handle, plus the
/// frame's row list.
#[test]
fn preparing_a_frame_allocates_nothing_per_cell_text() {
    use crate::paint_benchmark::{PaintBenchmark, Scenario};
    use crate::surface_performance::measure;

    for split in [false, true] {
        let mut fresh = PaintBenchmark::new();
        let ((), first, _) = measure(|| fresh.run(Scenario::FirstFrame, split));
        assert!(
            first <= 122,
            "first frame (split {split}) allocated {first} times"
        );
        for scenario in [Scenario::Hover, Scenario::OneRowChange] {
            let mut benchmark = PaintBenchmark::new();
            benchmark.run(Scenario::FirstFrame, split);
            let ((), allocations, _) = measure(|| benchmark.run(scenario, split));
            assert!(
                allocations <= 3,
                "{scenario:?} (split {split}) allocated {allocations} times for one row"
            );
        }
    }
}

#[test]
fn image_placeholders_leave_no_glyph_under_transparent_pixels() {
    assert!(blank_glyph("\u{10eeee}\u{0305}\u{030d}"));
    assert!(blank_glyph(" "));
    assert!(!blank_glyph("file.lua"));
}

#[test]
fn snapping_lands_on_whole_device_pixels() {
    // The case from the bug: a 8.4px cell on a 2x display.
    for column in 0..40 {
        let edge = snap(px(12.7 + column as f32 * 8.4), 2.0);
        let device = f32::from(edge) * 2.0;
        assert!(
            (device - device.round()).abs() < 1e-3,
            "column {column} landed at {device} device pixels"
        );
    }
}

#[test]
fn snapped_cells_tile_without_a_gap() {
    // The property the old per-cell layout lost: laid end to end, the cells
    // cover every pixel from the first edge to the last, with none counted
    // twice and none left out. Walking the row cell by cell must arrive
    // where measuring the whole row at once does.
    let edge = |column: u32| snap(px(12.7 + column as f32 * 8.4), 2.0);
    let mut walked = edge(0);
    for column in 0..109u32 {
        let right = edge(column + 1);
        assert_eq!(
            walked,
            edge(column),
            "column {column} did not start where its neighbour ended"
        );
        walked = right;
    }
    assert_eq!(walked, edge(109));
}

#[test]
fn snapping_never_collapses_a_cell() {
    // A cell at least one device pixel wide keeps at least one device
    // pixel: a snapped grid must not swallow a column.
    for column in 0..200u32 {
        let left = snap(px(12.7 + column as f32 * 8.4), 2.0);
        let right = snap(px(12.7 + (column + 1) as f32 * 8.4), 2.0);
        assert!(right > left, "column {column} was snapped away");
    }
}

#[test]
fn a_degenerate_scale_leaves_coordinates_alone() {
    assert_eq!(snap(px(10.3), 0.0), px(10.3));
    assert_eq!(snap(px(10.3), f32::NAN), px(10.3));
}

/// One eighth of a small cell is thinner than a device pixel, and snapping
/// both its edges to the same pixel would erase it. A block the terminal
/// asked for has to leave a mark, so the thinnest one is a single pixel
/// rather than nothing.
#[test]
fn a_block_thinner_than_a_device_pixel_still_leaves_a_mark() {
    // An eighth of a 4px cell: 0.5 device pixels at scale 2.
    let (left, right) = snapped_span(10.0, 10.25, 2.0);
    assert!(right > left, "the block was snapped out of existence");
    assert_eq!(right, px(10.5), "a vanishing block should take one pixel");
}

/// An empty span is empty on purpose and must not be inflated into a mark.
#[test]
fn an_empty_span_stays_empty() {
    let (left, right) = snapped_span(10.0, 10.0, 2.0);
    assert_eq!(left, right);
}

/// A light rule is weighed against the narrow side of the cell. Keying it
/// to the tall side instead draws every box on screen at twice the weight
/// of the text inside it.
#[test]
fn a_light_stroke_is_weighed_against_the_narrow_side_of_the_cell() {
    // The 8.4 x 16.8 logical cell a 14pt JetBrains Mono gives, at 2x.
    let strokes = stroke_widths(px(8.4), px(16.8), 2.0);
    assert_eq!(
        strokes.light * 2.0,
        2.0,
        "a light rule should be two device pixels here, not four"
    );
    assert_eq!(strokes.heavy, strokes.light * 2.0);
}

/// Every stroke lands on whole device pixels, and none of them vanishes at
/// a tiny cell or a degenerate scale.
#[test]
fn a_stroke_is_always_a_whole_number_of_device_pixels_and_never_zero() {
    for (w, h, scale) in [
        (8.4, 16.8, 2.0),
        (6.0, 12.0, 1.0),
        (3.0, 4.0, 1.0),
        (0.5, 0.5, 1.0),
        (8.4, 16.8, 0.0),
    ] {
        let strokes = stroke_widths(px(w), px(h), scale);
        let device = if scale > 0.0 { scale } else { 1.0 };
        let in_pixels = strokes.light * device;
        assert!(in_pixels >= 1.0, "light vanished at {w}x{h}@{scale}");
        assert!(
            (in_pixels - in_pixels.round()).abs() < 1e-4,
            "light was {in_pixels} device pixels at {w}x{h}@{scale}"
        );
    }
}

/// A style with no colour of its own and a cell style carrying every other
/// field at its quietest setting.
fn plain_style(foreground: SnapshotColor, background: SnapshotColor, inverse: bool) -> CellStyle {
    CellStyle {
        foreground,
        background,
        underline_color: SnapshotColor::Default,
        bold: false,
        italic: false,
        faint: false,
        blink: false,
        inverse,
        invisible: false,
        strikethrough: false,
        overline: false,
        underline: UnderlineStyle::None,
    }
}

/// A painter over no rows, with the defaults the colour tests use.
fn painter(pass: RowPass) -> GridPaint {
    GridPaint::new(GridPaintSpec {
        rows: Arc::from([]),
        pass,
        cursor: None,
        cursor_color: None,
        default_fg: unpack(0xaabbcc),
        default_bg: unpack(0x112233),
        palette: None,
        cell_width: px(8.4),
        cell_height: px(16.8),
        font_family: ".SystemUIFont".into(),
        font_size: px(14.0),
        focused: true,
        shapes: Default::default(),
    })
}

fn positioned(style: CellStyle) -> PositionedCell {
    PositionedCell {
        column: 0,
        columns: 1,
        text: "x".into(),
        style,
        selected: false,
        hovered_link: false,
    }
}

/// Colour resolution is arithmetic, not painting: it needs no Window.
#[test]
fn a_cell_with_no_opinion_takes_the_defaults() {
    let default_fg = Rgb {
        r: 0xaa,
        g: 0xbb,
        b: 0xcc,
    };
    let default_bg = Rgb {
        r: 0x11,
        g: 0x22,
        b: 0x33,
    };
    let style = plain_style(SnapshotColor::Default, SnapshotColor::Default, false);
    let (foreground, background) = cell_colors(&style, default_fg, default_bg, None);
    assert_eq!(foreground, rgb(pack(default_fg)));
    assert_eq!(background, rgb(pack(default_bg)));
}

/// Reverse video swaps them, which is the one rule worth pinning.
#[test]
fn reverse_video_swaps_foreground_and_background() {
    let default_fg = Rgb {
        r: 0xaa,
        g: 0xbb,
        b: 0xcc,
    };
    let default_bg = Rgb {
        r: 0x11,
        g: 0x22,
        b: 0x33,
    };
    let style = plain_style(SnapshotColor::Default, SnapshotColor::Default, true);
    let (foreground, background) = cell_colors(&style, default_fg, default_bg, None);
    assert_eq!(foreground, rgb(pack(default_bg)));
    assert_eq!(background, rgb(pack(default_fg)));
}

/// An invisible cell must vanish into its ground, not just match itself:
/// the glyph has to take on the ground's colour, so a bug that collapsed
/// the pair the other way round would still leave text visible.
#[test]
fn invisible_collapses_the_foreground_onto_the_background() {
    let fg_color = unpack(0x102030);
    let bg_color = unpack(0x405060);
    let mut style = plain_style(
        SnapshotColor::Rgb(fg_color),
        SnapshotColor::Rgb(bg_color),
        false,
    );
    style.invisible = true;
    let drawn = painter(RowPass::Whole).draw(&positioned(style), None);
    assert_eq!(Some(drawn.foreground), drawn.background);
    assert_eq!(
        drawn.foreground,
        rgb(pack(bg_color)),
        "invisible should collapse toward the background, not the foreground"
    );
}

/// A palette index is looked up in the supplied palette rather than
/// ignored: the chosen index's entry has to differ from both defaults, so
/// an implementation that fell back to a default (or read the wrong
/// slot) would be caught rather than accidentally matching by luck.
#[test]
fn a_palette_index_resolves_through_the_supplied_palette() {
    let default_fg = Rgb {
        r: 0xaa,
        g: 0xbb,
        b: 0xcc,
    };
    let default_bg = Rgb {
        r: 0x11,
        g: 0x22,
        b: 0x33,
    };
    // Every slot gets a distinct colour derived from its own index, so a
    // lookup that landed on the wrong slot (off by one, or any other
    // slot) would read back a different, and therefore wrong, colour.
    let palette: [Rgb; 256] = std::array::from_fn(|i| Rgb {
        r: i as u8,
        g: i as u8,
        b: i as u8,
    });
    let style = plain_style(SnapshotColor::Palette(42), SnapshotColor::Default, false);
    let (foreground, _background) = cell_colors(&style, default_fg, default_bg, Some(&palette));
    assert_eq!(
        foreground,
        rgb(pack(Rgb {
            r: 42,
            g: 42,
            b: 42
        }))
    );
}

/// An explicit RGB colour is not a default and not a palette index: it
/// must reach the drawn cell unchanged.
#[test]
fn an_explicit_rgb_colour_passes_through_unchanged() {
    let default_fg = Rgb {
        r: 0xaa,
        g: 0xbb,
        b: 0xcc,
    };
    let default_bg = Rgb {
        r: 0x11,
        g: 0x22,
        b: 0x33,
    };
    let fg_color = Rgb {
        r: 0x01,
        g: 0x02,
        b: 0x03,
    };
    let bg_color = Rgb {
        r: 0xfd,
        g: 0xfe,
        b: 0xff,
    };
    let style = plain_style(
        SnapshotColor::Rgb(fg_color),
        SnapshotColor::Rgb(bg_color),
        false,
    );
    let (foreground, background) = cell_colors(&style, default_fg, default_bg, None);
    assert_eq!(foreground, rgb(pack(fg_color)));
    assert_eq!(background, rgb(pack(bg_color)));
}

/// Inverse swaps first and hiding acts on the result, so a reversed hidden
/// cell settles on its original foreground, which is the ground it shows.
#[test]
fn inverse_and_invisible_together_collapse_onto_the_original_foreground() {
    let fg_color = unpack(0x102030);
    let bg_color = unpack(0x405060);
    let mut style = plain_style(
        SnapshotColor::Rgb(fg_color),
        SnapshotColor::Rgb(bg_color),
        true,
    );
    style.invisible = true;
    let drawn = painter(RowPass::Whole).draw(&positioned(style), None);
    assert_eq!(Some(drawn.foreground), drawn.background);
    assert_eq!(
        drawn.foreground,
        rgb(pack(fg_color)),
        "reversed and invisible together should settle on the pre-swap foreground"
    );
}

fn decorated(underline: UnderlineStyle, strikethrough: bool) -> CellStyle {
    CellStyle {
        underline,
        strikethrough,
        ..plain_style(SnapshotColor::Default, SnapshotColor::Default, false)
    }
}

#[test]
fn drawing_prepares_decorations_for_whitespace_without_glyph_ink() {
    let paint = GridPaint::new(GridPaintSpec {
        rows: Arc::from([]),
        pass: RowPass::Whole,
        cursor: None,
        cursor_color: None,
        default_fg: unpack(0xffffff),
        default_bg: unpack(0x112233),
        palette: None,
        cell_width: px(8.4),
        cell_height: px(16.8),
        font_family: ".SystemUIFont".into(),
        font_size: px(14.0),
        shapes: Default::default(),
        focused: true,
    });
    for text in ["", " ", "\t", "\u{3000}", "\u{10eeee}"] {
        let mut cell = PositionedCell {
            column: 0,
            columns: 1,
            text: text.into(),
            style: decorated(UnderlineStyle::Single, true),
            selected: false,
            hovered_link: false,
        };
        assert!(blank_glyph(&cell.text));
        let drawn = paint.draw(&cell, None);
        assert!(
            drawn.underline.is_some(),
            "{text:?} must retain its underline"
        );
        assert!(drawn.strikethrough.is_some());
        let block = CursorSnapshot {
            row: 0,
            column: 0,
            visible: true,
            blinking: false,
            style: CursorStyle::Block,
        };
        let on_cursor = paint.draw(&cell, Some(block));
        assert_eq!(
            on_cursor.underline.unwrap().color,
            Some(rgb(0x112233).into())
        );
        assert_eq!(
            on_cursor.strikethrough.unwrap().color,
            Some(rgb(0x112233).into())
        );
        cell.selected = true;
        assert_eq!(
            paint.draw(&cell, None).underline.unwrap().color,
            Some(rgb(0x112233).into())
        );
        cell.style.underline_color = SnapshotColor::Rgb(unpack(0xff0000));
        assert_eq!(
            paint.draw(&cell, None).underline.unwrap().color,
            Some(rgb(0xff0000).into())
        );
        cell.style = decorated(UnderlineStyle::None, false);
        let drawn = paint.draw(&cell, None);
        assert!(drawn.underline.is_none());
        assert!(drawn.strikethrough.is_none());
    }
}

/// A pane without Pane Focus marks its cursor without competing with the
/// one being typed into: a block becomes its outline and leaves the cell
/// its own colours; a bar or an underline keeps its shape.
#[test]
fn an_unfocused_pane_outlines_a_block_cursor_and_keeps_other_shapes() {
    let spec = |cursor: CursorSnapshot, focused: bool| GridPaintSpec {
        rows: Arc::from([]),
        pass: RowPass::Whole,
        cursor: Some(cursor),
        cursor_color: None,
        default_fg: unpack(0xffffff),
        default_bg: unpack(0x112233),
        palette: None,
        cell_width: px(8.4),
        cell_height: px(16.8),
        font_family: ".SystemUIFont".into(),
        font_size: px(14.0),
        shapes: Default::default(),
        focused,
    };
    let block = CursorSnapshot {
        row: 0,
        column: 0,
        visible: true,
        blinking: true,
        style: CursorStyle::Block,
    };
    assert_eq!(
        GridPaint::new(spec(block, true)).cursor.unwrap().style,
        CursorStyle::Block
    );
    let unfocused = GridPaint::new(spec(block, false));
    assert_eq!(unfocused.cursor.unwrap().style, CursorStyle::BlockHollow);
    let cell = PositionedCell {
        column: 0,
        columns: 1,
        text: "x".into(),
        style: plain_style(SnapshotColor::Default, SnapshotColor::Default, false),
        selected: false,
        hovered_link: false,
    };
    let drawn = unfocused.draw(&cell, unfocused.cursor);
    assert_eq!(
        drawn.background,
        Some(rgb(0x112233)),
        "the cell is not inverted"
    );
    assert_eq!(drawn.foreground, rgb(0xffffff));
    assert_eq!(
        drawn.cursor.map(|cursor| cursor.style),
        Some(CursorStyle::BlockHollow),
        "the outline is still drawn over the cell"
    );
    for style in [CursorStyle::Bar, CursorStyle::Underline] {
        let paint = GridPaint::new(spec(CursorSnapshot { style, ..block }, false));
        assert_eq!(paint.cursor.unwrap().style, style);
    }
}

#[test]
fn an_undecorated_cell_asks_for_no_underline_and_no_strikethrough() {
    let style = decorated(UnderlineStyle::None, false);
    let (underline, strikethrough) =
        decorations(&style, rgb(0xd8d8e0), unpack(0xd8d8e0), None, px(16.0));
    assert!(underline.is_none());
    assert!(strikethrough.is_none());
}

#[test]
fn a_single_underline_is_straight_and_a_curly_one_is_wavy() {
    let straight = decorations(
        &decorated(UnderlineStyle::Single, false),
        rgb(0xd8d8e0),
        unpack(0xd8d8e0),
        None,
        px(16.0),
    )
    .0
    .expect("an underline");
    assert!(!straight.wavy);
    let wavy = decorations(
        &decorated(UnderlineStyle::Curly, false),
        rgb(0xd8d8e0),
        unpack(0xd8d8e0),
        None,
        px(16.0),
    )
    .0
    .expect("an underline");
    assert!(wavy.wavy);
    // GPUI draws straight or wavy; the other kinds draw straight rather
    // than not at all.
    for kind in [
        UnderlineStyle::Double,
        UnderlineStyle::Dotted,
        UnderlineStyle::Dashed,
    ] {
        let line = decorations(
            &decorated(kind, false),
            rgb(0xd8d8e0),
            unpack(0xd8d8e0),
            None,
            px(16.0),
        )
        .0
        .expect("an underline");
        assert!(!line.wavy);
    }
}

#[test]
fn an_underline_takes_the_cells_underline_colour_or_its_foreground() {
    let mut style = decorated(UnderlineStyle::Single, false);
    let plain = decorations(&style, rgb(0x123456), unpack(0xd8d8e0), None, px(16.0))
        .0
        .expect("an underline");
    assert_eq!(plain.color, Some(rgb(0x123456).into()));

    style.underline_color = SnapshotColor::Rgb(unpack(0xff0000));
    let coloured = decorations(&style, rgb(0x123456), unpack(0xd8d8e0), None, px(16.0))
        .0
        .expect("an underline");
    assert_eq!(coloured.color, Some(rgb(0xff0000).into()));
}

#[test]
fn strikethrough_alone_asks_for_no_underline() {
    let style = decorated(UnderlineStyle::None, true);
    let (underline, strikethrough) =
        decorations(&style, rgb(0xd8d8e0), unpack(0xd8d8e0), None, px(16.0));
    assert!(underline.is_none());
    let strikethrough = strikethrough.expect("a strikethrough");
    assert_eq!(strikethrough.thickness, px(1.0));
}

#[test]
fn decoration_thickness_scales_with_the_row_and_never_vanishes() {
    let style = decorated(UnderlineStyle::Single, true);
    let (underline, strikethrough) =
        decorations(&style, rgb(0xd8d8e0), unpack(0xd8d8e0), None, px(48.0));
    assert_eq!(underline.expect("underline").thickness, px(3.0));
    assert_eq!(strikethrough.expect("strikethrough").thickness, px(3.0));
    let (thin, _) = decorations(&style, rgb(0xd8d8e0), unpack(0xd8d8e0), None, px(8.0));
    assert_eq!(thin.expect("underline").thickness, px(1.0));
}

/// The shaping gate, counted where Sprite calls `shape_line`: an
/// unchanged frame shapes nothing, a blink at most the cursor's cell, a
/// one-row change only that row, and a font or theme change everything a
/// cold cache would.
#[gpui::test]
fn shaping_happens_only_for_cells_whose_drawn_text_changed(cx: &mut gpui::TestAppContext) {
    let (probe, cx) = cx.add_window_view(|_, _| ShapeProbe {
        snapshot: crate::paint_benchmark::fixture(),
        layout: Default::default(),
        shapes: Default::default(),
        blink_on: true,
        font_size: px(14.0),
    });
    let frame = |cx: &mut gpui::VisualTestContext| -> usize {
        SHAPED_CELLS.with(|count| count.set(0));
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
        });
        SHAPED_CELLS.with(|count| count.get())
    };

    // Opening the window already painted a frame, so the count starts
    // from a cold cache to see what a first frame costs.
    probe.update(cx, |probe, _| probe.shapes = Rc::default());
    let first = frame(cx);
    assert!(first > 0, "the first frame shapes what it shows");
    assert_eq!(frame(cx), 0, "an unchanged frame shapes nothing");

    probe.update(cx, |probe, _| probe.blink_on = false);
    assert!(frame(cx) <= 1, "a blink reshapes at most the cursor's cell");
    probe.update(cx, |probe, _| probe.blink_on = true);
    assert!(frame(cx) <= 1, "a blink reshapes at most the cursor's cell");

    probe.update(cx, |probe, _| {
        probe.snapshot.generation += 1;
        Arc::make_mut(&mut probe.snapshot.rows[30]).cells[10].text = "Z".into();
    });
    let shapeable = probe.update(cx, |probe, _| {
        crate::grid::lay_out_row(&probe.snapshot.rows[30])
            .iter()
            .filter(|cell| glyph_kind(cell) == GlyphKind::Text)
            .count()
    });
    let one_row = frame(cx);
    assert!(one_row >= 1, "the changed cell is shaped");
    assert!(
        one_row <= shapeable,
        "only the changed row may reshape: {one_row} shapes for {shapeable} cells"
    );

    probe.update(cx, |probe, _| probe.font_size = px(15.0));
    let refont = frame(cx);
    probe.update(cx, |probe, _| probe.shapes = Rc::default());
    let cold = frame(cx);
    assert!(cold > 0);
    assert_eq!(
        refont, cold,
        "a font change reshapes everything a cold cache would"
    );

    probe.update(cx, |probe, _| probe.snapshot.default_foreground.r ^= 0xff);
    let rethemed = frame(cx);
    probe.update(cx, |probe, _| probe.shapes = Rc::default());
    assert_eq!(
        rethemed,
        frame(cx),
        "a theme change reshapes everything a cold cache would"
    );
}

/// One plain cell reading `a`, with `change` applied, in a row of its own.
fn one_cell_row(change: impl FnOnce(&mut PositionedCell)) -> Arc<Vec<PositionedCell>> {
    let mut cell = PositionedCell {
        column: 0,
        columns: 1,
        text: "a".into(),
        style: plain_style(SnapshotColor::Default, SnapshotColor::Default, false),
        selected: false,
        hovered_link: false,
    };
    change(&mut cell);
    Arc::new(vec![cell])
}

/// Bold, italic and a hovered link each shape differently from the plain
/// cell, so none of them may reuse its shape even with the same text and
/// colour.
#[test]
fn cells_differing_only_in_weight_slant_or_link_hover_never_share_a_shape() {
    let context = ShapeContext {
        family: "monospace".into(),
        font_size: px(14.0),
        scale: 2.0,
        default_fg: unpack(0xffffff),
        default_bg: unpack(0x000000),
        palette: None,
    };
    let white = rgb(0xffffff);
    let mut cache = ShapeCache::default();
    cache.begin_frame(context, 1);
    assert!(shapes_anew(&mut cache, &one_cell_row(|_| {}), white));
    assert!(shapes_anew(
        &mut cache,
        &one_cell_row(|cell| cell.style.bold = true),
        white
    ));
    assert!(shapes_anew(
        &mut cache,
        &one_cell_row(|cell| cell.style.italic = true),
        white
    ));
    assert!(shapes_anew(
        &mut cache,
        &one_cell_row(|cell| cell.hovered_link = true),
        white
    ));
    assert!(
        !shapes_anew(&mut cache, &one_cell_row(|_| {}), white),
        "an identical plain cell still finds its shape in the pool"
    );
}

/// The pool holds at most `MAX_DISTINCT_SHAPES`; the shape that would go
/// past it empties the pool, so what was pooled before is shaped again.
#[test]
fn filling_the_shape_pool_empties_it() {
    let context = ShapeContext {
        family: "monospace".into(),
        font_size: px(14.0),
        scale: 2.0,
        default_fg: unpack(0xffffff),
        default_bg: unpack(0x000000),
        palette: None,
    };
    let cells = one_cell_row(|_| {});
    let mut cache = ShapeCache::default();
    cache.begin_frame(context, 1);
    for color in 0..MAX_DISTINCT_SHAPES as u32 {
        assert!(shapes_anew(&mut cache, &cells, rgb(color)));
    }
    assert_eq!(cache.shapes.len(), MAX_DISTINCT_SHAPES);
    assert!(
        !shapes_anew(&mut cache, &cells, rgb(0)),
        "a full pool still serves what it holds"
    );

    assert!(shapes_anew(
        &mut cache,
        &cells,
        rgb(MAX_DISTINCT_SHAPES as u32)
    ));
    assert_eq!(
        cache.shapes.len(),
        1,
        "going past the cap starts the pool again"
    );
    assert!(
        shapes_anew(&mut cache, &cells, rgb(0)),
        "a shape dropped with the pool is shaped again"
    );
}

/// Scale factor cannot be changed on a test window, so its invalidation is
/// checked on the cache directly, alongside colour and row identity.
#[test]
fn a_scale_or_font_change_drops_every_cached_shape() {
    let context = |scale: f32, font_size: f32| ShapeContext {
        family: "monospace".into(),
        font_size: px(font_size),
        scale,
        default_fg: unpack(0xffffff),
        default_bg: unpack(0x000000),
        palette: None,
    };
    let row = || {
        Arc::new(vec![PositionedCell {
            column: 0,
            columns: 1,
            text: "a".into(),
            style: plain_style(SnapshotColor::Default, SnapshotColor::Default, false),
            selected: false,
            hovered_link: false,
        }])
    };
    let white = rgb(0xffffff);
    let mut cache = ShapeCache::default();
    let cells = row();
    cache.begin_frame(context(2.0, 14.0), 1);
    assert!(shapes_anew(&mut cache, &cells, white));
    cache.begin_frame(context(2.0, 14.0), 1);
    assert!(
        !shapes_anew(&mut cache, &cells, white),
        "an unchanged frame reuses the shape"
    );
    assert!(
        shapes_anew(&mut cache, &cells, rgb(0xff0000)),
        "a new drawn colour reshapes"
    );
    assert!(
        !shapes_anew(&mut cache, &cells, white),
        "the earlier colour is still pooled"
    );
    let rebuilt = row();
    assert!(
        !shapes_anew(&mut cache, &rebuilt, white),
        "a rebuilt row with the same text finds its shape in the pool"
    );
    cache.begin_frame(context(1.0, 14.0), 1);
    assert!(
        shapes_anew(&mut cache, &rebuilt, white),
        "a scale change reshapes"
    );
    cache.begin_frame(context(1.0, 16.0), 1);
    assert!(
        shapes_anew(&mut cache, &rebuilt, white),
        "a font size change reshapes"
    );
}

/// Faint (SGR 2) is ink at half strength, Ghostty's default
/// `faint-opacity`, and never a translucent ground, even where the
/// ground is the foreground colour, as under a selection.
#[test]
fn faint_text_draws_its_glyph_at_half_alpha_over_an_opaque_ground() {
    let mut style = plain_style(
        SnapshotColor::Rgb(unpack(0x102030)),
        SnapshotColor::Rgb(unpack(0x405060)),
        false,
    );
    style.faint = true;
    let mut cell = positioned(style);
    let drawn = painter(RowPass::Whole).draw(&cell, None);
    assert_eq!(
        drawn.foreground,
        Rgba {
            a: 0.5,
            ..rgb(0x102030)
        }
    );
    assert_eq!(drawn.background, Some(rgb(0x405060)));

    cell.selected = true;
    let selected = painter(RowPass::Whole).draw(&cell, None);
    assert_eq!(
        selected.background,
        Some(rgb(0x102030)),
        "the selection ground stays opaque"
    );
    assert_eq!(
        selected.foreground,
        Rgba {
            a: 0.5,
            ..rgb(0x405060)
        }
    );
}

/// Selecting hidden text shows the selection over it while the glyphs
/// stay hidden, in every pass that draws them.
#[test]
fn a_selected_hidden_cell_shows_the_selection_and_still_hides_its_glyph() {
    let mut style = plain_style(
        SnapshotColor::Rgb(unpack(0x102030)),
        SnapshotColor::Rgb(unpack(0x405060)),
        false,
    );
    style.invisible = true;
    let mut cell = positioned(style);
    let plain = painter(RowPass::Whole).draw(&cell, None);
    assert_eq!(plain.background, Some(rgb(0x405060)));
    assert_eq!(
        plain.foreground,
        rgb(0x405060),
        "hidden text is inked in its own ground"
    );

    cell.selected = true;
    let selected = painter(RowPass::Whole).draw(&cell, None);
    assert_eq!(
        selected.background,
        Some(rgb(0x102030)),
        "the selection is shown"
    );
    assert_eq!(
        selected.foreground,
        rgb(0x102030),
        "the glyph stays hidden in the selection's colour"
    );
    let text = painter(RowPass::Text).draw(&cell, None);
    assert_eq!(text.background, None);
    assert_eq!(
        text.foreground,
        rgb(0x102030),
        "the text pass hides it against the same ground"
    );
}
