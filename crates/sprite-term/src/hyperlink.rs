use crate::CellPosition;
use libghostty_vt::Terminal;
/// Whether a hyperlink target may be offered to the application.
///
/// Compares the scheme case-insensitively and requires the `://` form, so a
/// value like `javascript:` or a bare path is refused rather than guessed at.
pub(crate) fn is_allowed_link(uri: &str) -> bool {
    let Some((scheme, rest)) = uri.split_once(':') else {
        return false;
    };
    if !rest.starts_with("//") {
        return false;
    }
    ALLOWED_LINK_SCHEMES
        .iter()
        .any(|allowed| scheme.eq_ignore_ascii_case(allowed))
}

/// Schemes a hyperlink may use.
///
/// Deliberately tiny. `file:`, bare paths, and application schemes stay out
/// until someone trusts them explicitly: an escape sequence is untrusted input,
/// and opening a local path or a custom handler on its say-so is how a terminal
/// becomes an execution vector.
const ALLOWED_LINK_SCHEMES: [&str; 2] = ["https", "http"];

/// The allowed hyperlink target at a cell, if any.
///
/// Returns `None` for a cell with no link and for any target the scheme policy
/// refuses, so a caller cannot distinguish "no link" from "denied" and act on
/// the difference.
pub(crate) struct ResolvedHyperlink {
    pub(crate) uri: String,
    pub(crate) span: crate::HyperlinkSpan,
}

/// A visible URL wins over everything: what the person can read is where the
/// click goes. Failing that, an OSC 8 label is a link only when its address is
/// shown nowhere on the row — see `titled_link_at`.
///
/// A visible URL may run across rows, as a long link does when the terminal
/// or the application printing it wraps it, so the rows around the clicked one
/// are read as they are needed. See `wrapped_url_at`.
pub(crate) fn resolve_hyperlink(
    terminal: &Terminal<'_, '_>,
    position: CellPosition,
) -> Option<ResolvedHyperlink> {
    let columns = usize::from(terminal.cols().ok()?);
    let height = terminal.rows().ok()?;
    let mut lines = Lines::new(|row| {
        (row < height)
            .then(|| read_line(terminal, row, columns))
            .flatten()
    });
    if let Some((uri, span)) = wrapped_url_at(&mut lines, position) {
        return Some(ResolvedHyperlink { uri, span });
    }
    let row = &lines.get(position.row)?.cells;
    let (uri, start, end) = titled_link_at(terminal, row, position)?;
    Some(ResolvedHyperlink {
        uri,
        span: crate::HyperlinkSpan::single(position.row, start as u16, end as u16),
    })
}

/// One visible row: its characters, and whether the terminal wrapped it onto
/// the next row because it ran out of columns.
struct Line {
    cells: Vec<char>,
    soft_wrapped: bool,
}

fn read_line(terminal: &Terminal<'_, '_>, row: u16, columns: usize) -> Option<Line> {
    use libghostty_vt::point::{Coordinate as PointCoordinate, Point};

    let mut cells = vec![' '; columns];
    let mut soft_wrapped = false;
    for (column, character) in cells.iter_mut().enumerate() {
        let cell = terminal
            .grid_ref(Point::Viewport(PointCoordinate {
                x: column as u16,
                y: u32::from(row),
            }))
            .ok()?;
        if column == 0 {
            soft_wrapped = cell
                .row()
                .and_then(|line| line.is_wrapped())
                .unwrap_or(false);
        }
        let mut graphemes = [' '; 8];
        if let Ok(count) = cell.graphemes(&mut graphemes)
            && let Some(first) = graphemes[..count].first()
        {
            *character = *first;
        }
    }
    Some(Line {
        cells,
        soft_wrapped,
    })
}

/// The visible rows, each read once and only when asked for.
struct Lines<F> {
    read: F,
    read_rows: std::collections::HashMap<u16, Option<Line>>,
}

impl<F: FnMut(u16) -> Option<Line>> Lines<F> {
    fn new(read: F) -> Self {
        Self {
            read,
            read_rows: std::collections::HashMap::new(),
        }
    }

    fn get(&mut self, row: u16) -> Option<&Line> {
        let read = &mut self.read;
        self.read_rows
            .entry(row)
            .or_insert_with(|| read(row))
            .as_ref()
    }
}

/// The most rows one URL may run across.
const MOST_ROWS: u16 = 8;

/// How many blank cells may sit between a cut URL and the edge of the text.
/// An application that wraps its own text, as Claude Code does, can leave a
/// column of padding before the edge of its pane.
const EDGE_MARGIN: usize = 2;

/// The visible URL under a cell, joined across the rows it wraps onto.
///
/// A URL continues on the next row in two cases. The terminal wrapped the row
/// itself: the next row goes on from its first cell. Or an application broke
/// its own line, which the terminal cannot see: then the URL must run to the
/// edge of its text, within `EDGE_MARGIN` of the last column or of a pane
/// border, and the next row must go on at or before the column the URL began
/// in, as wrapped text does under its indent. A URL that merely ends at a line's
/// end, short of the edge, is never joined to the row below.
fn wrapped_url_at<F: FnMut(u16) -> Option<Line>>(
    lines: &mut Lines<F>,
    position: CellPosition,
) -> Option<(String, crate::HyperlinkSpan)> {
    let lowest = position.row.saturating_sub(MOST_ROWS - 1);
    let mut row = position.row;
    loop {
        let starts: Vec<(usize, usize)> = url_starts(&lines.get(row)?.cells).collect();
        for (start, end) in starts {
            if let Some(link) = follow_url(lines, row, start, end)
                && link.1.contains(position)
            {
                return Some(link);
            }
        }
        // A URL that began higher up reaches this row only through the row
        // above it.
        if row == lowest || row == 0 || !lines.get(row - 1).is_some_and(reaches_edge) {
            return None;
        }
        row -= 1;
    }
}

/// One URL from where it starts, followed onto the rows it wraps onto.
fn follow_url<F: FnMut(u16) -> Option<Line>>(
    lines: &mut Lines<F>,
    row: u16,
    start: usize,
    end: usize,
) -> Option<(String, crate::HyperlinkSpan)> {
    let first = lines.get(row)?;
    let region_left = first.cells[..start]
        .iter()
        .rposition(|cell| is_border(*cell))
        .map_or(0, |border| border + 1);
    // (row, start, end) of each row's part of the URL.
    let mut parts = vec![(row, start, end)];
    let mut resume: Option<usize> = None;
    while parts.len() < usize::from(MOST_ROWS) {
        let Some(&(at, _, part_end)) = parts.last() else {
            break;
        };
        // Every row the URL is cut on is cut at the same column.
        if part_end != end {
            break;
        }
        let Some(line) = lines.get(at) else {
            break;
        };
        let soft = line.soft_wrapped && part_end == line.cells.len();
        if !soft && !cut_at_edge(&line.cells, part_end) {
            break;
        }
        let Some(next) = at.checked_add(1).and_then(|below| lines.get(below)) else {
            break;
        };
        let column = if soft {
            0
        } else {
            match next.cells[region_left..]
                .iter()
                .position(|cell| !cell.is_whitespace())
            {
                Some(offset) => region_left + offset,
                None => break,
            }
        };
        let fits = match resume {
            None => soft || column <= start,
            Some(earlier) => column == earlier,
        };
        if !fits
            || !next
                .cells
                .get(column)
                .is_some_and(|cell| is_url_cell(*cell))
        {
            break;
        }
        let mut part_end = column;
        while part_end < next.cells.len() && is_url_cell(next.cells[part_end]) {
            part_end += 1;
        }
        resume = Some(column);
        parts.push((at + 1, column, part_end));
    }
    // Closing punctuation belongs to the sentence, not the address.
    let mut text: Vec<Vec<char>> = Vec::with_capacity(parts.len());
    for &(at, from, to) in &parts {
        text.push(lines.get(at)?.cells[from..to].to_vec());
    }
    loop {
        let last = text.last_mut()?;
        while last.last().is_some_and(|cell| TRAILING.contains(*cell)) {
            last.pop();
        }
        if !last.is_empty() || text.len() == 1 {
            break;
        }
        text.pop();
        parts.pop();
    }
    let uri: String = text.iter().flatten().collect();
    if !is_allowed_link(&uri) {
        return None;
    }
    let last = parts.len() - 1;
    let (_, last_start, _) = parts[last];
    let last_end = last_start + text[last].len();
    let span = if last == 0 {
        crate::HyperlinkSpan::single(row, start as u16, last_end as u16)
    } else {
        crate::HyperlinkSpan {
            row,
            start_column: start as u16,
            end_column: last_end as u16,
            rows: parts.len() as u16,
            left_column: parts[1].1 as u16,
            right_column: end as u16,
        }
    };
    Some((uri, span))
}

/// Punctuation that ends a sentence rather than an address.
const TRAILING: &str = ".,!?;:)]}>";

/// Whether text running to `end` stops there because the row ran out: only
/// blanks, at most `EDGE_MARGIN` of them, lie between it and the last column
/// or a pane border.
fn cut_at_edge(cells: &[char], end: usize) -> bool {
    let mut edge = end;
    while edge < cells.len() && !is_border(cells[edge]) {
        if !cells[edge].is_whitespace() {
            return false;
        }
        edge += 1;
    }
    end > 0 && !cells[end - 1].is_whitespace() && edge - end <= EDGE_MARGIN
}

/// Whether any text on a row runs to an edge, so the row below may go on
/// with it.
fn reaches_edge(line: &Line) -> bool {
    let cells = &line.cells;
    let edges = cells
        .iter()
        .enumerate()
        .filter(|(_, cell)| is_border(**cell))
        .map(|(edge, _)| edge)
        .chain(std::iter::once(cells.len()));
    line.soft_wrapped
        || edges.into_iter().any(|edge| {
            let lowest = edge.saturating_sub(EDGE_MARGIN + 1);
            cells[lowest..edge]
                .iter()
                .rposition(|cell| !cell.is_whitespace())
                .is_some_and(|offset| cut_at_edge(cells, lowest + offset + 1))
        })
}

/// A cell a URL can be made of: not a blank, and not a pane border.
fn is_url_cell(cell: char) -> bool {
    !cell.is_whitespace() && !is_border(cell)
}

/// Box-drawing and block characters, which terminal multiplexers such as
/// herdr and tmux draw between panes.
fn is_border(cell: char) -> bool {
    matches!(cell, '\u{2500}'..='\u{259f}')
}

/// The OSC 8 target of a descriptive label, when the label is the only way to
/// reach it (issue #161).
///
/// Two refusals keep the label from misleading. Linked text that shows a URL
/// offers only that URL, so a label cannot display one address and open
/// another. And a label whose address is printed elsewhere on the row stays
/// plain text: the address is the link, and the label beside it is not a
/// second one (issue #96).
fn titled_link_at(
    terminal: &Terminal<'_, '_>,
    row: &[char],
    position: CellPosition,
) -> Option<(String, usize, usize)> {
    let column = usize::from(position.column);
    let target = osc8_target(terminal, position.row, column)?;
    if !is_allowed_link(&target) {
        return None;
    }
    let same_link = |column: usize| {
        osc8_target(terminal, position.row, column).is_some_and(|other| other == target)
    };
    let mut start = column;
    while start > 0 && same_link(start - 1) {
        start -= 1;
    }
    let mut end = column + 1;
    while end < row.len() && same_link(end) {
        end += 1;
    }
    let address_is_shown = visible_urls(row)
        .any(|(uri, url_start, url_end)| uri == target || (url_start < end && start < url_end));
    (!address_is_shown).then_some((target, start, end))
}

fn osc8_target(terminal: &Terminal<'_, '_>, row: u16, column: usize) -> Option<String> {
    use libghostty_vt::point::{Coordinate as PointCoordinate, Point};

    let cell = terminal
        .grid_ref(Point::Viewport(PointCoordinate {
            x: u16::try_from(column).ok()?,
            y: u32::from(row),
        }))
        .ok()?;
    let mut buffer = [0_u8; 2048];
    let written = cell.hyperlink_uri(&mut buffer).ok()?;
    if written == 0 {
        return None;
    }
    std::str::from_utf8(&buffer[..written])
        .ok()
        .map(str::to_owned)
}

#[cfg(test)]
fn url_at(row: &[char], column: usize) -> Option<(String, usize, usize)> {
    visible_urls(row).find(|(_, start, end)| *start <= column && column < *end)
}

/// Where each URL on a row starts, and where its cells end before closing
/// punctuation is taken off.
fn url_starts(row: &[char]) -> impl Iterator<Item = (usize, usize)> + '_ {
    (0..row.len()).filter_map(|start| {
        let scheme = ALLOWED_LINK_SCHEMES.iter().find(|scheme| {
            row.get(start..start + scheme.len() + 3)
                .is_some_and(|characters| {
                    characters
                        .iter()
                        .copied()
                        .eq(scheme.chars().chain("://".chars()))
                })
        })?;
        let mut end = start + scheme.len() + 3;
        while end < row.len() && is_url_cell(row[end]) {
            end += 1;
        }
        Some((start, end))
    })
}

/// Every allowed URL printed on a row, with its cell span.
fn visible_urls(row: &[char]) -> impl Iterator<Item = (String, usize, usize)> + '_ {
    url_starts(row).filter_map(|(start, mut end)| {
        while end > start && TRAILING.contains(row[end - 1]) {
            end -= 1;
        }
        let uri: String = row[start..end].iter().collect();
        is_allowed_link(&uri).then_some((uri, start, end))
    })
}

#[cfg(test)]
mod hyperlink_tests {
    use super::url_at;

    #[test]
    fn resolves_visible_http_url_at_clicked_cell() {
        let row: Vec<char> = "open https://example.com/path now".chars().collect();
        assert_eq!(
            url_at(&row, 15),
            Some(("https://example.com/path".to_owned(), 5, 29))
        );
    }

    #[test]
    fn excludes_trailing_punctuation_and_non_http_schemes() {
        let row: Vec<char> = "https://example.com/path, ftp://example.com"
            .chars()
            .collect();
        assert_eq!(
            url_at(&row, 10).map(|(uri, _, _)| uri),
            Some("https://example.com/path".to_owned())
        );
        assert_eq!(url_at(&row, 30), None);
    }

    #[test]
    fn resolves_the_url_under_the_column_when_a_row_has_multiple_urls() {
        let row: Vec<char> = "https://one.example https://two.example/path"
            .chars()
            .collect();
        let column = "https://one.example ".chars().count();

        assert_eq!(
            url_at(&row, column).map(|(uri, _, _)| uri),
            Some("https://two.example/path".to_owned())
        );
    }
}

#[cfg(test)]
mod link_tests {
    use super::is_allowed_link;

    #[test]
    fn ordinary_web_links_are_allowed() {
        assert!(is_allowed_link("https://example.com/page"));
        assert!(is_allowed_link("http://example.com"));
        assert!(is_allowed_link("HTTPS://example.com"));
    }

    #[test]
    fn everything_else_is_refused() {
        for refused in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,<script>",
            "vscode://open",
            "/etc/passwd",
            "example.com",
            "",
            "https:/example.com",
            "https:example.com",
        ] {
            assert!(!is_allowed_link(refused), "{refused} must be refused");
        }
    }
}

#[cfg(test)]
mod wrapped_url_tests {
    use super::{Line, Lines, wrapped_url_at};
    use crate::{CellPosition, HyperlinkSpan};

    /// A screen `width` columns wide. A row ending in `\u{21b5}` was wrapped
    /// by the terminal; the mark itself is not drawn.
    fn screen(width: usize, rows: &[&str]) -> Vec<Line> {
        rows.iter()
            .map(|row| {
                let soft_wrapped = row.ends_with('\u{21b5}');
                let mut cells: Vec<char> = row.trim_end_matches('\u{21b5}').chars().collect();
                cells.resize(width, ' ');
                Line {
                    cells,
                    soft_wrapped,
                }
            })
            .collect()
    }

    fn link(lines: &[Line], row: u16, column: u16) -> Option<(String, HyperlinkSpan)> {
        let mut lines = Lines::new(|at: u16| {
            lines.get(usize::from(at)).map(|line| Line {
                cells: line.cells.clone(),
                soft_wrapped: line.soft_wrapped,
            })
        });
        wrapped_url_at(&mut lines, CellPosition { row, column })
    }

    fn uri(lines: &[Line], row: u16, column: u16) -> Option<String> {
        link(lines, row, column).map(|(uri, _)| uri)
    }

    const PR: &str = "https://github.com/HundredBillion/scm.nvim/pull/21";

    /// Claude Code breaks its own lines: the terminal sees two rows, the first
    /// cut one column short of the edge, the second under the indent.
    fn claude_code() -> Vec<Line> {
        screen(
            49,
            &[
                "  I've opened the pull request: https://github.c ",
                "  om/HundredBillion/scm.nvim/pull/21",
                "",
                "  What's your next change?",
            ],
        )
    }

    #[test]
    fn a_url_an_application_broke_across_two_rows_opens_whole_from_either_row() {
        let lines = claude_code();
        assert_eq!(uri(&lines, 0, 40).as_deref(), Some(PR));
        assert_eq!(uri(&lines, 1, 2).as_deref(), Some(PR));
        assert_eq!(uri(&lines, 1, 30).as_deref(), Some(PR));
    }

    #[test]
    fn a_wrapped_url_covers_its_cells_on_each_row_and_nothing_else() {
        let (_, span) = link(&claude_code(), 0, 40).unwrap_or_default_link();
        assert_eq!(
            span,
            HyperlinkSpan {
                row: 0,
                start_column: 32,
                end_column: 36,
                rows: 2,
                left_column: 2,
                right_column: 48,
            }
        );
        assert!(span.contains(CellPosition { row: 0, column: 47 }));
        assert!(!span.contains(CellPosition { row: 0, column: 48 }));
        assert!(!span.contains(CellPosition { row: 1, column: 1 }));
        assert!(!span.contains(CellPosition { row: 1, column: 36 }));
        assert_eq!(uri(&claude_code(), 1, 40), None, "past the link's end");
        assert_eq!(uri(&claude_code(), 3, 4), None, "the next paragraph");
    }

    #[test]
    fn a_url_the_terminal_wrapped_follows_its_wrap_mark() {
        let lines = screen(20, &["see https://example.\u{21b5}", "com/a/b/c done"]);
        assert_eq!(
            uri(&lines, 0, 8).as_deref(),
            Some("https://example.com/a/b/c")
        );
        assert_eq!(
            uri(&lines, 1, 3).as_deref(),
            Some("https://example.com/a/b/c")
        );
        assert_eq!(uri(&lines, 1, 11), None);
    }

    #[test]
    fn a_url_runs_across_as_many_rows_as_it_was_cut_into() {
        let lines = screen(
            24,
            &[
                "  https://example.com/aa",
                "  aaaaaaaaaaaaaaaaaaaaaa",
                "  bbbb.",
            ],
        );
        let whole = "https://example.com/aaaaaaaaaaaaaaaaaaaaaaaabbbb";
        assert_eq!(
            link(&lines, 1, 2).map(|(_, span)| span.rows),
            Some(3),
            "three rows"
        );
        assert_eq!(uri(&lines, 2, 3).as_deref(), Some(whole));
        assert_eq!(uri(&lines, 0, 2).as_deref(), Some(whole));
    }

    #[test]
    fn a_url_that_ends_short_of_the_edge_is_not_joined_to_the_next_row() {
        let lines = screen(40, &["  Merged: https://example.com/pull/7", "  thanks"]);
        assert_eq!(
            uri(&lines, 0, 12).as_deref(),
            Some("https://example.com/pull/7")
        );
        assert_eq!(uri(&lines, 1, 3), None);
    }

    #[test]
    fn a_next_row_indented_past_where_the_url_began_is_not_its_end() {
        let lines = screen(30, &["https://example.com/abcdefghi ", "      code block"]);
        assert_eq!(
            uri(&lines, 0, 3).as_deref(),
            Some("https://example.com/abcdefghi")
        );
    }

    #[test]
    fn a_url_cut_at_a_pane_border_continues_in_its_own_pane() {
        let lines = screen(
            40,
            &[
                "  go https://example.com/a \u{2502} other pane",
                "  bc/d                     \u{2502} more text",
            ],
        );
        assert_eq!(
            uri(&lines, 0, 6).as_deref(),
            Some("https://example.com/abc/d")
        );
        assert_eq!(uri(&lines, 1, 32), None);
    }

    #[test]
    fn a_url_beside_a_pane_border_does_not_take_the_border() {
        let lines = screen(40, &["https://example.com/x\u{2502}right pane"]);
        assert_eq!(uri(&lines, 0, 3).as_deref(), Some("https://example.com/x"));
    }

    #[test]
    fn closing_punctuation_on_the_last_row_is_left_out() {
        let lines = screen(24, &["(see https://example.co ", "m/x/y)."]);
        assert_eq!(
            uri(&lines, 1, 1).as_deref(),
            Some("https://example.com/x/y")
        );
    }

    trait OrDefaultLink {
        fn unwrap_or_default_link(self) -> (String, HyperlinkSpan);
    }

    impl OrDefaultLink for Option<(String, HyperlinkSpan)> {
        fn unwrap_or_default_link(self) -> (String, HyperlinkSpan) {
            self.unwrap_or_else(|| (String::new(), HyperlinkSpan::single(u16::MAX, 0, 0)))
        }
    }
}
