use super::GridPaint;
use gpui::{AnyElement, App, IntoElement};

#[derive(Default)]
pub(crate) struct RetainedGrid;

impl RetainedGrid {
    pub(crate) fn prepare(&mut self, grid: GridPaint, _scale: f32, _cx: &mut App) -> AnyElement {
        grid.into_any_element()
    }
}
