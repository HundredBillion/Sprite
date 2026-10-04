use super::*;

impl Workspace {
    pub(super) fn divider_overlay(drag: DividerDrag, cx: &mut Context<Self>) -> gpui::Div {
        // GPUI delivers a move only while the element under the pointer
        // is hovered, and a pointer outruns a seven-pixel strip at
        // once. The overlay is what keeps the moves coming — and it
        // stops the drag becoming a text selection in the pane below.
        // It covers the window rather than the panes, so a pointer that
        // strays up into the tab strip mid-drag still feeds it. Nothing
        // is swallowed by that: the overlay is only ever here while a
        // button is already down.
        div()
            .absolute()
            .left(px(0.0))
            .top(px(0.0))
            .size_full()
            .occlude()
            .cursor(drag.cursor())
            .on_mouse_move(
                cx.listener(|workspace, event: &gpui::MouseMoveEvent, _window, cx| {
                    // A move with the left button no longer held
                    // means the release happened somewhere this
                    // window never saw.
                    if event.dragging() {
                        workspace.drag_divider(event.position, cx);
                    } else {
                        workspace.end_divider_drag(cx);
                    }
                }),
            )
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|workspace, _event, _window, cx| {
                    workspace.end_divider_drag(cx);
                }),
            )
    }

    pub(super) fn begin_divider_drag(
        &mut self,
        placed: DividerPlacement,
        pointer: f32,
        cx: &mut Context<Self>,
    ) {
        self.mode = Mode::DraggingDivider(DividerDrag::begin(placed, pointer));
        cx.notify();
    }
    pub(super) fn drag_divider(&mut self, position: gpui::Point<Pixels>, cx: &mut Context<Self>) {
        let Mode::DraggingDivider(drag) = self.mode else {
            return;
        };
        let ratio = drag.ratio_for(drag.along(position));
        if self
            .tabs
            .set_divider_ratio(drag.pane, drag.direction, ratio)
        {
            self.refresh_layout(cx);
            cx.notify();
        } else {
            // The boundary is gone, so there is nothing left to move.
            self.end_divider_drag(cx);
        }
    }
    pub(super) fn end_divider_drag(&mut self, cx: &mut Context<Self>) {
        if matches!(self.mode, Mode::DraggingDivider(_)) {
            self.mode = Mode::Idle;
            cx.notify();
        }
    }
    /// Returns a split to even, which is where it started.
    pub(super) fn reset_divider(
        &mut self,
        pane: PaneId,
        direction: Direction,
        cx: &mut Context<Self>,
    ) {
        if self.tabs.set_divider_ratio(pane, direction, 0.5) {
            self.refresh_layout(cx);
            cx.notify();
        }
    }
    /// Moves the focused pane's boundary on one side by a step.
    ///
    /// A pane with no boundary there — one already against the edge of its tab
    /// — does nothing. Growing it by moving the *opposite* boundary would make
    /// one key mean two different motions depending on where the pane sits.
    pub(super) fn nudge_divider(
        &mut self,
        direction: Direction,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let Some(active) = self.tabs.active() else {
            return;
        };
        let Some(focused) = active.focus() else {
            return;
        };
        let Some(divider) = self.tabs.divider(focused, direction) else {
            return;
        };

        let (width, height, _) = self.pane_area(window);
        let ratio = nudged_ratio(&divider, width, height, direction);
        if self.tabs.set_divider_ratio(focused, direction, ratio) {
            self.refresh_layout(cx);
            cx.notify();
        }
    }
    /// The pane container's width and height in pixels, and the height the tab
    /// strip took above it.
    ///
    /// Asked here by both the layout and the keyboard, so a nudge is measured
    /// against the same space the boundary was drawn in.
    pub(super) fn pane_area(&self, window: &Window) -> (f32, f32, f32) {
        let viewport: Size<Pixels> = window.viewport_size();
        // A tab strip is only worth its height when there is more than one tab.
        let strip = if self.tabs.len() > 1 {
            TAB_STRIP_HEIGHT
        } else {
            0.0
        };
        (
            f32::from(viewport.width),
            (f32::from(viewport.height) - strip).max(1.0),
            strip,
        )
    }
}
/// Where a boundary should sit within its split, as a share of that split.
///
/// `origin`, `pointer` and `extent` are all along the axis being dragged, in
/// the same pixel space. The answer is absolute rather than accumulated, so a
/// pointer shoved past the floor and brought back puts the boundary under the
/// pointer again instead of leaving it offset by however far it was shoved.
pub(super) fn divider_ratio(origin: f32, extent: f32, pointer: f32, floor: f32) -> f32 {
    // A split with no room, or too little to honour the floor on both sides,
    // has no position that obeys the rule. Even is the least surprising of the
    // answers that break it.
    if extent <= 0.0 || extent < floor * 2.0 {
        return 0.5;
    }
    let low = floor / extent;
    ((pointer - origin) / extent).clamp(low, 1.0 - low)
}

/// Where a nudged boundary should land: one keyboard step of `divider` in
/// `direction`, measured against the pane container's `width` and `height`.
///
/// Pulled out of `nudge_divider` so the two choices a keyboard step has to
/// get right — which way each direction pushes the boundary, and which of
/// `width` or `height` its split's extent is measured against — sit where a
/// test can call them without a window.
pub(super) fn nudged_ratio(
    divider: &crate::pane_tree::Divider,
    width: f32,
    height: f32,
    direction: Direction,
) -> f32 {
    let extent = match divider.orientation {
        Orientation::Horizontal => divider.area.width * width,
        Orientation::Vertical => divider.area.height * height,
    };
    // Left and up always move the boundary towards its split's origin;
    // right and down away from it.
    let step = match direction {
        Direction::Left | Direction::Up => -DIVIDER_NUDGE_PX,
        Direction::Right | Direction::Down => DIVIDER_NUDGE_PX,
    };
    // Expressed as a pointer position within the split, so the keyboard
    // goes through the same clamp the mouse does and the two cannot
    // disagree about where the floor is.
    divider_ratio(0.0, extent, divider.ratio * extent + step, DIVIDER_FLOOR_PX)
}

/// Where a divider's grab strip starts along the axis the boundary moves on.
///
/// A pane is drawn a pixel short on its *far* edge, so a boundary at 400 leaves
/// its visible gap at `[399, 400)` — centred on 399.5, not on 400. The strip
/// has to centre on the gap rather than on the boundary, which costs it half a
/// divider's width on top of half its own: that lands the flex-centred line
/// exactly on the gap and reaches equally far into the pane on either side.
pub(super) fn strip_leading(boundary: f32) -> f32 {
    boundary - (DIVIDER_GRAB_PX + DIVIDER_PX) / 2.0
}

/// The pointer's position along the axis a boundary of this orientation moves
/// on.
///
/// A left-right boundary follows x and an up-down one follows y. Both the press
/// and the moves that follow it ask here rather than each re-deriving it: the
/// swapped pair reads perfectly plausibly and would be wrong everywhere.
pub(super) fn along_axis(orientation: Orientation, position: gpui::Point<Pixels>) -> f32 {
    match orientation {
        Orientation::Horizontal => f32::from(position.x),
        Orientation::Vertical => f32::from(position.y),
    }
}

/// The pointer this orientation's boundary asks the platform to show, while it
/// is hovered and while it is dragged.
///
/// One mapping rather than two, so the strip that is drawn and the drag it
/// starts cannot come to different conclusions about which way a boundary
/// moves.
pub(super) fn cursor_for(orientation: Orientation) -> CursorStyle {
    match orientation {
        Orientation::Horizontal => CursorStyle::ResizeLeftRight,
        Orientation::Vertical => CursorStyle::ResizeUpDown,
    }
}

/// One divider's geometry in window pixels, ready to draw and to drag.
///
/// Everything is in window coordinates rather than the pane container's,
/// because a pointer event arrives in window coordinates and a drag has to
/// compare the two without remembering how tall the tab strip was.
#[derive(Clone, Copy, Debug)]
pub(super) struct DividerPlacement {
    pub(super) pane: PaneId,
    pub(super) direction: Direction,
    pub(super) orientation: Orientation,
    /// The split's start along the axis the boundary moves on.
    pub(super) origin: f32,
    /// The split's size along that axis.
    pub(super) extent: f32,
    /// Where the line itself sits along that axis.
    pub(super) boundary: f32,
    /// The strip's start across the other axis.
    pub(super) across: f32,
    /// How long the strip is across that axis.
    pub(super) span: f32,
}

impl DividerPlacement {
    pub(super) fn along(&self, position: gpui::Point<Pixels>) -> f32 {
        along_axis(self.orientation, position)
    }
}

/// Turns each boundary's normalised area into the pixels it occupies.
///
/// `width` and `height` are the pane container's, and `strip` is how tall the
/// tab strip above it is — added back here so the answer is in window space.
pub(super) fn divider_placements(
    dividers: &[crate::pane_tree::Divider],
    width: f32,
    height: f32,
    strip: f32,
) -> Vec<DividerPlacement> {
    dividers
        .iter()
        .map(|divider| {
            let area = divider.area;
            let (origin, extent, across, span) = match divider.orientation {
                Orientation::Horizontal => (
                    area.x * width,
                    area.width * width,
                    strip + area.y * height,
                    area.height * height,
                ),
                Orientation::Vertical => (
                    strip + area.y * height,
                    area.height * height,
                    area.x * width,
                    area.width * width,
                ),
            };
            DividerPlacement {
                pane: divider.pane,
                direction: divider.direction,
                orientation: divider.orientation,
                origin,
                extent,
                boundary: origin + extent * divider.ratio,
                across,
                span,
            }
        })
        .collect()
}

/// A boundary being dragged, and the geometry it was grabbed with.
///
/// The split's geometry is taken once, at the press: the layout it describes is
/// the one the drag is moving, and re-deriving it per move would let the
/// boundary chase its own change.
#[derive(Clone, Copy, Debug)]
pub(super) struct DividerDrag {
    pub(super) pane: PaneId,
    pub(super) direction: Direction,
    pub(super) orientation: Orientation,
    pub(super) origin: f32,
    pub(super) extent: f32,
    /// How far the press landed from the line, so the boundary does not jump
    /// to centre itself under the pointer.
    pub(super) grab_offset: f32,
}

impl DividerDrag {
    pub(super) fn begin(placed: DividerPlacement, pointer: f32) -> Self {
        Self {
            pane: placed.pane,
            direction: placed.direction,
            orientation: placed.orientation,
            origin: placed.origin,
            extent: placed.extent,
            grab_offset: pointer - placed.boundary,
        }
    }

    pub(super) fn ratio_for(&self, pointer: f32) -> f32 {
        divider_ratio(
            self.origin,
            self.extent,
            pointer - self.grab_offset,
            DIVIDER_FLOOR_PX,
        )
    }

    pub(super) fn cursor(&self) -> CursorStyle {
        cursor_for(self.orientation)
    }

    /// The pointer's position along the axis this drag moves on.
    pub(super) fn along(&self, position: gpui::Point<Pixels>) -> f32 {
        along_axis(self.orientation, position)
    }
}

impl Workspace {
    pub(super) fn divider_elements(&self, strip: f32, cx: &mut Context<Self>) -> Vec<gpui::Div> {
        self.dividers
            .iter()
            .map(|(placed, group)| {
                let placed = *placed;
                // A group per divider, so the line can answer its own strip
                // being hovered without the workspace keeping any state.
                let group = group.clone();
                let horizontal = placed.orientation == Orientation::Horizontal;
                let leading = strip_leading(placed.boundary);
                // A dragged line stays lit even once the pointer has left the
                // strip behind, which it does the moment the drag gets going.
                let dragging = self.mode.divider_drag().is_some_and(|drag| {
                    drag.pane == placed.pane && drag.direction == placed.direction
                });
                // The container is a flex child below the tab strip, so a
                // window coordinate down the window has to lose that height
                // before it means anything to an absolutely placed child.
                let (element, line) = if horizontal {
                    (
                        div()
                            .left(px(leading))
                            .top(px(placed.across - strip))
                            .w(px(DIVIDER_GRAB_PX))
                            .h(px(placed.span)),
                        div().w(px(DIVIDER_PX)).h_full(),
                    )
                } else {
                    (
                        div()
                            .left(px(placed.across))
                            .top(px(leading - strip))
                            .w(px(placed.span))
                            .h(px(DIVIDER_GRAB_PX)),
                        div().w_full().h(px(DIVIDER_PX)),
                    )
                };
                element
                    .absolute()
                    .flex()
                    .items_center()
                    .justify_center()
                    .group(group.clone())
                    // The strip answers the pointer rather than the pane under
                    // it: a gesture on a divider is not a gesture in a pane.
                    .occlude()
                    .cursor(cursor_for(placed.orientation))
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(
                            move |workspace, event: &gpui::MouseDownEvent, _window, cx| {
                                // The second click of a double-click evens the
                                // split rather than starting a drag, so undoing
                                // an over-enthusiastic one takes a single
                                // gesture. Two or more, so a third click does
                                // not leave a stray drag behind.
                                if event.click_count >= 2 {
                                    workspace.end_divider_drag(cx);
                                    workspace.reset_divider(placed.pane, placed.direction, cx);
                                    return;
                                }
                                workspace.begin_divider_drag(
                                    placed,
                                    placed.along(event.position),
                                    cx,
                                );
                            },
                        ),
                    )
                    .child(
                        line.bg(rgb(if dragging { DIVIDER_HOVER } else { DIVIDER }))
                            .group_hover(group, |style| style.bg(rgb(DIVIDER_HOVER))),
                    )
            })
            .collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    /// Left and right must move opposite ways, up and down must move opposite
    /// ways, and each pair has to be sized against the right dimension of a
    /// non-square container — not the four ways this has quietly gone wrong
    /// before. The container is 800 by 400 and the divider's own area is a
    /// quarter of it on one axis and three quarters on the other, so a sign
    /// swap sends the ratio the wrong way and an axis swap lands on a
    /// different number rather than coincidentally the right one.
    #[test]
    fn a_nudge_steps_the_boundary_the_right_way_on_each_axis() {
        let area = crate::pane_tree::Rect {
            x: 0.25,
            y: 0.1,
            width: 0.5,
            height: 0.75,
        };
        let divider = |orientation, direction| crate::pane_tree::Divider {
            pane: PaneId(0),
            direction,
            orientation,
            ratio: 0.5,
            area,
        };

        // Horizontal extent is 0.5 * 800 = 400, so a 20 px step is 0.05 of it.
        let right = nudged_ratio(
            &divider(Orientation::Horizontal, Direction::Right),
            800.0,
            400.0,
            Direction::Right,
        );
        assert!((right - 0.55).abs() < 1e-6, "right grows the ratio");

        let left = nudged_ratio(
            &divider(Orientation::Horizontal, Direction::Left),
            800.0,
            400.0,
            Direction::Left,
        );
        assert!((left - 0.45).abs() < 1e-6, "left shrinks the ratio");

        // Vertical extent is 0.75 * 400 = 300, a different number from the
        // horizontal case above, so measuring against the wrong dimension
        // would not pass by accident.
        let down = nudged_ratio(
            &divider(Orientation::Vertical, Direction::Down),
            800.0,
            400.0,
            Direction::Down,
        );
        assert!(
            (down - (170.0 / 300.0)).abs() < 1e-6,
            "down grows the ratio"
        );

        let up = nudged_ratio(
            &divider(Orientation::Vertical, Direction::Up),
            800.0,
            400.0,
            Direction::Up,
        );
        assert!((up - (130.0 / 300.0)).abs() < 1e-6, "up shrinks the ratio");
    }
    /// A nudge goes through the same clamp the mouse does, so a boundary
    /// already at the floor stays there instead of a keyboard step pushing it
    /// past what a drag would ever allow.
    #[test]
    fn a_nudge_still_stops_at_the_floor() {
        let area = crate::pane_tree::Rect {
            x: 0.25,
            y: 0.1,
            width: 0.5,
            height: 0.75,
        };
        // Horizontal extent is 400, so the floor sits at 120 / 400 = 0.3 —
        // exactly where this divider already is.
        let divider = crate::pane_tree::Divider {
            pane: PaneId(0),
            direction: Direction::Left,
            orientation: Orientation::Horizontal,
            ratio: DIVIDER_FLOOR_PX / (area.width * 800.0),
            area,
        };
        let ratio = nudged_ratio(&divider, 800.0, 400.0, Direction::Left);
        assert!(
            (ratio - 0.3).abs() < 1e-6,
            "another step left must not cross the floor"
        );
    }
    #[test]
    fn a_pointer_in_the_middle_gives_an_even_split() {
        assert!((divider_ratio(100.0, 400.0, 300.0, 120.0) - 0.5).abs() < 1e-6);
    }
    #[test]
    fn a_pointer_is_measured_from_the_splits_own_origin() {
        // A nested split starting 100 px in: the pointer at 260 is two fifths
        // of the way across it. Measured against the window instead it would
        // read as 0.65, so this fails if the origin is ignored.
        assert!((divider_ratio(100.0, 400.0, 260.0, 120.0) - 0.4).abs() < 1e-6);
    }
    #[test]
    fn neither_side_may_be_driven_below_the_floor() {
        // 120 of 400 is 0.3, and 1 - 0.3 on the other end.
        assert!((divider_ratio(0.0, 400.0, -500.0, 120.0) - 0.3).abs() < 1e-6);
        assert!((divider_ratio(0.0, 400.0, 900.0, 120.0) - 0.7).abs() < 1e-6);
    }
    /// The reason the drag is absolute rather than accumulated: shoving the
    /// pointer past the floor and bringing it back must put the boundary under
    /// the pointer again, not leave it offset by however far it was shoved.
    #[test]
    fn a_boundary_pushed_past_the_floor_comes_straight_back() {
        let floor = 120.0;
        assert!((divider_ratio(0.0, 400.0, -500.0, floor) - 0.3).abs() < 1e-6);
        // Back inside the legal range, the boundary is under the pointer again.
        // An implementation that accumulated the overshoot would answer with
        // the 500 px it was shoved by still subtracted.
        assert!((divider_ratio(0.0, 400.0, 240.0, floor) - 0.6).abs() < 1e-6);
    }
    #[test]
    fn a_split_too_small_for_two_floors_stays_even() {
        // 200 px cannot give both sides 120, so no position satisfies the rule
        // and the boundary sits in the middle rather than at one extreme.
        assert!((divider_ratio(0.0, 200.0, 10.0, 120.0) - 0.5).abs() < 1e-6);
        assert!((divider_ratio(0.0, 0.0, 10.0, 120.0) - 0.5).abs() < 1e-6);
    }
    #[test]
    fn the_floor_is_the_one_the_product_promises() {
        assert!((DIVIDER_FLOOR_PX - 120.0).abs() < f32::EPSILON);
    }
    #[test]
    fn a_horizontal_split_places_its_strip_down_the_middle() {
        let divider = crate::pane_tree::Divider {
            pane: PaneId(0),
            direction: Direction::Right,
            orientation: Orientation::Horizontal,
            ratio: 0.5,
            area: crate::pane_tree::Rect::FULL,
        };
        let placed = divider_placements(&[divider], 800.0, 600.0, 28.0);

        assert_eq!(placed.len(), 1);
        let placed = placed[0];
        assert!((placed.boundary - 400.0).abs() < 1e-4, "half of 800");
        assert!((placed.origin - 0.0).abs() < 1e-4);
        assert!((placed.extent - 800.0).abs() < 1e-4);
        // Down the full height of the panes area, which starts below the strip.
        assert!((placed.across - 28.0).abs() < 1e-4);
        assert!((placed.span - 600.0).abs() < 1e-4);
    }
    #[test]
    fn a_vertical_split_measures_from_below_the_tab_strip() {
        let divider = crate::pane_tree::Divider {
            pane: PaneId(0),
            direction: Direction::Down,
            orientation: Orientation::Vertical,
            ratio: 0.25,
            area: crate::pane_tree::Rect::FULL,
        };
        let placed = divider_placements(&[divider], 800.0, 600.0, 28.0);

        let placed = placed[0];
        // The pointer arrives in window coordinates, so everything a drag
        // compares it against is in window coordinates too.
        assert!((placed.origin - 28.0).abs() < 1e-4);
        assert!((placed.extent - 600.0).abs() < 1e-4);
        assert!((placed.boundary - (28.0 + 150.0)).abs() < 1e-4);
        assert!((placed.across - 0.0).abs() < 1e-4);
        assert!((placed.span - 800.0).abs() < 1e-4);
    }
    #[test]
    fn a_nested_split_is_placed_inside_its_own_area_only() {
        let divider = crate::pane_tree::Divider {
            pane: PaneId(0),
            direction: Direction::Right,
            orientation: Orientation::Horizontal,
            ratio: 0.5,
            area: crate::pane_tree::Rect {
                x: 0.5,
                y: 0.0,
                width: 0.5,
                height: 1.0,
            },
        };
        let placed = divider_placements(&[divider], 800.0, 600.0, 0.0);

        let placed = placed[0];
        assert!((placed.origin - 400.0).abs() < 1e-4);
        assert!((placed.extent - 400.0).abs() < 1e-4);
        assert!((placed.boundary - 600.0).abs() < 1e-4);
    }
    /// A pane is shortened on its far edge, so the gap a boundary shows through
    /// sits just before the boundary rather than astride it.
    #[test]
    fn a_strip_centres_on_the_gap_rather_than_on_the_line() {
        // A boundary at 400 leaves its gap at [399, 400), whose middle is
        // 399.5 — so a seven-pixel strip starts at 396, not at 396.5.
        let leading = strip_leading(400.0);
        assert!((leading - 396.0).abs() < 1e-4);
        // Which is what lands the flex-centred line exactly on the gap, and
        // reaches the same three pixels into the pane on either side of it.
        assert!((leading + (DIVIDER_GRAB_PX - DIVIDER_PX) / 2.0 - 399.0).abs() < 1e-4);
        assert!((leading + DIVIDER_GRAB_PX - 403.0).abs() < 1e-4);
    }
    /// The press records where inside the strip it landed, so the boundary does
    /// not jump to centre itself under the pointer on the first move.
    #[test]
    fn a_grab_keeps_its_offset_within_the_strip() {
        let placed = divider_placements(
            &[crate::pane_tree::Divider {
                pane: PaneId(0),
                direction: Direction::Right,
                orientation: Orientation::Horizontal,
                ratio: 0.5,
                area: crate::pane_tree::Rect::FULL,
            }],
            800.0,
            600.0,
            0.0,
        )[0];
        // Pressed 3 px to the right of the line itself.
        let drag = DividerDrag::begin(placed, 403.0);
        assert!((drag.grab_offset - 3.0).abs() < 1e-4);

        // Moving to 500 should put the *line* at 497, not at 500.
        assert!((drag.ratio_for(500.0) - (497.0 / 800.0)).abs() < 1e-4);
    }
    #[test]
    fn a_drag_holds_the_floor_it_was_given() {
        let placed = divider_placements(
            &[crate::pane_tree::Divider {
                pane: PaneId(0),
                direction: Direction::Right,
                orientation: Orientation::Horizontal,
                ratio: 0.5,
                area: crate::pane_tree::Rect::FULL,
            }],
            800.0,
            600.0,
            0.0,
        )[0];
        let drag = DividerDrag::begin(placed, 400.0);
        assert!((drag.ratio_for(-200.0) - (DIVIDER_FLOOR_PX / 800.0)).abs() < 1e-4);
    }
    /// A left-right boundary moves with the pointer's x and an up-down one with
    /// its y. Swapping the two reads plausibly and would be wrong everywhere.
    #[test]
    fn a_drag_reads_the_axis_its_own_orientation_moves_on() {
        let drag = |orientation, direction| {
            DividerDrag::begin(
                divider_placements(
                    &[crate::pane_tree::Divider {
                        pane: PaneId(0),
                        direction,
                        orientation,
                        ratio: 0.5,
                        area: crate::pane_tree::Rect::FULL,
                    }],
                    800.0,
                    600.0,
                    0.0,
                )[0],
                0.0,
            )
        };
        let pointer = gpui::Point {
            x: gpui::px(120.0),
            y: gpui::px(450.0),
        };

        let sideways = drag(Orientation::Horizontal, Direction::Right);
        assert!((sideways.along(pointer) - 120.0).abs() < 1e-4);
        assert_eq!(sideways.cursor(), CursorStyle::ResizeLeftRight);

        let upright = drag(Orientation::Vertical, Direction::Down);
        assert!((upright.along(pointer) - 450.0).abs() < 1e-4);
        assert_eq!(upright.cursor(), CursorStyle::ResizeUpDown);
    }
    /// The press reads the same axis the drag then follows. The two probe
    /// coordinates differ so a transposition cannot pass by landing on a
    /// number that happens to be right for both axes.
    #[test]
    fn a_placement_reads_the_axis_its_own_orientation_moves_on() {
        let place = |orientation, direction| {
            divider_placements(
                &[crate::pane_tree::Divider {
                    pane: PaneId(0),
                    direction,
                    orientation,
                    ratio: 0.5,
                    area: crate::pane_tree::Rect::FULL,
                }],
                800.0,
                600.0,
                0.0,
            )[0]
        };
        let pointer = gpui::Point {
            x: gpui::px(120.0),
            y: gpui::px(450.0),
        };

        let sideways = place(Orientation::Horizontal, Direction::Right);
        assert!((sideways.along(pointer) - 120.0).abs() < 1e-4);

        let upright = place(Orientation::Vertical, Direction::Down);
        assert!((upright.along(pointer) - 450.0).abs() < 1e-4);
    }
}
