//! Decode Kitty Unicode image placeholders in terminal cells.

use gpui::{Pixels, px};
use sprite_term::{Placement, SnapshotColor};
use std::collections::HashMap;

use crate::grid::PositionedCell;

// Kitty assigns this ordered list of combining marks to row and column
// indexes; svgtree uses the same encoding for its image icons.
const DIACRITICS: &[char] = &[
    '\u{0305}',
    '\u{030d}',
    '\u{030e}',
    '\u{0310}',
    '\u{0312}',
    '\u{033d}',
    '\u{033e}',
    '\u{033f}',
    '\u{0346}',
    '\u{034a}',
    '\u{034b}',
    '\u{034c}',
    '\u{0350}',
    '\u{0351}',
    '\u{0352}',
    '\u{0357}',
    '\u{035b}',
    '\u{0363}',
    '\u{0364}',
    '\u{0365}',
    '\u{0366}',
    '\u{0367}',
    '\u{0368}',
    '\u{0369}',
    '\u{036a}',
    '\u{036b}',
    '\u{036c}',
    '\u{036d}',
    '\u{036e}',
    '\u{036f}',
    '\u{0483}',
    '\u{0484}',
    '\u{0485}',
    '\u{0486}',
    '\u{0487}',
    '\u{0592}',
    '\u{0593}',
    '\u{0594}',
    '\u{0595}',
    '\u{0597}',
    '\u{0598}',
    '\u{0599}',
    '\u{059c}',
    '\u{059d}',
    '\u{059e}',
    '\u{059f}',
    '\u{05a0}',
    '\u{05a1}',
    '\u{05a8}',
    '\u{05a9}',
    '\u{05ab}',
    '\u{05ac}',
    '\u{05af}',
    '\u{05c4}',
    '\u{0610}',
    '\u{0611}',
    '\u{0612}',
    '\u{0613}',
    '\u{0614}',
    '\u{0615}',
    '\u{0616}',
    '\u{0617}',
    '\u{0657}',
    '\u{0658}',
    '\u{0659}',
    '\u{065a}',
    '\u{065b}',
    '\u{065d}',
    '\u{065e}',
    '\u{06d6}',
    '\u{06d7}',
    '\u{06d8}',
    '\u{06d9}',
    '\u{06da}',
    '\u{06db}',
    '\u{06dc}',
    '\u{06df}',
    '\u{06e0}',
    '\u{06e1}',
    '\u{06e2}',
    '\u{06e4}',
    '\u{06e7}',
    '\u{06e8}',
    '\u{06eb}',
    '\u{06ec}',
    '\u{0730}',
    '\u{0732}',
    '\u{0733}',
    '\u{0735}',
    '\u{0736}',
    '\u{073a}',
    '\u{073d}',
    '\u{073f}',
    '\u{0740}',
    '\u{0741}',
    '\u{0743}',
    '\u{0745}',
    '\u{0747}',
    '\u{0749}',
    '\u{074a}',
    '\u{07eb}',
    '\u{07ec}',
    '\u{07ed}',
    '\u{07ee}',
    '\u{07ef}',
    '\u{07f0}',
    '\u{07f1}',
    '\u{07f3}',
    '\u{0816}',
    '\u{0817}',
    '\u{0818}',
    '\u{0819}',
    '\u{081b}',
    '\u{081c}',
    '\u{081d}',
    '\u{081e}',
    '\u{081f}',
    '\u{0820}',
    '\u{0821}',
    '\u{0822}',
    '\u{0823}',
    '\u{0825}',
    '\u{0826}',
    '\u{0827}',
    '\u{0829}',
    '\u{082a}',
    '\u{082b}',
    '\u{082c}',
    '\u{082d}',
    '\u{0951}',
    '\u{0953}',
    '\u{0954}',
    '\u{0f82}',
    '\u{0f83}',
    '\u{0f86}',
    '\u{0f87}',
    '\u{135d}',
    '\u{135e}',
    '\u{135f}',
    '\u{17dd}',
    '\u{193a}',
    '\u{1a17}',
    '\u{1a75}',
    '\u{1a76}',
    '\u{1a77}',
    '\u{1a78}',
    '\u{1a79}',
    '\u{1a7a}',
    '\u{1a7b}',
    '\u{1a7c}',
    '\u{1b6b}',
    '\u{1b6d}',
    '\u{1b6e}',
    '\u{1b6f}',
    '\u{1b70}',
    '\u{1b71}',
    '\u{1b72}',
    '\u{1b73}',
    '\u{1cd0}',
    '\u{1cd1}',
    '\u{1cd2}',
    '\u{1cda}',
    '\u{1cdb}',
    '\u{1ce0}',
    '\u{1dc0}',
    '\u{1dc1}',
    '\u{1dc3}',
    '\u{1dc4}',
    '\u{1dc5}',
    '\u{1dc6}',
    '\u{1dc7}',
    '\u{1dc8}',
    '\u{1dc9}',
    '\u{1dcb}',
    '\u{1dcc}',
    '\u{1dd1}',
    '\u{1dd2}',
    '\u{1dd3}',
    '\u{1dd4}',
    '\u{1dd5}',
    '\u{1dd6}',
    '\u{1dd7}',
    '\u{1dd8}',
    '\u{1dd9}',
    '\u{1dda}',
    '\u{1ddb}',
    '\u{1ddc}',
    '\u{1ddd}',
    '\u{1dde}',
    '\u{1ddf}',
    '\u{1de0}',
    '\u{1de1}',
    '\u{1de2}',
    '\u{1de3}',
    '\u{1de4}',
    '\u{1de5}',
    '\u{1de6}',
    '\u{1dfe}',
    '\u{20d0}',
    '\u{20d1}',
    '\u{20d4}',
    '\u{20d5}',
    '\u{20d6}',
    '\u{20d7}',
    '\u{20db}',
    '\u{20dc}',
    '\u{20e1}',
    '\u{20e7}',
    '\u{20e9}',
    '\u{20f0}',
    '\u{2cef}',
    '\u{2cf0}',
    '\u{2cf1}',
    '\u{2de0}',
    '\u{2de1}',
    '\u{2de2}',
    '\u{2de3}',
    '\u{2de4}',
    '\u{2de5}',
    '\u{2de6}',
    '\u{2de7}',
    '\u{2de8}',
    '\u{2de9}',
    '\u{2dea}',
    '\u{2deb}',
    '\u{2dec}',
    '\u{2ded}',
    '\u{2dee}',
    '\u{2def}',
    '\u{2df0}',
    '\u{2df1}',
    '\u{2df2}',
    '\u{2df3}',
    '\u{2df4}',
    '\u{2df5}',
    '\u{2df6}',
    '\u{2df7}',
    '\u{2df8}',
    '\u{2df9}',
    '\u{2dfa}',
    '\u{2dfb}',
    '\u{2dfc}',
    '\u{2dfd}',
    '\u{2dfe}',
    '\u{2dff}',
    '\u{a66f}',
    '\u{a67c}',
    '\u{a67d}',
    '\u{a6f0}',
    '\u{a6f1}',
    '\u{a8e0}',
    '\u{a8e1}',
    '\u{a8e2}',
    '\u{a8e3}',
    '\u{a8e4}',
    '\u{a8e5}',
    '\u{a8e6}',
    '\u{a8e7}',
    '\u{a8e8}',
    '\u{a8e9}',
    '\u{a8ea}',
    '\u{a8eb}',
    '\u{a8ec}',
    '\u{a8ed}',
    '\u{a8ee}',
    '\u{a8ef}',
    '\u{a8f0}',
    '\u{a8f1}',
    '\u{aab0}',
    '\u{aab2}',
    '\u{aab3}',
    '\u{aab7}',
    '\u{aab8}',
    '\u{aabe}',
    '\u{aabf}',
    '\u{aac1}',
    '\u{fe20}',
    '\u{fe21}',
    '\u{fe22}',
    '\u{fe23}',
    '\u{fe24}',
    '\u{fe25}',
    '\u{fe26}',
    '\u{10a0f}',
    '\u{10a38}',
    '\u{1d185}',
    '\u{1d186}',
    '\u{1d187}',
    '\u{1d188}',
    '\u{1d189}',
    '\u{1d1aa}',
    '\u{1d1ab}',
    '\u{1d1ac}',
    '\u{1d1ad}',
    '\u{1d242}',
    '\u{1d243}',
    '\u{1d244}',
];

pub(super) fn coordinates(text: &str) -> Option<(u32, u32)> {
    let mut chars = text.chars();
    if chars.next()? != '\u{10eeee}' {
        return None;
    }
    let row_mark = chars.next()?;
    let col_mark = chars.next()?;
    let row = DIACRITICS.iter().position(|mark| *mark == row_mark)?;
    let col = DIACRITICS.iter().position(|mark| *mark == col_mark)?;
    Some((row as u32, col as u32))
}

pub(super) struct ImageCell<'a> {
    pub placement: &'a Placement,
    pub column: u16,
    pub row: usize,
    pub image_column: u32,
    pub image_row: u32,
}

pub(super) struct ImageFit {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}

pub(super) fn fit_image(
    image_width: u32,
    image_height: u32,
    box_width: f32,
    box_height: f32,
) -> Option<ImageFit> {
    if image_width == 0 || image_height == 0 || box_width <= 0.0 || box_height <= 0.0 {
        return None;
    }
    let scale = (box_width / image_width as f32).min(box_height / image_height as f32);
    let width = image_width as f32 * scale;
    let height = image_height as f32 * scale;
    Some(ImageFit {
        left: (box_width - width) / 2.0,
        top: (box_height - height) / 2.0,
        width,
        height,
    })
}

/// Where one placeholder tile and the image inside it sit, relative to the
/// grid's corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct TileGeometry {
    pub left: Pixels,
    pub top: Pixels,
    pub width: Pixels,
    pub height: Pixels,
    /// The image's corner, relative to the tile's own.
    pub image_left: Pixels,
    pub image_top: Pixels,
}

/// Places one tile on the device-pixel grid the cells are painted on.
///
/// The tile's edges are the snapped column and row edges the painter uses, so
/// a tile meets its neighbours and the cells beside it edge to edge. Like the
/// painter's, they are snapped where they fall in the window, `grid_origin`
/// being the grid's corner there, and then given relative to that corner. The
/// image inside is positioned from the placement's own unsnapped corner, which
/// every tile of it shares, so the picture is one continuous image across
/// tiles.
pub(super) fn tile_geometry(
    cell: &ImageCell<'_>,
    fit: &ImageFit,
    grid_origin: gpui::Point<Pixels>,
    cell_width: Pixels,
    cell_height: Pixels,
    scale: f32,
) -> TileGeometry {
    use crate::grid::{Col, Row, column_edge, row_edge};
    let column = u32::from(cell.column);
    let (x, y) = (grid_origin.x, grid_origin.y);
    let left = column_edge(x, cell_width, Col(column), scale).pixels() - x;
    let right = column_edge(x, cell_width, Col(column + 1), scale).pixels() - x;
    let top = row_edge(y, cell_height, Row(cell.row), scale).pixels() - y;
    let bottom = row_edge(y, cell_height, Row(cell.row + 1), scale).pixels() - y;
    let image_left =
        px((f32::from(cell.column) - cell.image_column as f32) * f32::from(cell_width) + fit.left);
    let image_top =
        px((cell.row as f32 - cell.image_row as f32) * f32::from(cell_height) + fit.top);
    TileGeometry {
        left,
        top,
        width: right - left,
        height: bottom - top,
        image_left: image_left - left,
        image_top: image_top - top,
    }
}

fn rgb_id(color: SnapshotColor) -> Option<u32> {
    let SnapshotColor::Rgb(color) = color else {
        return None;
    };
    Some((u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b))
}

pub(super) fn image_cells<'a>(
    rows: &[std::sync::Arc<Vec<PositionedCell>>],
    placements: &'a [Placement],
) -> Vec<ImageCell<'a>> {
    let exact: HashMap<(u32, u32), &Placement> = placements
        .iter()
        .filter(|placement| placement.is_virtual)
        .map(|placement| ((placement.image, placement.placement), placement))
        .collect();
    if exact.is_empty() {
        return Vec::new();
    }
    let defaults: HashMap<u32, &Placement> = placements
        .iter()
        .filter(|placement| placement.is_virtual)
        .map(|placement| (placement.image, placement))
        .collect();
    let mut result = Vec::new();
    for (row, cells) in rows.iter().enumerate() {
        for cell in cells.iter() {
            let Some((image_row, image_column)) = coordinates(&cell.text) else {
                continue;
            };
            let Some(image_id) = rgb_id(cell.style.foreground) else {
                continue;
            };
            let placement_id = rgb_id(cell.style.underline_color).unwrap_or(0);
            let placement = exact.get(&(image_id, placement_id)).copied().or_else(|| {
                (placement_id == 0)
                    .then(|| defaults.get(&image_id).copied())
                    .flatten()
            });
            let Some(placement) = placement else {
                continue;
            };
            result.push(ImageCell {
                placement,
                column: cell.column,
                row,
                image_column,
                image_row,
            });
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::lay_out_row;
    use sprite_term::{
        CellStyle, CellWidth, Layer, Rectangle, RenderCell, RenderRow, Rgb, UnderlineStyle,
    };

    fn color(id: u32) -> SnapshotColor {
        SnapshotColor::Rgb(Rgb {
            r: (id >> 16) as u8,
            g: (id >> 8) as u8,
            b: id as u8,
        })
    }

    fn placement(image: u32, placement: u32) -> Placement {
        Placement {
            image,
            placement,
            is_virtual: true,
            layer: Layer::AboveText,
            source: Rectangle::default(),
            pixel_width: 40,
            pixel_height: 40,
            columns: 2,
            rows: 1,
            viewport_column: 0,
            viewport_row: 0,
            visible: false,
            x_offset: 0,
            y_offset: 0,
        }
    }

    fn row(fg: u32, placement_id: Option<u32>) -> RenderRow {
        let cell = |text: &str| RenderCell {
            text: text.into(),
            width: CellWidth::Narrow,
            selected: false,
            style: CellStyle {
                foreground: color(fg),
                background: SnapshotColor::Default,
                underline_color: placement_id.map(color).unwrap_or(SnapshotColor::Default),
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
        };
        RenderRow {
            cells: vec![
                cell("text"),
                cell("\u{10eeee}\u{0305}\u{0305}"),
                cell("\u{10eeee}\u{0305}\u{030d}"),
            ],
            wrapped: false,
        }
    }

    #[test]
    fn decodes_image_placeholder_coordinates() {
        assert_eq!(coordinates("\u{10eeee}\u{0305}\u{030d}"), Some((0, 1)));
        assert_eq!(coordinates("ordinary text"), None);
        assert_eq!(coordinates("\u{10eeee}\u{0305}"), None);
    }

    #[test]
    fn default_placement_matches_even_when_terminal_assigns_an_id() {
        let placements = [placement(1001, 3)];
        let cells = image_cells(&[lay_out_row(&row(1001, None)).into()], &placements);
        assert_eq!(cells.len(), 2);
        assert_eq!((cells[0].column, cells[0].image_column), (1, 0));
        assert_eq!((cells[1].column, cells[1].image_column), (2, 1));
        assert_eq!(cells[0].placement.placement, 3);
    }

    #[test]
    fn explicit_placement_requires_its_exact_id() {
        let placements = [placement(1001, 3)];
        assert_eq!(
            image_cells(&[lay_out_row(&row(1001, Some(4))).into()], &placements).len(),
            0
        );
        assert_eq!(
            image_cells(&[lay_out_row(&row(1001, Some(3))).into()], &placements).len(),
            2
        );
    }

    #[test]
    fn virtual_icons_keep_their_aspect_ratio_inside_cell_bounds() {
        let square = fit_image(40, 40, 30.0, 18.0).unwrap();
        assert_eq!(
            (square.left, square.top, square.width, square.height),
            (6.0, 0.0, 18.0, 18.0)
        );
        let wide = fit_image(40, 20, 30.0, 18.0).unwrap();
        assert_eq!(
            (wide.left, wide.top, wide.width, wide.height),
            (0.0, 1.5, 30.0, 15.0)
        );
        assert!(fit_image(0, 20, 30.0, 18.0).is_none());
    }

    /// Placeholder tiles are laid out by taffy, which rounds in device pixels.
    /// A tile whose edges and size are already whole device pixels, on the
    /// same grid the painter snaps cells to, lands exactly on its cell and
    /// meets its neighbour; the image inside is placed from the corner every
    /// tile of the placement shares, so it runs on without a step.
    #[test]
    fn placeholder_tiles_snap_to_the_cell_grid_and_share_one_image_origin() {
        use crate::grid::{Col, Row, column_edge, row_edge};
        let placement = placement(1001, 3);
        let (width, height, scale) = (px(8.4), px(16.8), 2.0);
        let fit = fit_image(40, 40, 2.0 * 8.4, 16.8).unwrap();
        let tile = |column: u16, image_column: u32| {
            tile_geometry(
                &ImageCell {
                    placement: &placement,
                    column,
                    row: 3,
                    image_column,
                    image_row: 0,
                },
                &fit,
                gpui::point(px(0.0), px(0.0)),
                width,
                height,
                scale,
            )
        };
        let (first, second) = (tile(5, 0), tile(6, 1));
        for value in [
            first.left,
            first.top,
            first.width,
            first.height,
            second.left,
            second.width,
        ] {
            let device = f32::from(value) * scale;
            assert!(
                (device - device.round()).abs() < 1e-3,
                "{value:?} is not on a device pixel"
            );
        }
        assert_eq!(
            first.left,
            column_edge(px(0.0), width, Col(5), scale).pixels()
        );
        assert_eq!(first.top, row_edge(px(0.0), height, Row(3), scale).pixels());
        assert_eq!(
            first.left + first.width,
            second.left,
            "neighbouring tiles meet"
        );
        let origin = |tile: TileGeometry| f32::from(tile.left + tile.image_left);
        assert!(
            (origin(first) - origin(second)).abs() < 1e-3,
            "both tiles place the image from the same corner"
        );
    }

    /// The painter snaps a cell's edges against where the grid sits in the
    /// window, so a grid whose corner is not on a device pixel has cells off
    /// the origin-0 grid. A tile is snapped the same way, then given relative
    /// to the grid's corner, so it still lands exactly on its cell.
    #[test]
    fn placeholder_tiles_snap_against_where_the_grid_sits_in_the_window() {
        use crate::grid::{Col, Row, column_edge, row_edge};
        let placement = placement(1001, 3);
        let (width, height, scale) = (px(8.4), px(16.8), 2.0);
        let grid = gpui::point(px(10.3), px(5.1));
        let fit = fit_image(40, 40, 2.0 * 8.4, 16.8).unwrap();
        let tile = tile_geometry(
            &ImageCell {
                placement: &placement,
                column: 5,
                row: 3,
                image_column: 0,
                image_row: 0,
            },
            &fit,
            grid,
            width,
            height,
            scale,
        );
        let painted = |edge: Pixels| f32::from(edge);
        let close = |a: f32, b: f32| (a - b).abs() < 1e-3;
        let left = column_edge(grid.x, width, Col(5), scale).pixels();
        let right = column_edge(grid.x, width, Col(6), scale).pixels();
        let top = row_edge(grid.y, height, Row(3), scale).pixels();
        let bottom = row_edge(grid.y, height, Row(4), scale).pixels();
        assert!(
            close(painted(grid.x + tile.left), painted(left)),
            "tile left {:?} against cell {left:?}",
            grid.x + tile.left
        );
        assert!(close(painted(grid.y + tile.top), painted(top)));
        assert!(close(painted(tile.width), painted(right - left)));
        assert!(close(painted(tile.height), painted(bottom - top)));
        // The picture still starts at the placement's own corner.
        assert!(close(
            painted(tile.left + tile.image_left),
            5.0 * 8.4 + fit.left
        ));
    }
}
