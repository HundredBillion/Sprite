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
///
/// **Adding `placements` did not bump it.** A new key is ignorable by
/// definition: a client reading the keys it knows sees exactly what it saw
/// before, and one that wants images now has them. Bumping for an addition
/// would make the version a change counter rather than a compatibility signal,
/// and a client pinning it would be forced to re-pin for something that cannot
/// affect it. What *would* bump it is a key changing meaning, changing type, or
/// going away.
pub const SCHEMA_VERSION: u32 = 1;

/// What every pane's content is, stated rather than implied.
const CONTENT_TRUST: &str = "untrusted_terminal_output";

/// The most encoded JSON one response may be.
pub const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

/// How many times a response is re-encoded while being brought under the limit.
///
/// Shedding is planned from measured row sizes, so one pass is normally enough;
/// the rest exist because a plan is an estimate and the guarantee is not.
const SHEDDING_PASSES: usize = 8;

/// Renders a report as the versioned response.
///
/// `pretty` changes whitespace only: both forms carry the same fields in the
/// same order, so a client that reads one can read the other.
pub fn render(report: &Report, pretty: bool) -> String {
    render_limited(report, pretty, MAX_RESPONSE_BYTES)
}

/// Renders a report within `limit` bytes of encoded JSON.
///
/// **The output is always a complete document.** Nothing is ever cut from the
/// encoded text: a response is brought under the limit by removing whole rows
/// and whole panes from the data and encoding again, so a client never receives
/// half a string, half a row, or half an object. Cutting encoded bytes would be
/// the obvious implementation and would produce malformed JSON at exactly the
/// moment a client is least able to cope.
///
/// What goes first, in order:
///
/// 1. history, oldest first, because it is the least current thing present;
/// 2. whole panes, because half a screen is worse than a named omission.
///
/// Metadata and the complete current screen outrank history, so a pane that
/// stays is a pane a client can trust to be whole.
pub fn render_limited(report: &Report, pretty: bool, limit: usize) -> String {
    let mut plan = Plan::new(report);
    for _ in 0..SHEDDING_PASSES {
        let encoded = encode(&document(report, &plan), pretty);
        if encoded.len() <= limit {
            return encoded;
        }
        // Nothing left to remove: the floor below is all that can be offered.
        if !plan.shed(report, encoded.len(), limit) {
            break;
        }
    }

    let encoded = encode(&document(report, &plan), pretty);
    if encoded.len() <= limit {
        return encoded;
    }
    // Everything has been shed and it still does not fit, which means the limit
    // is smaller than an empty answer. A valid document that reports the
    // failure beats a truncated one: "never malformed" outranks the size.
    encode(&floor(report), pretty)
}

fn encode(document: &Value, pretty: bool) -> String {
    if pretty {
        serde_json::to_string_pretty(document)
    } else {
        serde_json::to_string(document)
    }
    // A document built from typed data cannot fail to serialise; an empty
    // object is still valid JSON, so even an impossible failure is never a
    // partially written response.
    .unwrap_or_else(|_| "{}".to_owned())
}

/// The smallest honest answer: no panes, and every one of them named as omitted.
fn floor(report: &Report) -> Value {
    let mut root = Map::new();
    root.insert("schema_version".to_owned(), json!(SCHEMA_VERSION));
    root.insert("complete".to_owned(), json!(false));
    root.insert("panes".to_owned(), Value::Array(Vec::new()));
    root.insert(
        "errors".to_owned(),
        Value::Array(
            report
                .panes
                .iter()
                .map(|pane| limit_error(&pane.address))
                .chain(report.failures.iter().map(failure))
                .collect(),
        ),
    );
    Value::Object(root)
}

/// How much of each pane a response will carry.
struct Plan {
    /// Per pane, how many of its oldest history rows have been dropped.
    dropped: Vec<usize>,
    /// Per pane, whether it has been left out entirely.
    omitted: Vec<bool>,
    /// Per pane, the encoded size of one row, used to plan the next pass.
    row_cost: Vec<usize>,
}

impl Plan {
    fn new(report: &Report) -> Self {
        let row_cost = report
            .panes
            .iter()
            .map(|pane| {
                // Measured rather than guessed: a row of CJK text costs several
                // times a row of ASCII, and a plan built on the wrong number
                // simply means another pass.
                let rows = pane.snapshot.rows.len().max(1);
                let sampled = serde_json::to_string(&rows_value(&pane.snapshot.rows, 0))
                    .map(|text| text.len())
                    .unwrap_or(rows * 64);
                (sampled / rows).max(16)
            })
            .collect();
        Self {
            dropped: vec![0; report.panes.len()],
            omitted: vec![false; report.panes.len()],
            row_cost,
        }
    }

    /// Removes enough to close the gap, returning false when nothing is left.
    ///
    /// History and whole panes are shed in **separate passes**, never in one.
    /// A pass plans from estimated row sizes, and an estimate that overshoots
    /// would otherwise omit a pane that the re-encode was about to show fits
    /// comfortably — losing a whole screen to a rounding margin. So dropping
    /// history always returns, and a pane is only ever omitted after a real
    /// measurement of a response that already carries no history at all.
    fn shed(&mut self, report: &Report, encoded: usize, limit: usize) -> bool {
        let excess = encoded.saturating_sub(limit);
        // A small margin, because another pass costs a full encode of a
        // response that may be megabytes.
        let wanted = excess + excess / 16 + 64;

        if self.fattest_history(report).is_some() {
            // Water-filling: find the largest number of history rows every pane
            // may keep such that the total freed covers the excess, then bring
            // each pane down to it. Taking the whole excess from one pane would
            // be simpler and would strip the last panes bare while the first
            // kept everything — a response where some panes have full history
            // and others none is worse than one where all are trimmed equally.
            let remaining: Vec<usize> = (0..report.panes.len())
                .map(|index| report.panes[index].snapshot.history_rows - self.dropped[index])
                .collect();
            let freed_if_capped_at = |cap: usize| -> usize {
                remaining
                    .iter()
                    .enumerate()
                    .map(|(index, rows)| rows.saturating_sub(cap) * self.row_cost[index].max(1))
                    .sum()
            };

            let mut low = 0;
            let mut high = remaining.iter().copied().max().unwrap_or(0);
            while low < high {
                let middle = low + (high - low).div_ceil(2);
                if freed_if_capped_at(middle) >= wanted {
                    low = middle;
                } else {
                    high = middle - 1;
                }
            }
            for (index, rows) in remaining.iter().enumerate() {
                self.dropped[index] += rows.saturating_sub(low);
            }
            return true;
        }

        // No history remains anywhere and it still does not fit, so whole panes
        // go — the last in schema order first, so the panes a client asked
        // about most directly survive. Never half a screen.
        let mut freed = 0;
        let mut omitted_any = false;
        for index in (0..report.panes.len()).rev() {
            if self.omitted[index] {
                continue;
            }
            self.omitted[index] = true;
            omitted_any = true;
            let rows = report.panes[index].snapshot.rows.len() - self.dropped[index];
            freed += rows * self.row_cost[index].max(1);
            if freed >= wanted {
                break;
            }
        }
        omitted_any
    }

    /// The pane still carrying the most history, if any does.
    fn fattest_history(&self, report: &Report) -> Option<usize> {
        report
            .panes
            .iter()
            .enumerate()
            .filter(|(index, _)| !self.omitted[*index])
            .map(|(index, pane)| (index, pane.snapshot.history_rows - self.dropped[index]))
            .filter(|(_, remaining)| *remaining > 0)
            .max_by_key(|(_, remaining)| *remaining)
            .map(|(index, _)| index)
    }
}

fn document(report: &Report, plan: &Plan) -> Value {
    let mut root = Map::new();
    root.insert("schema_version".to_owned(), json!(SCHEMA_VERSION));
    // A multi-pane answer captures each pane independently and never pauses the
    // window, so panes may differ by a few milliseconds. Saying so is why each
    // pane carries its own capture time and the response claims no instant.
    let omitted: Vec<&PaneReport> = report
        .panes
        .iter()
        .enumerate()
        .filter(|(index, _)| plan.omitted[*index])
        .map(|(_, pane)| pane)
        .collect();
    // A response that left something out is not complete, whatever the panes
    // that survived say.
    root.insert(
        "complete".to_owned(),
        json!(report.complete && omitted.is_empty()),
    );
    root.insert(
        "panes".to_owned(),
        Value::Array(
            report
                .panes
                .iter()
                .enumerate()
                .filter(|(index, _)| !plan.omitted[*index])
                .map(|(index, report)| pane(report, plan.dropped[index]))
                .collect(),
        ),
    );
    root.insert(
        "errors".to_owned(),
        Value::Array(
            report
                .failures
                .iter()
                .map(failure)
                .chain(omitted.iter().map(|pane| limit_error(&pane.address)))
                .collect(),
        ),
    );
    Value::Object(root)
}

fn limit_error(address: &crate::observation::broker::PaneAddress) -> Value {
    json!({
        "pane": address.pane.0,
        "tab": address.tab.0,
        "error": "response_limit",
        "detail": "omitted whole rather than returned in part, to keep the response within its size limit",
    })
}

fn pane(report: &PaneReport, dropped: usize) -> Value {
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
    let dropped = dropped.min(snapshot.history_rows);
    object.insert(
        "history".to_owned(),
        json!({
            "requested": snapshot.requested,
            "returned": snapshot.history_rows - dropped,
            "available": snapshot.available,
            "dropped_for_size": dropped,
        }),
    );
    // Stated rather than left to be inferred from arithmetic: a client reading
    // a truncated pane should not have to work out that it is one.
    object.insert("truncated".to_owned(), json!(dropped > 0));
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

    // Images occupying terminal space, as metadata only. There is no field
    // here for bytes, pixels, or a filename, and none can be added by
    // accident: a pane's key set is asserted exactly, and so is this one.
    object.insert(
        "placements".to_owned(),
        Value::Array(
            snapshot
                .placements
                .iter()
                .map(|placement| {
                    json!({
                        "placement": placement.placement,
                        "image": placement.image,
                        "virtual": placement.is_virtual,
                        "z_order": match placement.layer {
                            sprite_term::Layer::BelowBackground => "below_background",
                            sprite_term::Layer::BelowText => "below_text",
                            sprite_term::Layer::AboveText => "above_text",
                        },
                        "transmission_format": match placement.format {
                            sprite_term::TransmittedFormat::Rgb => "rgb",
                            sprite_term::TransmittedFormat::Rgba => "rgba",
                            sprite_term::TransmittedFormat::Png => "png",
                            sprite_term::TransmittedFormat::Gray => "gray",
                            sprite_term::TransmittedFormat::GrayAlpha => "gray_alpha",
                            sprite_term::TransmittedFormat::Unknown => "unknown",
                        },
                        "pixel_size": {
                            "width": placement.image_width,
                            "height": placement.image_height,
                        },
                        "cells": {
                            "columns": placement.columns,
                            "rows": placement.rows,
                            "column": placement.viewport_column,
                            "row": placement.viewport_row,
                        },
                        "visible": placement.visible,
                    })
                })
                .collect(),
        ),
    );

    // Whole rows only. Dropping happens at row boundaries, which is also what
    // keeps every emitted row's Unicode intact: a row is never cut, so no
    // character can be halved.
    object.insert("rows".to_owned(), rows_value(&snapshot.rows, dropped));

    Value::Object(object)
}

fn rows_value(rows: &[sprite_term::PaneRow], dropped: usize) -> Value {
    Value::Array(
        rows.iter()
            .skip(dropped)
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
    )
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
                style: Default::default(),
            },
            viewport: Viewport {
                total_rows: 40,
                offset: 2,
                visible_rows: 24,
            },
            title: Some("a title".to_owned()),
            working_directory: Some("/home/somebody".to_owned()),
            placements: Vec::new(),
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
                "placements",
                "rows",
                "screen",
                "size",
                "tab",
                "tab_title",
                "title",
                "truncated",
                "viewport",
                "working_directory",
            ]
        );
    }

    /// A placement tells a client that an image occupies terminal space, and
    /// nothing that reproduces the picture.
    #[test]
    fn a_placement_reports_where_an_image_is_and_not_what_it_looks_like() {
        let mut snapshot = (*snapshot("text")).clone();
        snapshot.placements = vec![sprite_term::PlacementMetadata {
            image: 4,
            placement: 9,
            is_virtual: false,
            layer: sprite_term::Layer::AboveText,
            format: sprite_term::TransmittedFormat::Png,
            image_width: 640,
            image_height: 480,
            columns: 80,
            rows: 30,
            viewport_column: 2,
            viewport_row: -3,
            visible: true,
        }];
        let report = report_of(
            vec![PaneReport {
                address: address(0, 0, 0, 0.0, 0.0),
                snapshot: Arc::new(snapshot),
            }],
            Vec::new(),
        );

        let value = parse(&render(&report, false));
        let placement = &value["panes"][0]["placements"][0];
        assert_eq!(placement["image"], json!(4));
        assert_eq!(placement["placement"], json!(9));
        assert_eq!(placement["z_order"], json!("above_text"));
        assert_eq!(placement["transmission_format"], json!("png"));
        assert_eq!(placement["pixel_size"]["width"], json!(640));
        assert_eq!(placement["cells"]["columns"], json!(80));
        assert_eq!(
            placement["cells"]["row"],
            json!(-3),
            "a placement partly scrolled off the top keeps its negative row"
        );

        // The agreed key set, so a field added to the graphics projection
        // cannot reach the wire unnoticed.
        let mut keys: Vec<&str> = placement
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "cells",
                "image",
                "pixel_size",
                "placement",
                "transmission_format",
                "virtual",
                "visible",
                "z_order",
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

    // ---- response limiting ----------------------------------------------

    /// A snapshot with `history` rows of history in front of `screen` rows of
    /// current screen, each row `width` characters wide.
    fn bulky(history: usize, screen: usize, width: usize, fill: char) -> Arc<HistorySnapshot> {
        let mut base = (*snapshot("placeholder")).clone();
        base.rows = (0..history + screen)
            .map(|index| PaneRow {
                text: std::iter::repeat_n(fill, width)
                    .chain(format!("{index}").chars())
                    .collect(),
                wrapped: false,
                prompt: PromptKind::None,
            })
            .collect();
        base.history_rows = history;
        base.available = history;
        base.requested = history;
        Arc::new(base)
    }

    fn one_pane(snapshot: Arc<HistorySnapshot>) -> Report {
        report_of(
            vec![PaneReport {
                address: address(0, 0, 0, 0.0, 0.0),
                snapshot,
            }],
            Vec::new(),
        )
    }

    #[test]
    fn a_response_within_the_limit_is_untouched() {
        let report = one_pane(bulky(10, 5, 20, 'a'));
        let value = parse(&render_limited(&report, false, MAX_RESPONSE_BYTES));

        assert_eq!(value["panes"][0]["truncated"], json!(false));
        assert_eq!(value["panes"][0]["history"]["dropped_for_size"], json!(0));
        assert_eq!(
            value["panes"][0]["rows"].as_array().expect("rows").len(),
            15
        );
        assert_eq!(value["complete"], json!(true));
    }

    /// The boundary the task names: a pane whose history alone exceeds the
    /// limit. The screen must survive whole, and the JSON must still parse.
    #[test]
    fn a_pane_whose_history_exceeds_the_limit_keeps_its_screen() {
        let report = one_pane(bulky(4_000, 24, 200, 'x'));
        let limit = 64 * 1024;

        let rendered = render_limited(&report, false, limit);
        assert!(
            rendered.len() <= limit,
            "the response is within its limit: {} > {limit}",
            rendered.len()
        );
        let value = parse(&rendered);

        let pane = &value["panes"][0];
        assert_eq!(pane["truncated"], json!(true));
        assert!(
            pane["history"]["dropped_for_size"]
                .as_u64()
                .expect("a count")
                > 0,
            "history was what went"
        );
        let rows = pane["rows"].as_array().expect("rows");
        assert!(
            rows.len() >= 24,
            "the whole current screen survived: {} rows",
            rows.len()
        );
        // The rows kept are the newest, because the oldest go first.
        let last = rows.last().expect("a last row")["text"]
            .as_str()
            .expect("text");
        assert!(last.ends_with(&format!("{}", 4_000 + 24 - 1)));
    }

    /// Rows are dropped whole, so no character is ever halved — the reason
    /// shedding happens in the data rather than in the encoded bytes.
    #[test]
    fn shedding_never_cuts_a_row_or_a_character() {
        // Wide characters, so a byte-wise cut would leave broken UTF-8.
        let report = one_pane(bulky(2_000, 24, 120, '界'));
        let rendered = render_limited(&report, false, 96 * 1024);
        let value = parse(&rendered);

        let original = &report.panes[0].snapshot.rows;
        let dropped = value["panes"][0]["history"]["dropped_for_size"]
            .as_u64()
            .expect("a count") as usize;
        for (offset, row) in value["panes"][0]["rows"]
            .as_array()
            .expect("rows")
            .iter()
            .enumerate()
        {
            assert_eq!(
                row["text"].as_str().expect("text"),
                original[dropped + offset].text,
                "every emitted row is one of the original rows, entire"
            );
        }
    }

    /// When even the screens do not fit, whole panes are left out and named —
    /// never half a screen.
    #[test]
    fn panes_are_omitted_whole_and_reported_as_response_limit() {
        let panes: Vec<PaneReport> = (0..4)
            .map(|index| PaneReport {
                address: address(0, 0, index, index as f32 / 10.0, 0.0),
                snapshot: bulky(0, 200, 300, 'y'),
            })
            .collect();
        let report = report_of(panes, Vec::new());

        let limit = 96 * 1024;
        let rendered = render_limited(&report, false, limit);
        assert!(rendered.len() <= limit, "within the limit");
        let value = parse(&rendered);

        let kept = value["panes"].as_array().expect("panes");
        assert!(!kept.is_empty(), "something survived");
        assert!(kept.len() < 4, "and something did not");
        for pane in kept {
            assert_eq!(
                pane["rows"].as_array().expect("rows").len(),
                200,
                "a pane that survived kept its whole screen"
            );
        }
        assert_eq!(value["complete"], json!(false));

        let errors = value["errors"].as_array().expect("errors");
        assert_eq!(errors.len(), 4 - kept.len(), "every omission is named");
        for error in errors {
            assert_eq!(error["error"], json!("response_limit"));
        }
        // The panes that survive are the first in schema order.
        let surviving: Vec<u64> = kept
            .iter()
            .map(|pane| pane["pane"].as_u64().expect("id"))
            .collect();
        assert_eq!(surviving, (0..kept.len() as u64).collect::<Vec<_>>());
    }

    /// The guarantee that outranks every other: whatever the limit, the output
    /// parses.
    #[test]
    fn the_output_is_valid_json_at_every_limit() {
        let report = report_of(
            vec![
                PaneReport {
                    address: address(0, 0, 0, 0.0, 0.0),
                    snapshot: bulky(500, 40, 80, 'z'),
                },
                PaneReport {
                    address: address(0, 0, 1, 0.5, 0.0),
                    snapshot: bulky(500, 40, 80, '界'),
                },
            ],
            vec![Failure {
                address: address(0, 0, 2, 0.9, 0.0),
                kind: FailureKind::Timeout,
                reason: "the pane did not answer within the deadline".to_owned(),
            }],
        );

        for limit in [0, 1, 40, 200, 1_000, 8_000, 64_000, 512_000, 8_000_000] {
            for pretty in [false, true] {
                let rendered = render_limited(&report, pretty, limit);
                // Parsing is the assertion: malformed output cannot pass.
                let value = parse(&rendered);
                assert_eq!(value["schema_version"], json!(SCHEMA_VERSION));
                assert!(
                    value["panes"].is_array() && value["errors"].is_array(),
                    "the shape survives at limit {limit}"
                );
                // Below the size of an empty document nothing can be promised
                // except that it is valid JSON, which is what was just checked.
                if limit >= 4_000 {
                    assert!(
                        rendered.len() <= limit,
                        "limit {limit} pretty={pretty}: got {} bytes",
                        rendered.len()
                    );
                }
            }
        }
    }

    /// The real limit, at the scale it exists for: several panes each holding
    /// the maximum history, which together far exceed 16 MiB.
    #[test]
    fn the_real_limit_holds_at_the_scale_it_exists_for() {
        let panes: Vec<PaneReport> = (0..4)
            .map(|index| PaneReport {
                address: address(0, 0, index, index as f32 / 10.0, 0.0),
                // 5,000 history rows of wide characters, each row 400 columns:
                // about 6 MiB of encoded JSON per pane, so four cannot fit.
                snapshot: bulky(5_000, 40, 400, '界'),
            })
            .collect();
        let report = report_of(panes, Vec::new());

        let started = std::time::Instant::now();
        let rendered = render(&report, false);
        let took = started.elapsed();

        assert!(
            rendered.len() <= MAX_RESPONSE_BYTES,
            "{} bytes exceeds the 16 MiB limit",
            rendered.len()
        );
        let value = parse(&rendered);

        // History is what goes, so all four panes survive with their screens
        // whole. `complete` is about panes being present, not about history
        // being full — a shed pane says so with `truncated` and
        // `dropped_for_size`, which is a different fact from a missing pane.
        assert_eq!(value["panes"].as_array().expect("panes").len(), 4);
        assert_eq!(value["complete"], json!(true));
        // Equal treatment: no pane keeps its full history while another is
        // stripped bare.
        let dropped: Vec<u64> = value["panes"]
            .as_array()
            .expect("panes")
            .iter()
            .map(|pane| pane["history"]["dropped_for_size"].as_u64().expect("count"))
            .collect();
        let smallest = dropped.iter().min().copied().expect("a pane");
        let largest = dropped.iter().max().copied().expect("a pane");
        assert!(
            largest - smallest <= 1,
            "every pane gave up the same history: {dropped:?}"
        );

        for pane in value["panes"].as_array().expect("panes") {
            assert_eq!(pane["truncated"], json!(true));
            assert!(pane["history"]["dropped_for_size"].as_u64().expect("count") > 0);
            assert_eq!(
                pane["rows"].as_array().expect("rows").len() as u64,
                pane["history"]["returned"].as_u64().expect("returned") + 40,
                "the 40-row current screen is whole in every pane"
            );
        }
        // Deliberately generous: these tests run unoptimised, where encoding
        // several megabytes is many times slower than in the build a user runs.
        // The number worth quoting is measured in release, not here.
        assert!(took < std::time::Duration::from_secs(10), "took {took:?}");
        eprintln!(
            "  16 MiB limit: {} bytes, {} panes kept, {} dropped rows, took {took:?}",
            rendered.len(),
            value["panes"].as_array().expect("panes").len(),
            value["panes"][0]["history"]["dropped_for_size"]
        );
    }

    #[test]
    fn an_impossible_limit_still_answers_with_a_valid_document() {
        let report = one_pane(bulky(100, 24, 80, 'q'));
        let value = parse(&render_limited(&report, false, 1));

        assert_eq!(value["complete"], json!(false));
        assert!(value["panes"].as_array().expect("panes").is_empty());
        assert_eq!(value["errors"][0]["error"], json!("response_limit"));
    }
}
