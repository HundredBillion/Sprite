//! Where Surfaces sit in a pane, and how many fit: at most one fill, one dock
//! per side, and any number of overlays, the last opened on top.
//!
//! Generic over what a Surface is, so the bookkeeping is tested with plain
//! values while the view stores its own hosted type.

use crate::surface::Refusal;
use crate::surface::channel::{Position, Side};

#[derive(Debug)]
pub(crate) struct SurfaceHost<S> {
    pub fill: Option<S>,
    pub left: Option<S>,
    pub right: Option<S>,
    pub overlays: Vec<S>,
}

impl<S> Default for SurfaceHost<S> {
    fn default() -> Self {
        Self {
            fill: None,
            left: None,
            right: None,
            overlays: Vec::new(),
        }
    }
}

impl<S> SurfaceHost<S> {
    /// Places a Surface, or refuses if the position is taken. Overlays are
    /// never refused: they stack.
    pub fn place(&mut self, position: Position, side: Side, surface: S) -> Result<(), Refusal> {
        let slot = match position {
            Position::Overlay => {
                self.overlays.push(surface);
                return Ok(());
            }
            Position::Fill => &mut self.fill,
            Position::Dock => match side {
                Side::Left => &mut self.left,
                Side::Right => &mut self.right,
            },
        };
        if slot.is_some() {
            return Err(Refusal::PositionOccupied);
        }
        *slot = Some(surface);
        Ok(())
    }

    /// Removes the first Surface that matches, saying where it was.
    pub fn take(&mut self, matches: impl Fn(&S) -> bool) -> Option<(S, Position)> {
        if self.fill.as_ref().is_some_and(&matches) {
            return self.fill.take().map(|surface| (surface, Position::Fill));
        }
        if self.left.as_ref().is_some_and(&matches) {
            return self.left.take().map(|surface| (surface, Position::Dock));
        }
        if self.right.as_ref().is_some_and(&matches) {
            return self.right.take().map(|surface| (surface, Position::Dock));
        }
        let index = self.overlays.iter().position(matches)?;
        Some((self.overlays.remove(index), Position::Overlay))
    }

    pub fn get_mut(&mut self, matches: impl Fn(&S) -> bool) -> Option<&mut S> {
        self.iter_mut().find(|surface| matches(surface))
    }

    /// Fill, left dock, right dock, then overlays in opening order — the
    /// order focus cycles through them.
    pub fn iter(&self) -> impl Iterator<Item = &S> {
        self.fill
            .iter()
            .chain(self.left.iter())
            .chain(self.right.iter())
            .chain(self.overlays.iter())
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut S> {
        self.fill
            .iter_mut()
            .chain(self.left.iter_mut())
            .chain(self.right.iter_mut())
            .chain(self.overlays.iter_mut())
    }

    pub fn is_empty(&self) -> bool {
        self.iter().next().is_none()
    }

    /// The strips the docks take, left and right, as `width` measures them.
    pub fn dock_widths(&self, width: impl Fn(&S) -> f32) -> (f32, f32) {
        (
            self.left.as_ref().map(&width).unwrap_or(0.0),
            self.right.as_ref().map(&width).unwrap_or(0.0),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_fill_or_a_second_dock_on_the_same_side_is_refused() {
        let mut host: SurfaceHost<&str> = SurfaceHost::default();
        assert_eq!(host.place(Position::Fill, Side::Left, "a"), Ok(()));
        assert_eq!(
            host.place(Position::Fill, Side::Right, "b"),
            Err(Refusal::PositionOccupied)
        );
        assert_eq!(host.place(Position::Dock, Side::Left, "c"), Ok(()));
        assert_eq!(
            host.place(Position::Dock, Side::Left, "d"),
            Err(Refusal::PositionOccupied)
        );
        // The other side is free: two docks coexist.
        assert_eq!(host.place(Position::Dock, Side::Right, "e"), Ok(()));
        assert_eq!(
            host.iter().copied().collect::<Vec<_>>(),
            vec!["a", "c", "e"]
        );
    }

    #[test]
    fn overlays_stack_in_the_order_they_opened() {
        let mut host: SurfaceHost<&str> = SurfaceHost::default();
        for name in ["first", "second", "third"] {
            assert_eq!(host.place(Position::Overlay, Side::Left, name), Ok(()));
        }
        assert_eq!(host.overlays, vec!["first", "second", "third"]);
        assert_eq!(
            host.take(|s| *s == "second"),
            Some(("second", Position::Overlay))
        );
        assert_eq!(host.overlays, vec!["first", "third"]);
    }

    #[test]
    fn taking_a_surface_frees_its_position_and_says_which_it_was() {
        let mut host: SurfaceHost<&str> = SurfaceHost::default();
        host.place(Position::Dock, Side::Right, "dock")
            .expect("place");
        host.place(Position::Fill, Side::Left, "fill")
            .expect("place");
        assert_eq!(host.take(|s| *s == "dock"), Some(("dock", Position::Dock)));
        assert_eq!(host.take(|s| *s == "dock"), None);
        assert_eq!(host.place(Position::Dock, Side::Right, "again"), Ok(()));
        assert_eq!(host.take(|s| *s == "fill"), Some(("fill", Position::Fill)));
        assert!(!host.is_empty());
        assert_eq!(
            host.take(|s| *s == "again"),
            Some(("again", Position::Dock))
        );
        assert!(host.is_empty());
    }

    #[test]
    fn dock_widths_come_from_the_docks_alone() {
        let mut host: SurfaceHost<(&str, f32)> = SurfaceHost::default();
        host.place(Position::Dock, Side::Left, ("l", 240.0))
            .expect("place");
        host.place(Position::Overlay, Side::Left, ("o", 999.0))
            .expect("place");
        assert_eq!(host.dock_widths(|(_, width)| *width), (240.0, 0.0));
        host.place(Position::Dock, Side::Right, ("r", 100.0))
            .expect("place");
        assert_eq!(host.dock_widths(|(_, width)| *width), (240.0, 100.0));
        assert_eq!(
            host.get_mut(|(name, _)| *name == "r").map(|(_, w)| *w),
            Some(100.0)
        );
    }
}
