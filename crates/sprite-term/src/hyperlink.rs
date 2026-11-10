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
pub(crate) fn resolve_hyperlink(
    terminal: &Terminal<'_, '_>,
    position: CellPosition,
) -> Option<ResolvedHyperlink> {
    use libghostty_vt::terminal::{Point, PointCoordinate};

    let columns = usize::from(terminal.cols().ok()?);
    let mut row = vec![' '; columns];
    for (column, character) in row.iter_mut().enumerate() {
        let cell = terminal
            .grid_ref(Point::Viewport(PointCoordinate {
                x: column as u16,
                y: u32::from(position.row),
            }))
            .ok()?;
        let mut graphemes = [' '; 8];
        if let Ok(count) = cell.graphemes(&mut graphemes)
            && let Some(first) = graphemes[..count].first()
        {
            *character = *first;
        }
    }
    let (uri, start, end) = url_at(&row, usize::from(position.column))
        .or_else(|| titled_link_at(terminal, &row, position))?;
    Some(ResolvedHyperlink {
        uri,
        span: crate::HyperlinkSpan {
            row: position.row,
            start_column: start as u16,
            end_column: end as u16,
        },
    })
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
    use libghostty_vt::terminal::{Point, PointCoordinate};

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

fn url_at(row: &[char], column: usize) -> Option<(String, usize, usize)> {
    visible_urls(row).find(|(_, start, end)| *start <= column && column < *end)
}

/// Every allowed URL printed on a row, with its cell span.
fn visible_urls(row: &[char]) -> impl Iterator<Item = (String, usize, usize)> + '_ {
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
        while end < row.len() && !row[end].is_whitespace() {
            end += 1;
        }
        while end > start && ".,!?;:)]}>".contains(row[end - 1]) {
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
