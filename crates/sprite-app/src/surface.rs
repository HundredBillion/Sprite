//! Native Surfaces: what a program describes over the Surface Channel and
//! Sprite draws inside that program's pane.

pub mod channel;
pub mod client;
pub mod description;
pub mod grid;
pub mod host;
pub mod list;
pub mod render;
pub mod style;
mod wire;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DockSize(f32);

impl DockSize {
    pub fn pixels(self) -> f32 {
        self.0
    }
}

impl Default for DockSize {
    fn default() -> Self {
        Self(240.0)
    }
}

impl TryFrom<f32> for DockSize {
    type Error = &'static str;

    fn try_from(pixels: f32) -> Result<Self, Self::Error> {
        if (64.0..=4096.0).contains(&pixels) {
            Ok(Self(pixels))
        } else {
            Err("size is between 64 and 4096 pixels")
        }
    }
}

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
    Ineligible,
    TokenConflict(crate::tokens::TokenConflict),
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
            Refusal::Ineligible => "ineligible".to_owned(),
            Refusal::TokenConflict(conflict) => format!(
                "token conflict: {} already registered as #{:02x}{:02x}{:02x}",
                conflict.name, conflict.standing.r, conflict.standing.g, conflict.standing.b
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dock_size_accepts_only_finite_pixels_within_inclusive_bounds() {
        for (pixels, accepted) in [
            (1.0, false),
            (0.0, false),
            (-1.0, false),
            (f32::from_bits(64.0_f32.to_bits() - 1), false),
            (64.0, true),
            (240.5, true),
            (4096.0, true),
            (f32::from_bits(4096.0_f32.to_bits() + 1), false),
            (f32::NAN, false),
            (f32::INFINITY, false),
            (f32::NEG_INFINITY, false),
        ] {
            let size = DockSize::try_from(pixels);
            assert_eq!(size.is_ok(), accepted, "{pixels}");
            if let Ok(size) = size {
                assert_eq!(size.pixels(), pixels);
            }
        }
        assert_eq!(DockSize::default().pixels(), 240.0);
    }

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
            Refusal::Ineligible,
            Refusal::TokenConflict(crate::tokens::TokenConflict {
                name: "x".into(),
                standing: crate::tokens::unpack(0x12ab03),
            }),
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
