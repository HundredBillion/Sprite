//! The observation endpoint's request grammar, in one place, in both
//! directions.
//!
//! A request crosses two processes as a line of text, and this module is the
//! only description of what that line may say: `parse` reads one a client sent,
//! `render` writes one a client is about to send, and the types between them
//! are shared with the command line that fills them in. Two separate
//! descriptions of a scope — one for the flags a person types, one for the
//! words that go over the socket — is what lets an option be added to one and
//! forgotten in the other.
//!
//! It sits beside `broker` rather than inside the window view because `broker`
//! promises that a request which could mutate cannot be constructed, and this
//! is where that promise is kept: every read, and the one write, meet here, so
//! an auditor reading `broker.rs` finds the whole grammar next to it.

use sprite_term::HistoryLines;

use crate::observation::broker::{self, Denied, PaneSource, Refusal};
use crate::observation::endpoint::DENIED;
use crate::observation::schema;
use crate::pane_tree::PaneId;
use crate::workspace::ReloadRequest;

/// Which panes a caller asked about.
///
/// Every variant reads. There is deliberately no variant that writes, sends
/// input, subscribes, or opens a stream: a request that could mutate cannot be
/// constructed, so no code downstream has to refuse one.
///
/// That is a promise about this type, not about everything the socket accepts.
/// The endpoint also takes one verb that writes — a configuration reload — and
/// `respond`, below, turns it away before it can reach these types. Anyone
/// auditing what a request can do therefore has to read this whole module,
/// which is where the grammar lives and where that one write is handled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scope {
    /// The requester's own tab. The default, and by default without the
    /// requester itself — a pane asking "what else is going on" rarely means
    /// its own output.
    Tab { include_self: bool },
    /// One named pane.
    Pane(PaneId),
    /// Every pane in the window. Never beyond it.
    Window,
}

impl Default for Scope {
    /// What a request that names no scope at all means: the caller's own tab,
    /// without the caller. Spelled once here so the command line and the wire
    /// parser cannot drift about what "no flags" is short for.
    fn default() -> Self {
        Self::Tab {
            include_self: false,
        }
    }
}

/// A parsed, authorised request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Query {
    /// The pane the caller says it is.
    ///
    /// Self-reported, and only used to shape the default scope. It is not a
    /// privilege: see the threat model at the top of `broker`.
    pub from: Option<PaneId>,
    pub scope: Scope,
    pub lines: HistoryLines,
    /// Lay the JSON out for a human. Whitespace only; never a second schema.
    pub pretty: bool,
}

/// Reads a request body into a query.
///
/// The grammar is deliberately tiny. Anything unrecognised is refused rather
/// than ignored, so a client cannot smuggle a verb past a lenient parser.
pub fn parse(body: &str) -> Result<Query, Refusal> {
    let mut words = body.split_whitespace().peekable();
    // The protocol token is optional so that a client older than this window
    // is understood rather than refused; a *newer* one names a version this
    // window does not know, and is told so.
    if let Some(word) = words.peek()
        && word.starts_with("sprite-observation/")
    {
        let spoken = *word;
        words.next();
        if spoken != broker::PROTOCOL {
            return Err(Refusal::UnsupportedProtocol);
        }
    }
    match (words.next(), words.next()) {
        (Some("panes"), Some("snapshot")) => {}
        _ => return Err(Refusal::Malformed("the only request is: panes snapshot")),
    }

    let mut from = None;
    let mut scope = None;
    let mut include_self = false;
    let mut pretty = false;
    let mut lines = HistoryLines::default();

    while let Some(word) = words.next() {
        match word {
            "--include-self" => include_self = true,
            "--pretty" => pretty = true,
            "--window" => {
                if scope.is_some() {
                    return Err(Refusal::Malformed("scope given twice"));
                }
                scope = Some(Scope::Window);
            }
            "--pane" => {
                if scope.is_some() {
                    return Err(Refusal::Malformed("scope given twice"));
                }
                let value = words
                    .next()
                    .ok_or(Refusal::Malformed("--pane needs a number"))?;
                let pane = value
                    .parse()
                    .map_err(|_| Refusal::Malformed("--pane needs a number"))?;
                scope = Some(Scope::Pane(PaneId(pane)));
            }
            "--from" => {
                let value = words
                    .next()
                    .ok_or(Refusal::Malformed("--from needs a number"))?;
                let pane = value
                    .parse()
                    .map_err(|_| Refusal::Malformed("--from needs a number"))?;
                from = Some(PaneId(pane));
            }
            "--lines" => {
                let value = words
                    .next()
                    .ok_or(Refusal::Malformed("--lines needs a number"))?;
                let count: usize = value
                    .parse()
                    .map_err(|_| Refusal::Malformed("--lines needs a number"))?;
                // Clamped, not refused, exactly as the extraction path does.
                lines = HistoryLines::new(count);
            }
            _ => return Err(Refusal::Malformed("unknown option")),
        }
    }

    if include_self && !matches!(scope, None | Some(Scope::Tab { .. })) {
        return Err(Refusal::Malformed("--include-self only applies to a tab"));
    }

    Ok(Query {
        from,
        scope: scope.unwrap_or(Scope::Tab { include_self }),
        lines,
        pretty,
    })
}

/// Renders a query as the wire text a client sends.
///
/// The inverse of `parse`, and kept beside it so the two cannot drift: this is
/// the seam between the two processes, and both sides of it are now written
/// from the same type.
///
/// The word order is the order the client assembled by hand before this
/// function existed. `parse` accepts the flags in any order, so nothing here
/// depends on it — but a window is long-lived and the command is a fresh
/// process each time, so an upgraded client routinely talks to a window that
/// started before it. Keep sending the line that has always been sent.
pub fn render(query: &Query) -> String {
    let mut text = format!("{} panes snapshot", broker::PROTOCOL);
    if let Some(from) = query.from {
        text.push_str(&format!(" --from {}", from.0));
    }
    match query.scope {
        // The default scope is written by saying nothing, which is what makes
        // the common request short.
        Scope::Tab {
            include_self: false,
        } => {}
        Scope::Tab { include_self: true } => text.push_str(" --include-self"),
        Scope::Pane(pane) => text.push_str(&format!(" --pane {}", pane.0)),
        Scope::Window => text.push_str(" --window"),
    }
    // Said only when it differs from what a silent request would get, so that
    // asking for the usual amount of history still sends the usual short line.
    //
    // That rests on `HistoryLines::DEFAULT` being the same number at both ends
    // of the socket. It is today, and while it holds the omission is exact. But
    // this line may be read by a window that started before the client was
    // built, so changing that constant would mean somebody who explicitly typed
    // the old default silently receives the new one: the number is no longer on
    // the wire to say otherwise. Changing `DEFAULT` is therefore a change to the
    // protocol, not an implementation detail, and needs the version to say so.
    if query.lines != HistoryLines::default() {
        text.push_str(&format!(" --lines {}", query.lines.get()));
    }
    if query.pretty {
        text.push_str(" --pretty");
    }
    text
}

/// The two things a shell command can ask about this window's configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConfigVerb {
    Reload,
    Print,
}

/// How long an endpoint thread will wait for the window to answer a reload.
const RELOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Answers one authenticated request.
///
/// Runs on an endpoint thread, never the GPUI thread: a request must not be
/// able to hold up drawing, and the deadline inside `collect` is what keeps it
/// from holding up the endpoint either.
pub(crate) fn respond(
    panes: &dyn PaneSource,
    reload: &async_channel::Sender<ReloadRequest>,
    body: &str,
) -> String {
    // One check, both verbs. Previously `broker::parse` compared the token to
    // PROTOCOL while `config_request` discarded it, so a newer client's config
    // reload was honoured and only its snapshot request refused — the write
    // verb being the one that got through.
    let body = match protocol_check(body) {
        Ok(rest) => rest,
        Err(_) => {
            return format!(
                "unsupported protocol; this window speaks {}",
                broker::PROTOCOL
            );
        }
    };
    // One verb that is not a question about panes. It is authenticated by the
    // same key and reachable only from inside this window, which is the same
    // rule observation lives by: a caller that could not read this window's
    // panes cannot reload its settings either.
    if let Some(verb) = config_request(body) {
        return ask_window(reload, verb);
    }
    let query = match parse(body) {
        Ok(query) => query,
        // A malformed request describes the caller's own words and reveals
        // nothing about the window's contents, so it may say so.
        Err(Refusal::Malformed(why)) => return format!("malformed: {why}"),
        Err(Refusal::UnsupportedProtocol) => {
            return format!(
                "unsupported protocol; this window speaks {}",
                broker::PROTOCOL
            );
        }
        Err(Refusal::Denied) => return DENIED.to_owned(),
    };
    match broker::collect(&query, panes, broker::DEADLINE) {
        Ok(report) => schema::render(&report, query.pretty),
        Err(Denied) => DENIED.to_owned(),
    }
}

/// Validates and strips the optional protocol token.
///
/// Optional so that a client *older* than this window is understood rather than
/// refused. A *newer* one names a version this window does not know and is told
/// so — for every verb, which is the whole point of checking here rather than
/// inside each parser.
fn protocol_check(body: &str) -> Result<&str, Refusal> {
    let trimmed = body.trim_start();
    if !trimmed.starts_with("sprite-observation/") {
        return Ok(body);
    }
    let (token, rest) = trimmed
        .split_once(char::is_whitespace)
        .unwrap_or((trimmed, ""));
    if token != broker::PROTOCOL {
        return Err(Refusal::UnsupportedProtocol);
    }
    Ok(rest)
}

/// Which configuration verb a request body is, if it is one at all.
///
/// The protocol token is optional here for the same reason it is in the pane
/// parser: a client older than this window should be understood. It is only
/// ever *dropped*, though, not merely stepped over — a leading word that
/// looks like a protocol token but names a version this window does not
/// speak is left in place, so it lands in the `config`-verb position, fails
/// to match, and the request falls through to `broker::parse`, which does
/// compare it and refuses. Skipping any `sprite-observation/`-shaped word
/// unconditionally, as this once did, let a second embedded token carry an
/// unchecked version past this function and into `ask_window`.
fn config_request(body: &str) -> Option<ConfigVerb> {
    let mut words = body.split_whitespace().peekable();
    if let Some(word) = words.peek()
        && *word == broker::PROTOCOL
    {
        words.next();
    }
    match (words.next(), words.next(), words.next()) {
        (Some("config"), Some("reload"), None) => Some(ConfigVerb::Reload),
        (Some("config"), Some("print"), None) => Some(ConfigVerb::Print),
        _ => None,
    }
}

/// Hands the question to the GPUI thread and waits, briefly, for its answer.
fn ask_window(reload: &async_channel::Sender<ReloadRequest>, what: ConfigVerb) -> String {
    let (reply, answer) = std::sync::mpsc::sync_channel(1);
    if reload.send_blocking(ReloadRequest { what, reply }).is_err() {
        return "this window is no longer answering".to_owned();
    }
    match answer.recv_timeout(RELOAD_TIMEOUT) {
        Ok(answer) => answer,
        Err(_) => "this window did not answer in time; nothing was changed".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{ConfigVerb, Query, Scope, config_request, parse, render, respond};
    use crate::pane_tree::PaneId;
    use sprite_term::HistoryLines;

    /// The grammar has two directions and they must be the same grammar.
    ///
    /// This is the test that could not be written while `cli` and `broker` each
    /// defined their own idea of a scope: there was no single value to send one
    /// way and read back the other. It walks every scope against every
    /// combination of the remaining flags rather than a hand-picked few, because
    /// a disagreement between the directions would otherwise hide in whichever
    /// combination nobody thought to list.
    #[test]
    fn every_request_survives_the_wire_and_back() {
        let scopes = [
            Scope::Tab {
                include_self: false,
            },
            Scope::Tab { include_self: true },
            Scope::Pane(PaneId(7)),
            Scope::Window,
        ];
        // The default is included because it is the one length `render`
        // deliberately leaves off the wire, and both ends of the clamp because
        // a value that survives one direction may not survive the other.
        let lengths = [
            HistoryLines::default(),
            HistoryLines::new(0),
            HistoryLines::new(12),
            HistoryLines::new(HistoryLines::MAX),
        ];

        for scope in scopes {
            for from in [None, Some(PaneId(4))] {
                for pretty in [false, true] {
                    for lines in lengths {
                        let original = Query {
                            from,
                            scope,
                            lines,
                            pretty,
                        };
                        let text = render(&original);
                        let parsed = parse(&text).unwrap_or_else(|error| {
                            panic!("{text:?} did not parse back: {error:?}")
                        });
                        assert_eq!(parsed, original, "round trip changed {text:?}");
                    }
                }
            }
        }
    }

    /// Two history lengths the client used to send verbatim are normalised
    /// before they leave, because a `Query` holds a `HistoryLines` and that type
    /// has no way to say "the usual amount" or to hold more than the maximum:
    /// the default goes unsaid, and an over-large request is clamped here
    /// instead of at the window. Neither changes what the window does, and this
    /// is what says so.
    #[test]
    fn a_normalised_history_length_still_means_what_it_always_meant() {
        let asked_for = |text: &str| parse(text).expect("a request").lines;

        assert_eq!(
            asked_for("panes snapshot"),
            asked_for(&format!("panes snapshot --lines {}", HistoryLines::DEFAULT))
        );
        assert_eq!(
            asked_for(&format!("panes snapshot --lines {}", HistoryLines::MAX + 1)),
            asked_for(&format!("panes snapshot --lines {}", HistoryLines::MAX))
        );
    }

    /// Self-consistency is not compatibility: the window is still spoken to by
    /// clients built before this module owned the grammar. These are the exact
    /// lines the client used to assemble by hand, minus the key it puts in
    /// front, so a change of word order here fails rather than reaching a
    /// released window.
    #[test]
    fn the_rendered_text_is_what_the_client_has_always_sent() {
        let asked = |scope, from, lines, pretty| {
            render(&Query {
                from,
                scope,
                lines,
                pretty,
            })
        };
        let tab = Scope::Tab {
            include_self: false,
        };

        assert_eq!(
            asked(tab, Some(PaneId(4)), HistoryLines::default(), false),
            "sprite-observation/1 panes snapshot --from 4"
        );
        assert_eq!(
            asked(Scope::Window, Some(PaneId(0)), HistoryLines::new(12), true),
            "sprite-observation/1 panes snapshot --from 0 --window --lines 12 --pretty"
        );
        assert_eq!(
            asked(
                Scope::Pane(PaneId(9)),
                Some(PaneId(0)),
                HistoryLines::default(),
                false
            ),
            "sprite-observation/1 panes snapshot --from 0 --pane 9"
        );
        assert_eq!(
            asked(
                Scope::Tab { include_self: true },
                Some(PaneId(0)),
                HistoryLines::default(),
                false
            ),
            "sprite-observation/1 panes snapshot --from 0 --include-self"
        );
        assert_eq!(
            asked(Scope::Window, None, HistoryLines::default(), false),
            "sprite-observation/1 panes snapshot --window"
        );
    }

    #[test]
    fn a_configuration_request_is_told_from_a_pane_query() {
        assert_eq!(config_request("config reload"), Some(ConfigVerb::Reload));
        assert_eq!(
            config_request("sprite-observation/1 config reload"),
            Some(ConfigVerb::Reload)
        );
        assert_eq!(
            config_request("  config   print  "),
            Some(ConfigVerb::Print)
        );

        assert_eq!(config_request("panes snapshot"), None);
        assert_eq!(config_request("config"), None);
        assert_eq!(config_request("config reload --now"), None);
        assert_eq!(config_request(""), None);
    }

    /// A `PaneSource` with nothing in it. A refused request never reaches a
    /// pane, so `begin` is unreachable.
    struct NoPanes;

    impl crate::observation::broker::PaneSource for NoPanes {
        fn panes(&self) -> Vec<crate::observation::broker::PaneAddress> {
            Vec::new()
        }

        fn begin(
            &self,
            _pane: crate::pane_tree::PaneId,
            _lines: sprite_term::HistoryLines,
        ) -> Result<crate::observation::broker::Pending, String> {
            unreachable!("a refused request never reaches a pane")
        }
    }

    /// The divergence: `config reload` is a write, and it was the verb that got
    /// through. Both verbs must refuse a version this window does not speak.
    #[test]
    fn a_newer_protocol_is_refused_for_every_verb() {
        let (reload, _keep_open) = async_channel::bounded(1);

        for body in [
            "sprite-observation/99 config reload",
            "sprite-observation/99 panes snapshot",
        ] {
            let answer = respond(&NoPanes, &reload, body);
            assert!(
                answer.starts_with("unsupported protocol"),
                "{body:?} was answered with {answer:?}"
            );
        }
    }

    /// A second, embedded token once slipped past `config_request` unchecked:
    /// the first token satisfied `protocol_check` and was stripped, leaving a
    /// *second* `sprite-observation/`-shaped word that `config_request` used
    /// to skip on sight rather than compare. Both verbs must still refuse.
    #[test]
    fn a_second_embedded_protocol_token_is_also_refused() {
        let (reload, _keep_open) = async_channel::bounded(1);

        for body in [
            "sprite-observation/1 sprite-observation/999 config reload",
            "sprite-observation/1 sprite-observation/999 panes snapshot",
        ] {
            let answer = respond(&NoPanes, &reload, body);
            assert!(
                answer.starts_with("unsupported protocol"),
                "{body:?} was answered with {answer:?}"
            );
        }
    }

    /// The version this window does speak still reaches the parser.
    #[test]
    fn the_spoken_protocol_still_reaches_the_parser() {
        let (reload, _keep_open) = async_channel::bounded(1);
        let answer = respond(
            &NoPanes,
            &reload,
            "sprite-observation/1 panes snapshot --window",
        );
        assert!(
            !answer.starts_with("unsupported protocol"),
            "the current protocol was refused: {answer:?}"
        );
    }
}
