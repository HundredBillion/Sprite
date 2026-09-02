//! The observation endpoint's request grammar, in one place.
//!
//! `broker` promises that a request which could mutate cannot be constructed,
//! and that is true of everything it defines. The endpoint is where the whole
//! grammar meets — every read, and the one write — so it belongs beside that
//! promise rather than inside the window view, where an auditor reading
//! `broker.rs` would never find it.

use crate::observation::broker::{self, Denied, PaneSource, Refusal};
use crate::observation::endpoint::DENIED;
use crate::observation::schema;
use crate::workspace::ReloadRequest;

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
    let query = match broker::parse(body) {
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
    use super::{ConfigVerb, config_request, respond};

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
