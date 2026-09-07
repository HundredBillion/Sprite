//! Box Drawing characters built from arms rather than shaped as glyphs.
//!
//! The same defect `block_elements` fixes reaches box drawing by the same
//! route: a shaped glyph's ink is as wide as the font's advance, which is not
//! the snapped cell width, so a rule built from a run of `─` is broken by a
//! sliver of background wherever the snapped step is wider than the ink.
//!
//! A box drawing character is described here as up to four arms — left, up,
//! right, down — each carrying a weight, plus the handful of characters that
//! are not arms at all: the dashes, the arcs and the diagonals. An arm is
//! drawn from the centre of the cell to the edge it points at, so two
//! neighbours that both have an arm on their shared edge meet exactly on it.
//!
//! Thickness is a whole number of device pixels rather than a fraction of the
//! cell. A rule one and a bit pixels thick is drawn as a grey smear by
//! antialiasing, and a row of them at slightly different offsets is the same
//! beading this module exists to remove.

/// How heavy one arm is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Weight {
    Light,
    Heavy,
    Double,
}

/// Which arms a character has, and how heavy each one is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Arms {
    pub left: Option<Weight>,
    pub up: Option<Weight>,
    pub right: Option<Weight>,
    pub down: Option<Weight>,
}

/// What one box drawing character is made of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BoxGlyph {
    /// Arms meeting at the centre of the cell.
    Arms(Arms),
    /// A broken line: `count` dashes along one axis. The gaps are the point,
    /// so unlike every other character here it deliberately does not tile.
    Dashed {
        count: u8,
        weight: Weight,
        vertical: bool,
    },
    /// A rounded corner joining two edges.
    Arc { right: bool, down: bool },
    /// One or both of the corner-to-corner diagonals.
    Diagonal { rising: bool, falling: bool },
}

/// A point on an outline.
pub(crate) type Point = (f32, f32);

/// One step along an outline: a straight edge, or a quadratic curve through a
/// control point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Step {
    Line(Point),
    Curve { ctrl: Point, to: Point },
}

/// A closed shape to fill, for the characters that are not rectangles.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Outline {
    pub start: Point,
    pub steps: Vec<Step>,
}

impl Outline {
    /// Every point the outline passes through, control points excluded.
    ///
    /// Only the tests ask this — the paint walks the steps directly — but it is
    /// what lets them assert where an arc lands without restating how a curve
    /// is built.
    #[cfg(test)]
    pub(crate) fn points(&self) -> Vec<Point> {
        let mut points = vec![self.start];
        for step in &self.steps {
            points.push(match *step {
                Step::Line(to) => to,
                Step::Curve { to, .. } => to,
            });
        }
        points
    }
}

/// The cell a character is drawn into, in logical pixels, already snapped.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cell {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

/// Stroke widths for one cell size, in logical pixels.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Strokes {
    pub light: f32,
    pub heavy: f32,
}

/// A rectangle to fill: `(left, top, right, bottom)`.
pub(crate) type Rect = (f32, f32, f32, f32);

/// The description of a box drawing character, or `None` for anything else.
pub(crate) fn box_glyph(ch: char) -> Option<BoxGlyph> {
    use Weight::{Double, Heavy, Light};
    let arms = |left, up, right, down| {
        Some(BoxGlyph::Arms(Arms {
            left,
            up,
            right,
            down,
        }))
    };
    match ch {
        '\u{2500}' => arms(Some(Light), None, Some(Light), None),
        '\u{2501}' => arms(Some(Heavy), None, Some(Heavy), None),
        '\u{2502}' => arms(None, Some(Light), None, Some(Light)),
        '\u{2503}' => arms(None, Some(Heavy), None, Some(Heavy)),
        '\u{256D}' => Some(BoxGlyph::Arc {
            right: true,
            down: true,
        }),
        '\u{256E}' => Some(BoxGlyph::Arc {
            right: false,
            down: true,
        }),
        '\u{256F}' => Some(BoxGlyph::Arc {
            right: false,
            down: false,
        }),
        '\u{2570}' => Some(BoxGlyph::Arc {
            right: true,
            down: false,
        }),
        '\u{2571}' => Some(BoxGlyph::Diagonal {
            rising: true,
            falling: false,
        }),
        '\u{2572}' => Some(BoxGlyph::Diagonal {
            rising: false,
            falling: true,
        }),
        '\u{2573}' => Some(BoxGlyph::Diagonal {
            rising: true,
            falling: true,
        }),
        '\u{2504}' => Some(BoxGlyph::Dashed {
            count: 3,
            weight: Light,
            vertical: false,
        }),
        '\u{2505}' => Some(BoxGlyph::Dashed {
            count: 3,
            weight: Heavy,
            vertical: false,
        }),
        '\u{2506}' => Some(BoxGlyph::Dashed {
            count: 3,
            weight: Light,
            vertical: true,
        }),
        '\u{2507}' => Some(BoxGlyph::Dashed {
            count: 3,
            weight: Heavy,
            vertical: true,
        }),
        '\u{2508}' => Some(BoxGlyph::Dashed {
            count: 4,
            weight: Light,
            vertical: false,
        }),
        '\u{2509}' => Some(BoxGlyph::Dashed {
            count: 4,
            weight: Heavy,
            vertical: false,
        }),
        '\u{250A}' => Some(BoxGlyph::Dashed {
            count: 4,
            weight: Light,
            vertical: true,
        }),
        '\u{250B}' => Some(BoxGlyph::Dashed {
            count: 4,
            weight: Heavy,
            vertical: true,
        }),
        '\u{254C}' => Some(BoxGlyph::Dashed {
            count: 2,
            weight: Light,
            vertical: false,
        }),
        '\u{254D}' => Some(BoxGlyph::Dashed {
            count: 2,
            weight: Heavy,
            vertical: false,
        }),
        '\u{254E}' => Some(BoxGlyph::Dashed {
            count: 2,
            weight: Light,
            vertical: true,
        }),
        '\u{254F}' => Some(BoxGlyph::Dashed {
            count: 2,
            weight: Heavy,
            vertical: true,
        }),
        '\u{250C}' => arms(None, None, Some(Light), Some(Light)),
        '\u{250D}' => arms(None, None, Some(Heavy), Some(Light)),
        '\u{250E}' => arms(None, None, Some(Light), Some(Heavy)),
        '\u{250F}' => arms(None, None, Some(Heavy), Some(Heavy)),
        '\u{2510}' => arms(Some(Light), None, None, Some(Light)),
        '\u{2511}' => arms(Some(Heavy), None, None, Some(Light)),
        '\u{2512}' => arms(Some(Light), None, None, Some(Heavy)),
        '\u{2513}' => arms(Some(Heavy), None, None, Some(Heavy)),
        '\u{2514}' => arms(None, Some(Light), Some(Light), None),
        '\u{2515}' => arms(None, Some(Light), Some(Heavy), None),
        '\u{2516}' => arms(None, Some(Heavy), Some(Light), None),
        '\u{2517}' => arms(None, Some(Heavy), Some(Heavy), None),
        '\u{2518}' => arms(Some(Light), Some(Light), None, None),
        '\u{2519}' => arms(Some(Heavy), Some(Light), None, None),
        '\u{251A}' => arms(Some(Light), Some(Heavy), None, None),
        '\u{251B}' => arms(Some(Heavy), Some(Heavy), None, None),
        '\u{251C}' => arms(None, Some(Light), Some(Light), Some(Light)),
        '\u{251D}' => arms(None, Some(Light), Some(Heavy), Some(Light)),
        '\u{251E}' => arms(None, Some(Heavy), Some(Light), Some(Light)),
        '\u{251F}' => arms(None, Some(Light), Some(Light), Some(Heavy)),
        '\u{2520}' => arms(None, Some(Heavy), Some(Light), Some(Heavy)),
        '\u{2521}' => arms(None, Some(Heavy), Some(Heavy), Some(Light)),
        '\u{2522}' => arms(None, Some(Light), Some(Heavy), Some(Heavy)),
        '\u{2523}' => arms(None, Some(Heavy), Some(Heavy), Some(Heavy)),
        '\u{2524}' => arms(Some(Light), Some(Light), None, Some(Light)),
        '\u{2525}' => arms(Some(Heavy), Some(Light), None, Some(Light)),
        '\u{2526}' => arms(Some(Light), Some(Heavy), None, Some(Light)),
        '\u{2527}' => arms(Some(Light), Some(Light), None, Some(Heavy)),
        '\u{2528}' => arms(Some(Light), Some(Heavy), None, Some(Heavy)),
        '\u{2529}' => arms(Some(Heavy), Some(Heavy), None, Some(Light)),
        '\u{252A}' => arms(Some(Heavy), Some(Light), None, Some(Heavy)),
        '\u{252B}' => arms(Some(Heavy), Some(Heavy), None, Some(Heavy)),
        '\u{252C}' => arms(Some(Light), None, Some(Light), Some(Light)),
        '\u{252D}' => arms(Some(Heavy), None, Some(Light), Some(Light)),
        '\u{252E}' => arms(Some(Light), None, Some(Heavy), Some(Light)),
        '\u{252F}' => arms(Some(Heavy), None, Some(Heavy), Some(Light)),
        '\u{2530}' => arms(Some(Light), None, Some(Light), Some(Heavy)),
        '\u{2531}' => arms(Some(Heavy), None, Some(Light), Some(Heavy)),
        '\u{2532}' => arms(Some(Light), None, Some(Heavy), Some(Heavy)),
        '\u{2533}' => arms(Some(Heavy), None, Some(Heavy), Some(Heavy)),
        '\u{2534}' => arms(Some(Light), Some(Light), Some(Light), None),
        '\u{2535}' => arms(Some(Heavy), Some(Light), Some(Light), None),
        '\u{2536}' => arms(Some(Light), Some(Light), Some(Heavy), None),
        '\u{2537}' => arms(Some(Heavy), Some(Light), Some(Heavy), None),
        '\u{2538}' => arms(Some(Light), Some(Heavy), Some(Light), None),
        '\u{2539}' => arms(Some(Heavy), Some(Heavy), Some(Light), None),
        '\u{253A}' => arms(Some(Light), Some(Heavy), Some(Heavy), None),
        '\u{253B}' => arms(Some(Heavy), Some(Heavy), Some(Heavy), None),
        '\u{253C}' => arms(Some(Light), Some(Light), Some(Light), Some(Light)),
        '\u{253D}' => arms(Some(Heavy), Some(Light), Some(Light), Some(Light)),
        '\u{253E}' => arms(Some(Light), Some(Light), Some(Heavy), Some(Light)),
        '\u{253F}' => arms(Some(Heavy), Some(Light), Some(Heavy), Some(Light)),
        '\u{2540}' => arms(Some(Light), Some(Heavy), Some(Light), Some(Light)),
        '\u{2541}' => arms(Some(Light), Some(Light), Some(Light), Some(Heavy)),
        '\u{2542}' => arms(Some(Light), Some(Heavy), Some(Light), Some(Heavy)),
        '\u{2543}' => arms(Some(Heavy), Some(Heavy), Some(Light), Some(Light)),
        '\u{2544}' => arms(Some(Light), Some(Heavy), Some(Heavy), Some(Light)),
        '\u{2545}' => arms(Some(Heavy), Some(Light), Some(Light), Some(Heavy)),
        '\u{2546}' => arms(Some(Light), Some(Light), Some(Heavy), Some(Heavy)),
        '\u{2547}' => arms(Some(Heavy), Some(Heavy), Some(Heavy), Some(Light)),
        '\u{2548}' => arms(Some(Heavy), Some(Light), Some(Heavy), Some(Heavy)),
        '\u{2549}' => arms(Some(Heavy), Some(Heavy), Some(Light), Some(Heavy)),
        '\u{254A}' => arms(Some(Light), Some(Heavy), Some(Heavy), Some(Heavy)),
        '\u{254B}' => arms(Some(Heavy), Some(Heavy), Some(Heavy), Some(Heavy)),
        '\u{2550}' => arms(Some(Double), None, Some(Double), None),
        '\u{2551}' => arms(None, Some(Double), None, Some(Double)),
        '\u{2552}' => arms(None, None, Some(Double), Some(Light)),
        '\u{2553}' => arms(None, None, Some(Light), Some(Double)),
        '\u{2554}' => arms(None, None, Some(Double), Some(Double)),
        '\u{2555}' => arms(Some(Double), None, None, Some(Light)),
        '\u{2556}' => arms(Some(Light), None, None, Some(Double)),
        '\u{2557}' => arms(Some(Double), None, None, Some(Double)),
        '\u{2558}' => arms(None, Some(Light), Some(Double), None),
        '\u{2559}' => arms(None, Some(Double), Some(Light), None),
        '\u{255A}' => arms(None, Some(Double), Some(Double), None),
        '\u{255B}' => arms(Some(Double), Some(Light), None, None),
        '\u{255C}' => arms(Some(Light), Some(Double), None, None),
        '\u{255D}' => arms(Some(Double), Some(Double), None, None),
        '\u{255E}' => arms(None, Some(Light), Some(Double), Some(Light)),
        '\u{255F}' => arms(None, Some(Double), Some(Light), Some(Double)),
        '\u{2560}' => arms(None, Some(Double), Some(Double), Some(Double)),
        '\u{2561}' => arms(Some(Double), Some(Light), None, Some(Light)),
        '\u{2562}' => arms(Some(Light), Some(Double), None, Some(Double)),
        '\u{2563}' => arms(Some(Double), Some(Double), None, Some(Double)),
        '\u{2564}' => arms(Some(Double), None, Some(Double), Some(Light)),
        '\u{2565}' => arms(Some(Light), None, Some(Light), Some(Double)),
        '\u{2566}' => arms(Some(Double), None, Some(Double), Some(Double)),
        '\u{2567}' => arms(Some(Double), Some(Light), Some(Double), None),
        '\u{2568}' => arms(Some(Light), Some(Double), Some(Light), None),
        '\u{2569}' => arms(Some(Double), Some(Double), Some(Double), None),
        '\u{256A}' => arms(Some(Double), Some(Light), Some(Double), Some(Light)),
        '\u{256B}' => arms(Some(Light), Some(Double), Some(Light), Some(Double)),
        '\u{256C}' => arms(Some(Double), Some(Double), Some(Double), Some(Double)),
        '\u{2574}' => arms(Some(Light), None, None, None),
        '\u{2575}' => arms(None, Some(Light), None, None),
        '\u{2576}' => arms(None, None, Some(Light), None),
        '\u{2577}' => arms(None, None, None, Some(Light)),
        '\u{2578}' => arms(Some(Heavy), None, None, None),
        '\u{2579}' => arms(None, Some(Heavy), None, None),
        '\u{257A}' => arms(None, None, Some(Heavy), None),
        '\u{257B}' => arms(None, None, None, Some(Heavy)),
        '\u{257C}' => arms(Some(Light), None, Some(Heavy), None),
        '\u{257D}' => arms(None, Some(Light), None, Some(Heavy)),
        '\u{257E}' => arms(Some(Heavy), None, Some(Light), None),
        '\u{257F}' => arms(None, Some(Heavy), None, Some(Light)),
        _ => None,
    }
}

/// Emits the rectangles one character's arms occupy inside `cell`.
pub(crate) fn box_rects(
    glyph: &BoxGlyph,
    cell: Cell,
    strokes: Strokes,
    mut emit: impl FnMut(Rect),
) {
    let arms = match glyph {
        BoxGlyph::Arms(arms) => arms,
        // Drawn as outlines instead; see `box_outlines`.
        BoxGlyph::Arc { .. } | BoxGlyph::Diagonal { .. } => return,
        BoxGlyph::Dashed {
            count,
            weight,
            vertical,
        } => {
            dashes(cell, strokes, *count, *weight, *vertical, emit);
            return;
        }
    };

    // The centre lines every arm is measured from. Both are shared by every
    // cell in the row, so arms in neighbouring cells line up by construction.
    let mid_x = midpoint(cell.left, cell.right);
    let mid_y = midpoint(cell.top, cell.bottom);

    let doubled = |arm: Option<Weight>| arm == Some(Weight::Double);

    // Where the strokes of each axis sit. A double arm is two strokes either
    // side of the centre; anything else is one stroke on it. The gap between a
    // double pair is one light stroke, so the pair reads as a pair at the size
    // a terminal actually draws them.
    let horizontal_weight = arms.left.or(arms.right);
    let vertical_weight = arms.up.or(arms.down);
    let h_lines = stroke_lines(mid_y, horizontal_weight, strokes);
    let v_lines = stroke_lines(mid_x, vertical_weight, strokes);

    // A double line opens at an intersection with another double line, which
    // is what leaves the square in the middle of a double cross. A light or
    // heavy line runs straight through instead.
    let break_h = |towards: Option<Weight>| doubled(towards) && h_lines.len() == 2;
    let break_v = |towards: Option<Weight>| doubled(towards) && v_lines.len() == 2;

    // The far edges of the perpendicular strokes, which is where an arm that
    // turns has to reach so the corner is filled rather than notched.
    let v_outer_left = v_lines.first().map_or(mid_x, |l| l.0);
    let v_outer_right = v_lines.last().map_or(mid_x, |l| l.1);
    let h_outer_top = h_lines.first().map_or(mid_y, |l| l.0);
    let h_outer_bottom = h_lines.last().map_or(mid_y, |l| l.1);

    for (index, &(top, bottom)) in h_lines.iter().enumerate() {
        let outer = index == 0;
        // The stroke this one turns into at each end: outer meets outer.
        let turn_left = if outer {
            v_outer_left
        } else {
            v_lines.last().map_or(mid_x, |l| l.0)
        };
        let turn_right = if outer {
            v_lines.first().map_or(mid_x, |l| l.1)
        } else {
            v_outer_right
        };

        if break_h(arms.up) && outer || break_h(arms.down) && !outer {
            // Opened in the middle: each end is its own piece, and only where
            // there is an arm to carry it.
            if arms.left.is_some() {
                emit((cell.left, top, v_lines_far(&v_lines, true), bottom));
            }
            if arms.right.is_some() {
                emit((v_lines_far(&v_lines, false), top, cell.right, bottom));
            }
        } else if arms.left.is_some() || arms.right.is_some() {
            let left = if arms.left.is_some() {
                cell.left
            } else {
                turn_left
            };
            let right = if arms.right.is_some() {
                cell.right
            } else {
                turn_right
            };
            emit((left, top, right, bottom));
        }
    }

    for (index, &(left, right)) in v_lines.iter().enumerate() {
        let outer = index == 0;
        let turn_up = if outer {
            h_outer_top
        } else {
            h_lines.last().map_or(mid_y, |l| l.0)
        };
        let turn_down = if outer {
            h_lines.first().map_or(mid_y, |l| l.1)
        } else {
            h_outer_bottom
        };

        if break_v(arms.left) && outer || break_v(arms.right) && !outer {
            if arms.up.is_some() {
                emit((left, cell.top, right, h_lines_far(&h_lines, true)));
            }
            if arms.down.is_some() {
                emit((left, h_lines_far(&h_lines, false), right, cell.bottom));
            }
        } else if arms.up.is_some() || arms.down.is_some() {
            let top = if arms.up.is_some() { cell.top } else { turn_up };
            let bottom = if arms.down.is_some() {
                cell.bottom
            } else {
                turn_down
            };
            emit((left, top, right, bottom));
        }
    }
}

/// Where an opened stroke stops: at the far edge of the first perpendicular
/// stroke, so the corner square is covered rather than notched.
fn v_lines_far(lines: &[(f32, f32)], leading: bool) -> f32 {
    if leading {
        lines.first().map_or(0.0, |l| l.1)
    } else {
        lines.last().map_or(0.0, |l| l.0)
    }
}

/// The horizontal counterpart of `v_lines_far`.
fn h_lines_far(lines: &[(f32, f32)], leading: bool) -> f32 {
    if leading {
        lines.first().map_or(0.0, |l| l.1)
    } else {
        lines.last().map_or(0.0, |l| l.0)
    }
}

/// The span each stroke of one axis occupies, centred on `centre`.
///
/// One stroke for a light or heavy arm, two for a double, and none where the
/// axis has no arm at all.
fn stroke_lines(centre: f32, weight: Option<Weight>, strokes: Strokes) -> Vec<(f32, f32)> {
    match weight {
        None => Vec::new(),
        Some(Weight::Double) => {
            let half = strokes.light / 2.0;
            let offset = strokes.light;
            vec![
                (centre - offset - half, centre - offset + half),
                (centre + offset - half, centre + offset + half),
            ]
        }
        Some(other) => {
            let half = match other {
                Weight::Heavy => strokes.heavy,
                _ => strokes.light,
            } / 2.0;
            vec![(centre - half, centre + half)]
        }
    }
}

/// Emits the filled outlines for the characters that are not rectangles.
///
/// An arc is stroked by running its outer curve out and its inner curve back,
/// each a quadratic through the corner a square join would have used. Offsetting
/// the control point alongside the endpoints is an approximation of a true
/// parallel curve, and at the size a terminal draws a quarter arc the error is
/// far below one pixel.
pub(crate) fn box_outlines(
    glyph: &BoxGlyph,
    cell: Cell,
    strokes: Strokes,
    mut emit: impl FnMut(Outline),
) {
    let mid_x = midpoint(cell.left, cell.right);
    let mid_y = midpoint(cell.top, cell.bottom);
    let half = strokes.light / 2.0;

    match *glyph {
        BoxGlyph::Arms(_) | BoxGlyph::Dashed { .. } => {}
        BoxGlyph::Arc { right, down } => {
            // The edge points the arc joins, and the corner it bends around.
            let x_edge = if right { cell.right } else { cell.left };
            let y_edge = if down { cell.bottom } else { cell.top };
            // Which side of the centre line is the outside of the bend.
            let x_out = if right { -half } else { half };
            let y_out = if down { -half } else { half };

            emit(Outline {
                start: (x_edge, mid_y + y_out),
                steps: vec![
                    Step::Curve {
                        ctrl: (mid_x + x_out, mid_y + y_out),
                        to: (mid_x + x_out, y_edge),
                    },
                    Step::Line((mid_x - x_out, y_edge)),
                    Step::Curve {
                        ctrl: (mid_x - x_out, mid_y - y_out),
                        to: (x_edge, mid_y - y_out),
                    },
                    Step::Line((x_edge, mid_y + y_out)),
                ],
            });
        }
        BoxGlyph::Diagonal { rising, falling } => {
            // A diagonal is a thick line corner to corner. Its width is taken
            // horizontally rather than perpendicular to the line, which is what
            // makes two of them cross in a symmetric X.
            if rising {
                emit(diagonal(
                    (cell.left, cell.bottom),
                    (cell.right, cell.top),
                    half,
                ));
            }
            if falling {
                emit(diagonal(
                    (cell.left, cell.top),
                    (cell.right, cell.bottom),
                    half,
                ));
            }
        }
    }
}

/// A thick straight line as a four-cornered outline.
fn diagonal(from: Point, to: Point, half: f32) -> Outline {
    Outline {
        start: (from.0 - half, from.1),
        steps: vec![
            Step::Line((to.0 - half, to.1)),
            Step::Line((to.0 + half, to.1)),
            Step::Line((from.0 + half, from.1)),
            Step::Line((from.0 - half, from.1)),
        ],
    }
}

/// Lays `count` dashes along one axis of the cell.
///
/// Each dash gets an equal share of the span and fills three fifths of it, so
/// the gap is visible at the smallest cell a terminal is used at without the
/// line reading as dotted at the largest.
fn dashes(
    cell: Cell,
    strokes: Strokes,
    count: u8,
    weight: Weight,
    vertical: bool,
    mut emit: impl FnMut(Rect),
) {
    const DASH_SHARE: f32 = 0.6;

    let thickness = match weight {
        Weight::Heavy => strokes.heavy,
        _ => strokes.light,
    };
    let half = thickness / 2.0;
    let (start, end) = if vertical {
        (cell.top, cell.bottom)
    } else {
        (cell.left, cell.right)
    };
    let centre = if vertical {
        midpoint(cell.left, cell.right)
    } else {
        midpoint(cell.top, cell.bottom)
    };

    let pitch = (end - start) / f32::from(count);
    for index in 0..count {
        let dash_start = start + pitch * f32::from(index);
        let dash_end = dash_start + pitch * DASH_SHARE;
        if vertical {
            emit((centre - half, dash_start, centre + half, dash_end));
        } else {
            emit((dash_start, centre - half, dash_end, centre + half));
        }
    }
}

/// The midpoint of a span.
fn midpoint(start: f32, end: f32) -> f32 {
    start + (end - start) / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid_paint::snap;
    use gpui::px;

    fn collect(ch: char, cell: Cell, strokes: Strokes) -> Vec<Rect> {
        let glyph = box_glyph(ch).unwrap_or_else(|| panic!("{ch} is a box drawing character"));
        let mut rects = Vec::new();
        box_rects(&glyph, cell, strokes, |r| rects.push(r));
        rects
    }

    /// The seam, for box drawing. A rule is a run of `─`, and each cell's
    /// horizontal arm has to end on the very float the next one begins on.
    #[test]
    fn a_rule_of_light_horizontals_tiles_without_a_seam() {
        let edge = |column: u32| f32::from(snap(px(12.7 + column as f32 * 8.4), 2.0));
        let strokes = Strokes {
            light: 1.0,
            heavy: 2.0,
        };

        for column in 0..109u32 {
            let cell = Cell {
                left: edge(column),
                top: 0.0,
                right: edge(column + 1),
                bottom: 16.0,
            };
            let rects = collect('\u{2500}', cell, strokes);
            assert_eq!(rects.len(), 1, "a light horizontal is one rectangle");
            assert_eq!(rects[0].0, cell.left, "column {column} began off its edge");
            assert_eq!(
                rects[0].2,
                edge(column + 1),
                "column {column} left a seam before its neighbour"
            );
        }
    }

    fn cell() -> Cell {
        Cell {
            left: 100.0,
            top: 200.0,
            right: 108.0,
            bottom: 216.0,
        }
    }

    fn strokes() -> Strokes {
        Strokes {
            light: 1.0,
            heavy: 3.0,
        }
    }

    /// A corner reaches the two edges it points at and neither of the others.
    /// An arm that stops short of its edge is exactly the seam, arriving from
    /// the character table rather than from the arithmetic.
    #[test]
    fn a_corner_reaches_the_two_edges_it_points_at() {
        let c = cell();
        // Down and right: the top-left corner of a box.
        let rects = collect('\u{250C}', c, strokes());
        assert_eq!(rects.len(), 2, "a corner is two arms");

        let touches_right = rects.iter().any(|r| r.2 == c.right);
        let touches_bottom = rects.iter().any(|r| r.3 == c.bottom);
        let touches_left = rects.iter().any(|r| r.0 == c.left);
        let touches_top = rects.iter().any(|r| r.1 == c.top);
        assert!(
            touches_right && touches_bottom,
            "should reach right and bottom"
        );
        assert!(!touches_left, "a top-left corner has no left arm");
        assert!(!touches_top, "a top-left corner has no up arm");
    }

    /// A cross is one full-width rectangle and one full-height one, crossing.
    #[test]
    fn a_light_cross_spans_both_directions_in_one_piece_each() {
        let c = cell();
        let rects = collect('\u{253C}', c, strokes());
        assert_eq!(rects.len(), 2, "a cross is two rectangles, not four arms");
        assert!(
            rects.iter().any(|r| r.0 == c.left && r.2 == c.right),
            "no full-width horizontal"
        );
        assert!(
            rects.iter().any(|r| r.1 == c.top && r.3 == c.bottom),
            "no full-height vertical"
        );
    }

    /// Heavy is drawn heavier than light, and both are centred on the same
    /// line, so a heavy rule and a light one meet on their shared centre.
    #[test]
    fn heavy_is_thicker_than_light_and_shares_its_centre() {
        let c = cell();
        let light = collect('\u{2500}', c, strokes())[0];
        let heavy = collect('\u{2501}', c, strokes())[0];

        let thickness = |r: Rect| r.3 - r.1;
        assert!(
            thickness(heavy) > thickness(light),
            "heavy {:?} was not thicker than light {:?}",
            thickness(heavy),
            thickness(light)
        );
        let centre = |r: Rect| (r.1 + r.3) / 2.0;
        assert_eq!(centre(heavy), centre(light), "the two are not concentric");
    }

    /// A column of `│` has to tile down the rows the same way a rule tiles
    /// across the columns.
    #[test]
    fn a_column_of_light_verticals_tiles_without_a_seam() {
        let edge = |row: u32| f32::from(snap(px(7.3 + row as f32 * 16.8), 2.0));
        for row in 0..40u32 {
            let c = Cell {
                left: 0.0,
                top: edge(row),
                right: 8.0,
                bottom: edge(row + 1),
            };
            let rects = collect('\u{2502}', c, strokes());
            assert_eq!(rects.len(), 1);
            assert_eq!(rects[0].1, c.top, "row {row} began off its edge");
            assert_eq!(rects[0].3, edge(row + 1), "row {row} left a seam below it");
        }
    }

    /// A double rule is two parallel strokes, both spanning the whole cell so
    /// a run of them still tiles, and symmetric about the same centre a light
    /// rule uses so the two can meet.
    #[test]
    fn a_double_rule_is_two_parallel_full_width_strokes() {
        let c = cell();
        let rects = collect('\u{2550}', c, strokes());
        assert_eq!(rects.len(), 2, "a double rule is two strokes");
        for r in &rects {
            assert_eq!(r.0, c.left);
            assert_eq!(r.2, c.right);
        }
        let centre = (c.top + c.bottom) / 2.0;
        let mid = |r: &Rect| (r.1 + r.3) / 2.0;
        let offsets: Vec<f32> = rects.iter().map(|r| mid(r) - centre).collect();
        assert_eq!(offsets[0], -offsets[1], "the strokes are not symmetric");
    }

    /// A double corner nests: the outer stroke turns at the outer corner and
    /// the inner stroke at the inner one. Drawing both arms full length
    /// instead would fill the corner into a solid block.
    #[test]
    fn a_double_corner_nests_its_two_strokes() {
        let c = cell();
        // Right and down: the top-left corner of a double box.
        let rects = collect('\u{2554}', c, strokes());
        assert_eq!(rects.len(), 4, "two arms, two strokes each");

        let horizontals: Vec<&Rect> = rects.iter().filter(|r| r.2 == c.right).collect();
        assert_eq!(
            horizontals.len(),
            2,
            "both horizontals reach the right edge"
        );
        // The outer (upper) horizontal starts further left than the inner one:
        // that is what makes the corner nest rather than square off.
        let mut starts: Vec<f32> = horizontals.iter().map(|r| r.0).collect();
        starts.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert!(
            starts[0] < starts[1],
            "the two horizontals start at the same x, so the corner is square"
        );
    }

    /// Every arm of a double cross is broken by the perpendicular pair, which
    /// is what leaves the open square in the middle.
    #[test]
    fn a_double_cross_is_four_separate_corner_pieces() {
        let rects = collect('\u{256C}', cell(), strokes());
        assert_eq!(rects.len(), 8, "four corners, two strokes each");
    }

    /// A light cross keeps its single stroke running straight through. Only a
    /// double line opens at an intersection.
    #[test]
    fn a_light_arm_is_not_broken_by_a_crossing_double() {
        let c = cell();
        // Double left and right, light up and down.
        let rects = collect('\u{256A}', c, strokes());
        assert!(
            rects.iter().any(|r| r.1 == c.top && r.3 == c.bottom),
            "the light vertical should run the full height"
        );
        assert_eq!(
            rects
                .iter()
                .filter(|r| r.0 == c.left && r.2 == c.right)
                .count(),
            2,
            "both double horizontals should run the full width"
        );
    }

    /// A dashed line is the one place a gap is deliberate. The dashes are
    /// evenly spaced, lie on the same centre a solid rule uses, and stay
    /// inside the cell so a run of them does not overlap.
    #[test]
    fn a_dashed_line_is_evenly_spaced_dashes_inside_its_cell() {
        let c = cell();
        // Light triple dash horizontal.
        let rects = collect('\u{2504}', c, strokes());
        assert_eq!(rects.len(), 3, "a triple dash is three dashes");

        for r in &rects {
            assert!(r.0 >= c.left && r.2 <= c.right, "{r:?} left its cell");
        }
        let centre = (c.top + c.bottom) / 2.0;
        for r in &rects {
            assert_eq!((r.1 + r.3) / 2.0, centre, "a dash is off the centre line");
        }
        // Evenly spaced: the pitch between successive dashes is constant.
        let pitch: Vec<f32> = rects.windows(2).map(|w| w[1].0 - w[0].0).collect();
        assert!(
            (pitch[0] - pitch[1]).abs() < 1e-4,
            "dashes are unevenly spaced: {pitch:?}"
        );
        // And they are dashes, not a solid line.
        assert!(rects[0].2 < rects[1].0, "the dashes touch, so it is a rule");
    }

    /// The quadruple dash has four, and the vertical variants run down the
    /// cell rather than across it.
    #[test]
    fn dash_counts_and_directions_follow_the_character() {
        let c = cell();
        assert_eq!(collect('\u{2508}', c, strokes()).len(), 4, "quadruple dash");
        assert_eq!(collect('\u{254C}', c, strokes()).len(), 2, "double dash");

        let vertical = collect('\u{2506}', c, strokes());
        assert_eq!(vertical.len(), 3);
        let centre = (c.left + c.right) / 2.0;
        for r in &vertical {
            assert_eq!((r.0 + r.2) / 2.0, centre, "a vertical dash is off centre");
            assert!(r.1 >= c.top && r.3 <= c.bottom, "{r:?} left its cell");
        }
    }

    fn outline_of(ch: char, cell: Cell, strokes: Strokes) -> Vec<Outline> {
        let glyph = box_glyph(ch).unwrap_or_else(|| panic!("{ch} is a box drawing character"));
        let mut out = Vec::new();
        box_outlines(&glyph, cell, strokes, |o| out.push(o));
        out
    }

    /// An arc still has to meet its neighbours on the cell edge: a rounded box
    /// is arcs at the corners and rules along the sides, and the join is the
    /// same join every other character here makes.
    #[test]
    fn an_arc_meets_the_two_edges_it_connects() {
        let c = cell();
        // Rounded top-left: connects right and down.
        let arcs = outline_of('\u{256D}', c, strokes());
        assert_eq!(arcs.len(), 1, "an arc is one filled outline");

        let points = arcs[0].points();
        assert!(
            points.iter().any(|p| p.0 == c.right),
            "the arc does not reach the right edge"
        );
        assert!(
            points.iter().any(|p| p.1 == c.bottom),
            "the arc does not reach the bottom edge"
        );
        assert!(
            !points.iter().any(|p| p.0 == c.left || p.1 == c.top),
            "a top-left arc should touch neither the left nor the top edge"
        );
    }

    /// A diagonal spans corner to corner, and the cross is both diagonals.
    #[test]
    fn a_diagonal_runs_corner_to_corner() {
        let c = cell();
        assert_eq!(
            outline_of('\u{2571}', c, strokes()).len(),
            1,
            "one diagonal"
        );
        assert_eq!(
            outline_of('\u{2572}', c, strokes()).len(),
            1,
            "one diagonal"
        );
        assert_eq!(
            outline_of('\u{2573}', c, strokes()).len(),
            2,
            "a cross is two"
        );

        // The forward slash rises: it starts low on the left and ends high on
        // the right. Getting the sense wrong draws a backslash.
        let points = outline_of('\u{2571}', c, strokes())[0].points();
        let leftmost = points
            .iter()
            .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap())
            .unwrap();
        let rightmost = points
            .iter()
            .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap())
            .unwrap();
        assert!(
            leftmost.1 > rightmost.1,
            "the forward slash does not rise: {leftmost:?} to {rightmost:?}"
        );
    }

    /// Arms characters produce no outlines, and arcs produce no rectangles:
    /// the two halves of the paint must not both draw the same character.
    #[test]
    fn each_character_is_drawn_by_exactly_one_half_of_the_paint() {
        let c = cell();
        assert!(
            outline_of('\u{2500}', c, strokes()).is_empty(),
            "a rule has no outline"
        );
        assert!(
            collect('\u{256D}', c, strokes()).is_empty(),
            "an arc has no rectangles"
        );
        assert!(
            collect('\u{2571}', c, strokes()).is_empty(),
            "a diagonal has no rectangles"
        );
    }
}
