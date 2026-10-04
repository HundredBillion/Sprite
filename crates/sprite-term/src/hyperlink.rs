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

pub(crate) fn resolve_hyperlink(
    terminal: &Terminal<'_, '_>,
    position: CellPosition,
) -> Option<ResolvedHyperlink> {
    use libghostty_vt::terminal::{Point, PointCoordinate};

    let grid_ref = terminal
        .grid_ref(Point::Viewport(PointCoordinate {
            x: position.column,
            y: u32::from(position.row),
        }))
        .ok()?;

    let mut buffer = [0_u8; 2048];
    if let Ok(written) = grid_ref.hyperlink_uri(&mut buffer)
        && written != 0
        && let Ok(uri) = std::str::from_utf8(&buffer[..written])
        && crate::is_allowed_link(uri)
    {
        let columns = usize::from(terminal.cols().ok()?);
        let mut start = usize::from(position.column);
        let mut end = start + 1;
        while start > 0 && hyperlink_uri_matches(terminal, position.row, start - 1, uri) {
            start -= 1;
        }
        while end < columns && hyperlink_uri_matches(terminal, position.row, end, uri) {
            end += 1;
        }
        return Some(ResolvedHyperlink {
            uri: uri.to_owned(),
            span: crate::HyperlinkSpan {
                row: position.row,
                start_column: start as u16,
                end_column: end as u16,
            },
        });
    }

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
    url_at(&row, usize::from(position.column)).map(|(uri, start, end)| ResolvedHyperlink {
        uri,
        span: crate::HyperlinkSpan {
            row: position.row,
            start_column: start as u16,
            end_column: end as u16,
        },
    })
}

fn hyperlink_uri_matches(
    terminal: &Terminal<'_, '_>,
    row: u16,
    column: usize,
    expected: &str,
) -> bool {
    use libghostty_vt::terminal::{Point, PointCoordinate};
    let Ok(cell) = terminal.grid_ref(Point::Viewport(PointCoordinate {
        x: column as u16,
        y: u32::from(row),
    })) else {
        return false;
    };
    let mut buffer = [0_u8; 2048];
    cell.hyperlink_uri(&mut buffer)
        .is_ok_and(|written| written != 0 && &buffer[..written] == expected.as_bytes())
}

fn url_at(row: &[char], column: usize) -> Option<(String, usize, usize)> {
    for start in 0..row.len() {
        let Some(scheme) = ALLOWED_LINK_SCHEMES.iter().find(|scheme| {
            row.get(start..start + scheme.len() + 3)
                .is_some_and(|characters| {
                    characters
                        .iter()
                        .copied()
                        .eq(scheme.chars().chain("://".chars()))
                })
        }) else {
            continue;
        };
        let mut end = start + scheme.len() + 3;
        while end < row.len() && !row[end].is_whitespace() {
            end += 1;
        }
        while end > start && ".,!?;:)]}>".contains(row[end - 1]) {
            end -= 1;
        }
        if start <= column && column < end {
            let uri: String = row[start..end].iter().collect();
            if crate::is_allowed_link(&uri) {
                return Some((uri, start, end));
            }
        }
    }
    None
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
