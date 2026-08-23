//! The versioned JSON a client actually receives.
//!
//! **Every field is written out by hand.** Nothing here is derived from a Rust
//! type, because a derive includes whatever the type happens to hold: add a
//! field to a snapshot for the renderer's benefit and it would silently appear
//! on the wire. The PRD's exclusion list — screenshots, colours, fonts, raw
//! control sequences, clipboard data, environment values, image bytes, decoded
//! pixels, filenames — is therefore enforced by construction: those things
//! cannot leak because no line below writes them.
//!
//! Snapshots are untrusted data. Every pane declares
//! `content_trust: "untrusted_terminal_output"`, and Sprite does not classify,
//! redact, or neutralise what a terminal displayed. A client that feeds this to
//! a language model is feeding it text an arbitrary program chose.

use serde_json::{Map, Value, json};

use crate::observation::broker::{Failure, PaneReport, Report};

/// The schema clients pin against.
///
/// Bump only for a change a client cannot ignore. Pretty printing is whitespace
/// and never a second schema, so it does not bump anything.
pub const SCHEMA_VERSION: u32 = 1;

/// What every pane's content is, stated rather than implied.
const CONTENT_TRUST: &str = "untrusted_terminal_output";

/// Renders a report as the versioned response.
///
/// `pretty` changes whitespace only: both forms carry the same fields in the
/// same order, so a client that reads one can read the other.
pub fn render(report: &Report, pretty: bool) -> String {
    let document = document(report);
    if pretty {
        serde_json::to_string_pretty(&document)
    } else {
        serde_json::to_string(&document)
    }
    // A document built from typed data cannot fail to serialise; an empty
    // object is still valid JSON, so even an impossible failure is never a
    // partially written response.
    .unwrap_or_else(|_| "{}".to_owned())
}

fn document(report: &Report) -> Value {
    let mut root = Map::new();
    root.insert("schema_version".to_owned(), json!(SCHEMA_VERSION));
    // A multi-pane answer captures each pane independently and never pauses the
    // window, so panes may differ by a few milliseconds. Saying so is why each
    // pane carries its own capture time and the response claims no instant.
    root.insert("complete".to_owned(), json!(report.complete));
    root.insert(
        "panes".to_owned(),
        Value::Array(report.panes.iter().map(pane).collect()),
    );
    root.insert(
        "errors".to_owned(),
        Value::Array(report.failures.iter().map(failure).collect()),
    );
    Value::Object(root)
}

fn pane(report: &PaneReport) -> Value {
    let snapshot = &report.snapshot;
    let address = &report.address;

    let mut object = Map::new();
    object.insert("pane".to_owned(), json!(address.pane.0));
    object.insert("tab".to_owned(), json!(address.tab.0));
    // Tabs have no titles yet; `null` rather than the active pane's title,
    // which would be a guess dressed as a fact.
    object.insert("tab_title".to_owned(), Value::Null);
    object.insert("focused".to_owned(), json!(address.focused));
    object.insert("content_trust".to_owned(), json!(CONTENT_TRUST));

    // Normalised, so a client learns left/right and above/below without being
    // coupled to pixels, DPI, or a monitor size.
    object.insert(
        "layout".to_owned(),
        json!({
            "x": address.rect.x,
            "y": address.rect.y,
            "width": address.rect.width,
            "height": address.rect.height,
        }),
    );

    object.insert(
        "size".to_owned(),
        json!({ "columns": snapshot.size.cols, "rows": snapshot.size.rows }),
    );
    object.insert(
        "cursor".to_owned(),
        json!({
            "row": snapshot.cursor.row,
            "column": snapshot.cursor.column,
            "visible": snapshot.cursor.visible,
        }),
    );
    object.insert(
        "viewport".to_owned(),
        json!({
            "total_rows": snapshot.viewport.total_rows,
            "offset": snapshot.viewport.offset,
            "visible_rows": snapshot.viewport.visible_rows,
        }),
    );
    object.insert(
        "screen".to_owned(),
        json!(match snapshot.screen {
            sprite_term::ScreenKind::Primary => "primary",
            sprite_term::ScreenKind::Alternate => "alternate",
        }),
    );
    object.insert(
        "history".to_owned(),
        json!({
            "requested": snapshot.requested,
            "returned": snapshot.history_rows,
            "available": snapshot.available,
        }),
    );
    object.insert("generation".to_owned(), json!(snapshot.generation));
    object.insert(
        "captured_at_unix_ms".to_owned(),
        // Beyond an f64's exact integer range is decades away, but a JSON
        // number that lost precision would be a wrong timestamp rather than a
        // missing one, so it is emitted as an integer.
        json!(u64::try_from(snapshot.captured_at_unix_ms).unwrap_or(u64::MAX)),
    );

    object.insert("title".to_owned(), optional(snapshot.title.as_deref()));
    object.insert(
        "working_directory".to_owned(),
        optional(snapshot.working_directory.as_deref()),
    );
    // Basename or null. Never arguments, never environment values, and never a
    // guess from what happens to be displayed.
    object.insert(
        "foreground_executable".to_owned(),
        optional(snapshot.foreground.as_deref()),
    );

    object.insert(
        "rows".to_owned(),
        Value::Array(
            snapshot
                .rows
                .iter()
                .map(|row| {
                    json!({
                        "text": row.text,
                        "wrapped": row.wrapped,
                        "prompt": match row.prompt {
                            sprite_term::PromptKind::None => "none",
                            sprite_term::PromptKind::Prompt => "prompt",
                            sprite_term::PromptKind::Continuation => "continuation",
                        },
                    })
                })
                .collect(),
        ),
    );

    Value::Object(object)
}

fn failure(failure: &Failure) -> Value {
    json!({
        "pane": failure.address.pane.0,
        "tab": failure.address.tab.0,
        "error": failure.kind.as_str(),
        "detail": failure.reason,
    })
}

fn optional(value: Option<&str>) -> Value {
    // `null` rather than an empty string: "the child never said" and "the child
    // said nothing" are different facts.
    value.map_or(Value::Null, |value| json!(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observation::broker::{FailureKind, PaneAddress, order_for_schema};
    use crate::pane_tree::{PaneId, Rect};
    use crate::tabs::TabId;
    use sprite_term::{
        CursorSnapshot, HistorySnapshot, PaneRow, PromptKind, ScreenKind, TerminalSize, Viewport,
    };
    use std::sync::Arc;

    fn snapshot(text: &str) -> Arc<HistorySnapshot> {
        Arc::new(HistorySnapshot {
            generation: 7,
            size: TerminalSize::DEFAULT,
            screen: ScreenKind::Primary,
            rows: vec![PaneRow {
                text: text.to_owned(),
                wrapped: false,
                prompt: PromptKind::Prompt,
            }],
            history_rows: 1,
            requested: 500,
            available: 12,
            cursor: CursorSnapshot {
                row: 3,
                column: 4,
                visible: true,
                blinking: false,
            },
            viewport: Viewport {
                total_rows: 40,
                offset: 2,
                visible_rows: 24,
            },
            title: Some("a title".to_owned()),
            working_directory: Some("/home/somebody".to_owned()),
            captured_at_unix_ms: 1_800_000_000_000,
            foreground: Some("vim".to_owned()),
        })
    }

    fn address(tab: u64, tab_order: usize, pane: u64, y: f32, x: f32) -> PaneAddress {
        PaneAddress {
            tab: TabId(tab),
            tab_order,
            pane: PaneId(pane),
            rect: Rect {
                x,
                y,
                width: 0.5,
                height: 0.5,
            },
            focused: pane == 0,
        }
    }

    fn report_of(panes: Vec<PaneReport>, failures: Vec<Failure>) -> Report {
        Report {
            complete: failures.is_empty(),
            panes,
            failures,
        }
    }

    fn parse(text: &str) -> Value {
        serde_json::from_str(text).expect("the response is valid JSON")
    }

    #[test]
    fn the_response_is_one_versioned_object_with_a_panes_array() {
        let report = report_of(
            vec![PaneReport {
                address: address(0, 0, 0, 0.0, 0.0),
                snapshot: snapshot("hello"),
            }],
            Vec::new(),
        );

        let value = parse(&render(&report, false));
        assert_eq!(value["schema_version"], json!(SCHEMA_VERSION));
        assert_eq!(value["complete"], json!(true));
        assert!(value["panes"].is_array());
        assert_eq!(value["panes"][0]["rows"][0]["text"], json!("hello"));
    }

    /// Pretty printing is whitespace. Two renderings of one report must parse
    /// to the same document, or there would be two schemas.
    #[test]
    fn pretty_printing_changes_whitespace_and_nothing_else() {
        let report = report_of(
            vec![PaneReport {
                address: address(0, 0, 1, 0.0, 0.0),
                snapshot: snapshot("same"),
            }],
            Vec::new(),
        );

        let compact = render(&report, false);
        let pretty = render(&report, true);

        assert_ne!(compact, pretty, "they do differ, in whitespace");
        assert!(pretty.contains('\n'), "the pretty form is laid out");
        assert!(!compact.contains('\n'), "the compact form is one line");
        assert_eq!(
            parse(&compact),
            parse(&pretty),
            "and they are the same document"
        );
    }

    /// Concurrent completion order must not change serialisation: the order is
    /// tabs by window order, then top edge, then left edge, then pane ID.
    #[test]
    fn ordering_is_by_layout_and_not_by_completion_order() {
        // Deliberately built in a scrambled order, as concurrent captures would
        // finish in.
        let panes = vec![
            PaneReport {
                address: address(1, 1, 9, 0.0, 0.0),
                snapshot: snapshot("second tab"),
            },
            PaneReport {
                address: address(0, 0, 5, 0.5, 0.0),
                snapshot: snapshot("lower"),
            },
            PaneReport {
                address: address(0, 0, 4, 0.0, 0.5),
                snapshot: snapshot("upper right"),
            },
            PaneReport {
                address: address(0, 0, 3, 0.0, 0.0),
                snapshot: snapshot("upper left"),
            },
        ];
        let mut report = report_of(panes, Vec::new());
        // The window's own ordering, not a copy of it written for the test.
        order_for_schema(&mut report);
        let value = parse(&render(&report, false));

        let order: Vec<u64> = value["panes"]
            .as_array()
            .expect("an array")
            .iter()
            .map(|pane| pane["pane"].as_u64().expect("a pane id"))
            .collect();
        assert_eq!(
            order,
            vec![3, 4, 5, 9],
            "tab order, then top edge, then left edge, then id"
        );
    }

    #[test]
    fn every_pane_declares_its_content_untrusted() {
        let report = report_of(
            vec![
                PaneReport {
                    address: address(0, 0, 0, 0.0, 0.0),
                    snapshot: snapshot("one"),
                },
                PaneReport {
                    address: address(0, 0, 1, 0.5, 0.0),
                    snapshot: snapshot("two"),
                },
            ],
            Vec::new(),
        );

        let value = parse(&render(&report, false));
        for pane in value["panes"].as_array().expect("an array") {
            assert_eq!(pane["content_trust"], json!("untrusted_terminal_output"));
        }
    }

    /// The exclusions are a promise about what cannot leak, so they are checked
    /// against the rendered text rather than against the types.
    #[test]
    fn the_response_excludes_everything_it_promises_to() {
        let report = report_of(
            vec![PaneReport {
                address: address(0, 0, 0, 0.0, 0.0),
                snapshot: snapshot("text"),
            }],
            Vec::new(),
        );
        let rendered = render(&report, true);
        let value = parse(&rendered);

        for forbidden in [
            "screenshot",
            "colour",
            "color",
            "foreground_color",
            "background",
            "font",
            "style",
            "bold",
            "italic",
            "underline",
            "escape",
            "control_sequence",
            "raw",
            "clipboard",
            "environment",
            "env",
            "image",
            "pixels",
            "bytes",
            "filename",
            "files",
        ] {
            assert!(
                !rendered.contains(forbidden),
                "the response must not mention {forbidden:?}"
            );
        }

        // Positively: a pane carries exactly the agreed keys, so a field added
        // to a snapshot for the renderer cannot arrive here unnoticed.
        let mut keys: Vec<&str> = value["panes"][0]
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "captured_at_unix_ms",
                "content_trust",
                "cursor",
                "focused",
                "foreground_executable",
                "generation",
                "history",
                "layout",
                "pane",
                "rows",
                "screen",
                "size",
                "tab",
                "tab_title",
                "title",
                "viewport",
                "working_directory",
            ]
        );
    }

    #[test]
    fn a_missing_foreground_executable_is_null_rather_than_a_guess() {
        let mut bare = (*snapshot("text")).clone();
        bare.foreground = None;
        bare.title = None;
        bare.working_directory = None;
        let report = report_of(
            vec![PaneReport {
                address: address(0, 0, 0, 0.0, 0.0),
                snapshot: Arc::new(bare),
            }],
            Vec::new(),
        );

        let value = parse(&render(&report, false));
        assert_eq!(value["panes"][0]["foreground_executable"], Value::Null);
        assert_eq!(value["panes"][0]["title"], Value::Null);
        assert_eq!(value["panes"][0]["working_directory"], Value::Null);
    }

    #[test]
    fn failures_are_named_and_do_not_discard_healthy_panes() {
        let report = report_of(
            vec![PaneReport {
                address: address(0, 0, 0, 0.0, 0.0),
                snapshot: snapshot("healthy"),
            }],
            vec![
                Failure {
                    address: address(0, 0, 1, 0.5, 0.0),
                    kind: FailureKind::Timeout,
                    reason: "the pane did not answer within the deadline".to_owned(),
                },
                Failure {
                    address: address(0, 0, 2, 0.5, 0.5),
                    kind: FailureKind::Closed,
                    reason: "the pane closed before it answered".to_owned(),
                },
            ],
        );

        let value = parse(&render(&report, false));
        assert_eq!(value["complete"], json!(false));
        assert_eq!(value["panes"].as_array().expect("array").len(), 1);
        assert_eq!(value["errors"][0]["error"], json!("pane_timeout"));
        assert_eq!(value["errors"][0]["pane"], json!(1));
        assert_eq!(value["errors"][1]["error"], json!("pane_closed"));
    }

    /// Terminal text is arbitrary bytes chosen by an arbitrary program. It must
    /// survive as data rather than escaping into the document's structure.
    #[test]
    fn terminal_text_cannot_break_out_of_the_document() {
        let hostile = "\" , \"injected\": true, \\ \u{1b}[31m \u{7} </script> \u{202e}";
        let mut snapshot = (*snapshot("placeholder")).clone();
        snapshot.rows[0].text = hostile.to_owned();
        let report = report_of(
            vec![PaneReport {
                address: address(0, 0, 0, 0.0, 0.0),
                snapshot: Arc::new(snapshot),
            }],
            Vec::new(),
        );

        let value = parse(&render(&report, false));
        assert_eq!(
            value["panes"][0]["rows"][0]["text"],
            json!(hostile),
            "the text round-trips exactly"
        );
        assert!(
            value["panes"][0].get("injected").is_none(),
            "and invents no field"
        );
    }
}
