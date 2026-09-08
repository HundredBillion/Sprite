//! Native Surfaces: what a program describes over the Surface Channel and
//! Sprite draws inside that program's pane.

pub mod channel;
pub mod client;
pub mod description;
// Hosted by the terminal view, which follows.
#[allow(dead_code)]
pub mod grid;
pub mod host;
pub mod render;
pub mod style;

/// One Surface, for the life of the connection that opened it.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SurfaceId(pub u64);

/// Why a message was not acted on. Each is a distinct sentence, so a program
/// can branch on one without parsing prose.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Refusal {
    /// The one answer to a bad or missing key; it says nothing more.
    Denied,
    UnsupportedVersion,
    Malformed(String),
    UnknownKind(String),
    UnknownStyle(String),
    UnknownPane,
    NotATerminal,
    PositionOccupied,
    TokenConflict,
}

impl Refusal {
    pub fn reason(&self) -> String {
        match self {
            Refusal::Denied => "denied".to_owned(),
            Refusal::UnsupportedVersion => "unsupported version".to_owned(),
            Refusal::Malformed(why) => format!("malformed: {why}"),
            Refusal::UnknownKind(kind) => format!("unknown element kind: {kind}"),
            Refusal::UnknownStyle(token) => format!("unknown style token: {token}"),
            Refusal::UnknownPane => "unknown pane".to_owned(),
            Refusal::NotATerminal => "pane not a terminal".to_owned(),
            Refusal::PositionOccupied => "position occupied".to_owned(),
            Refusal::TokenConflict => "token conflict".to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_refusal_reason_is_distinct() {
        let reasons: Vec<String> = [
            Refusal::Denied,
            Refusal::UnsupportedVersion,
            Refusal::Malformed("x".into()),
            Refusal::UnknownKind("x".into()),
            Refusal::UnknownStyle("x".into()),
            Refusal::UnknownPane,
            Refusal::NotATerminal,
            Refusal::PositionOccupied,
            Refusal::TokenConflict,
        ]
        .iter()
        .map(Refusal::reason)
        .collect();
        let mut unique = reasons.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), reasons.len(), "{reasons:?}");
        assert_eq!(Refusal::PositionOccupied.reason(), "position occupied");
        assert_eq!(
            Refusal::Malformed("no root".into()).reason(),
            "malformed: no root"
        );
    }
}
