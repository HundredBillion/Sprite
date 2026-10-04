//! The utility tokens a Surface Description may use for style, and what each
//! one does to a GPUI element.
//!
//! This table is the whole vocabulary. Tokens are Tailwind's names spelt with
//! underscores, because that is what GPUI's `Styled` methods are called, and a
//! token does exactly one thing: call the method of the same name. Sprite
//! builds no cascade and no selector engine; a program that wants a layout
//! says which flexbox properties it wants, as Zed's own views do.

use gpui::{DefiniteLength, FontWeight, Length, Styled, rems};

/// Tokens that stand alone, in the order a reader might look for them.
#[cfg(test)] // Read only by `vocabulary`, which is test-only.
const FIXED: [&str; 41] = [
    "flex",
    "flex_col",
    "flex_row",
    "flex_1",
    "flex_none",
    "flex_wrap",
    "items_start",
    "items_center",
    "items_end",
    "justify_start",
    "justify_center",
    "justify_end",
    "justify_between",
    "relative",
    "absolute",
    "overflow_hidden",
    "truncate",
    "w_full",
    "h_full",
    "size_full",
    "w_auto",
    "h_auto",
    "text_xs",
    "text_sm",
    "text_base",
    "text_lg",
    "text_xl",
    "text_2xl",
    "font_medium",
    "font_bold",
    "italic",
    "underline",
    "rounded_sm",
    "rounded_md",
    "rounded_lg",
    "rounded_xl",
    "border_1",
    "border_2",
    "border_t_1",
    "border_b_1",
    "cursor_pointer",
];

/// Prefixes that take a step on the spacing scale: `p_2`, `gap_x_4`, `w_16`.
#[cfg(test)] // Read only by `vocabulary`, which is test-only.
const SPACED: [&str; 24] = [
    "p", "px", "py", "pt", "pb", "pl", "pr", "m", "mx", "my", "mt", "mb", "ml", "mr", "gap",
    "gap_x", "gap_y", "w", "h", "size", "min_w", "min_h", "max_w", "max_h",
];

/// Tailwind's spacing scale: the suffix and the number of quarter-rems it
/// means. `0p5` is Tailwind's `0.5`, spelt so it can be an identifier.
const STEPS: [(&str, f32); 24] = [
    ("0", 0.0),
    ("0p5", 0.5),
    ("1", 1.0),
    ("1p5", 1.5),
    ("2", 2.0),
    ("2p5", 2.5),
    ("3", 3.0),
    ("3p5", 3.5),
    ("4", 4.0),
    ("5", 5.0),
    ("6", 6.0),
    ("8", 8.0),
    ("10", 10.0),
    ("12", 12.0),
    ("16", 16.0),
    ("20", 20.0),
    ("24", 24.0),
    ("32", 32.0),
    ("40", 40.0),
    ("48", 48.0),
    ("64", 64.0),
    ("72", 72.0),
    ("80", 80.0),
    ("96", 96.0),
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Utility(Operation);

#[derive(Clone, Copy, Debug, PartialEq)]
enum Operation {
    Flex,
    FlexCol,
    FlexRow,
    Flex1,
    FlexNone,
    FlexWrap,
    ItemsStart,
    ItemsCenter,
    ItemsEnd,
    JustifyStart,
    JustifyCenter,
    JustifyEnd,
    JustifyBetween,
    Relative,
    Absolute,
    OverflowHidden,
    Truncate,
    WFull,
    HFull,
    SizeFull,
    WAuto,
    HAuto,
    TextXs,
    TextSm,
    TextBase,
    TextLg,
    TextXl,
    Text2xl,
    FontMedium,
    FontBold,
    Italic,
    Underline,
    RoundedSm,
    RoundedMd,
    RoundedLg,
    RoundedXl,
    Border1,
    Border2,
    BorderT1,
    BorderB1,
    CursorPointer,
    P(f32),
    Px(f32),
    Py(f32),
    Pt(f32),
    Pb(f32),
    Pl(f32),
    Pr(f32),
    M(f32),
    Mx(f32),
    My(f32),
    Mt(f32),
    Mb(f32),
    Ml(f32),
    Mr(f32),
    Gap(f32),
    GapX(f32),
    GapY(f32),
    W(f32),
    H(f32),
    Size(f32),
    MinW(f32),
    MinH(f32),
    MaxW(f32),
    MaxH(f32),
}

pub fn parse(token: &str) -> Option<Utility> {
    Some(Utility(match token {
        "flex" => Operation::Flex,
        "flex_col" => Operation::FlexCol,
        "flex_row" => Operation::FlexRow,
        "flex_1" => Operation::Flex1,
        "flex_none" => Operation::FlexNone,
        "flex_wrap" => Operation::FlexWrap,
        "items_start" => Operation::ItemsStart,
        "items_center" => Operation::ItemsCenter,
        "items_end" => Operation::ItemsEnd,
        "justify_start" => Operation::JustifyStart,
        "justify_center" => Operation::JustifyCenter,
        "justify_end" => Operation::JustifyEnd,
        "justify_between" => Operation::JustifyBetween,
        "relative" => Operation::Relative,
        "absolute" => Operation::Absolute,
        "overflow_hidden" => Operation::OverflowHidden,
        "truncate" => Operation::Truncate,
        "w_full" => Operation::WFull,
        "h_full" => Operation::HFull,
        "size_full" => Operation::SizeFull,
        "w_auto" => Operation::WAuto,
        "h_auto" => Operation::HAuto,
        "text_xs" => Operation::TextXs,
        "text_sm" => Operation::TextSm,
        "text_base" => Operation::TextBase,
        "text_lg" => Operation::TextLg,
        "text_xl" => Operation::TextXl,
        "text_2xl" => Operation::Text2xl,
        "font_medium" => Operation::FontMedium,
        "font_bold" => Operation::FontBold,
        "italic" => Operation::Italic,
        "underline" => Operation::Underline,
        "rounded_sm" => Operation::RoundedSm,
        "rounded_md" => Operation::RoundedMd,
        "rounded_lg" => Operation::RoundedLg,
        "rounded_xl" => Operation::RoundedXl,
        "border_1" => Operation::Border1,
        "border_2" => Operation::Border2,
        "border_t_1" => Operation::BorderT1,
        "border_b_1" => Operation::BorderB1,
        "cursor_pointer" => Operation::CursorPointer,
        _ => {
            let (prefix, suffix) = token.rsplit_once('_')?;
            let amount = step(suffix)?;
            match prefix {
                "p" => Operation::P(amount),
                "px" => Operation::Px(amount),
                "py" => Operation::Py(amount),
                "pt" => Operation::Pt(amount),
                "pb" => Operation::Pb(amount),
                "pl" => Operation::Pl(amount),
                "pr" => Operation::Pr(amount),
                "m" => Operation::M(amount),
                "mx" => Operation::Mx(amount),
                "my" => Operation::My(amount),
                "mt" => Operation::Mt(amount),
                "mb" => Operation::Mb(amount),
                "ml" => Operation::Ml(amount),
                "mr" => Operation::Mr(amount),
                "gap" => Operation::Gap(amount),
                "gap_x" => Operation::GapX(amount),
                "gap_y" => Operation::GapY(amount),
                "w" => Operation::W(amount),
                "h" => Operation::H(amount),
                "size" => Operation::Size(amount),
                "min_w" => Operation::MinW(amount),
                "min_h" => Operation::MinH(amount),
                "max_w" => Operation::MaxW(amount),
                "max_h" => Operation::MaxH(amount),
                _ => return None,
            }
        }
    }))
}

pub fn apply_all<E: Styled>(mut element: E, tokens: &[Utility]) -> E {
    for token in tokens {
        element = token.apply(element);
    }
    element
}

impl Utility {
    fn apply<E: Styled>(self, element: E) -> E {
        match self.0 {
            Operation::Flex => element.flex(),
            Operation::FlexCol => element.flex_col(),
            Operation::FlexRow => element.flex_row(),
            Operation::Flex1 => element.flex_1(),
            Operation::FlexNone => element.flex_none(),
            Operation::FlexWrap => element.flex_wrap(),
            Operation::ItemsStart => element.items_start(),
            Operation::ItemsCenter => element.items_center(),
            Operation::ItemsEnd => element.items_end(),
            Operation::JustifyStart => element.justify_start(),
            Operation::JustifyCenter => element.justify_center(),
            Operation::JustifyEnd => element.justify_end(),
            Operation::JustifyBetween => element.justify_between(),
            Operation::Relative => element.relative(),
            Operation::Absolute => element.absolute(),
            Operation::OverflowHidden => element.overflow_hidden(),
            Operation::Truncate => element.truncate(),
            Operation::WFull => element.w_full(),
            Operation::HFull => element.h_full(),
            Operation::SizeFull => element.size_full(),
            Operation::WAuto => element.w_auto(),
            Operation::HAuto => element.h_auto(),
            Operation::TextXs => element.text_xs(),
            Operation::TextSm => element.text_sm(),
            Operation::TextBase => element.text_base(),
            Operation::TextLg => element.text_lg(),
            Operation::TextXl => element.text_xl(),
            Operation::Text2xl => element.text_2xl(),
            Operation::FontMedium => element.font_weight(FontWeight::MEDIUM),
            Operation::FontBold => element.font_weight(FontWeight::BOLD),
            Operation::Italic => element.italic(),
            Operation::Underline => element.underline(),
            Operation::RoundedSm => element.rounded_sm(),
            Operation::RoundedMd => element.rounded_md(),
            Operation::RoundedLg => element.rounded_lg(),
            Operation::RoundedXl => element.rounded_xl(),
            Operation::Border1 => element.border_1(),
            Operation::Border2 => element.border_2(),
            Operation::BorderT1 => element.border_t_1(),
            Operation::BorderB1 => element.border_b_1(),
            Operation::CursorPointer => element.cursor_pointer(),
            Operation::P(amount) => {
                let definite: DefiniteLength = rems(amount).into();
                element.p(definite)
            }
            Operation::Px(amount) => {
                let definite: DefiniteLength = rems(amount).into();
                element.px(definite)
            }
            Operation::Py(amount) => {
                let definite: DefiniteLength = rems(amount).into();
                element.py(definite)
            }
            Operation::Pt(amount) => {
                let definite: DefiniteLength = rems(amount).into();
                element.pt(definite)
            }
            Operation::Pb(amount) => {
                let definite: DefiniteLength = rems(amount).into();
                element.pb(definite)
            }
            Operation::Pl(amount) => {
                let definite: DefiniteLength = rems(amount).into();
                element.pl(definite)
            }
            Operation::Pr(amount) => {
                let definite: DefiniteLength = rems(amount).into();
                element.pr(definite)
            }
            Operation::M(amount) => {
                let definite: DefiniteLength = rems(amount).into();
                let length = Length::Definite(definite);
                element.m(length)
            }
            Operation::Mx(amount) => {
                let definite: DefiniteLength = rems(amount).into();
                let length = Length::Definite(definite);
                element.mx(length)
            }
            Operation::My(amount) => {
                let definite: DefiniteLength = rems(amount).into();
                let length = Length::Definite(definite);
                element.my(length)
            }
            Operation::Mt(amount) => {
                let definite: DefiniteLength = rems(amount).into();
                let length = Length::Definite(definite);
                element.mt(length)
            }
            Operation::Mb(amount) => {
                let definite: DefiniteLength = rems(amount).into();
                let length = Length::Definite(definite);
                element.mb(length)
            }
            Operation::Ml(amount) => {
                let definite: DefiniteLength = rems(amount).into();
                let length = Length::Definite(definite);
                element.ml(length)
            }
            Operation::Mr(amount) => {
                let definite: DefiniteLength = rems(amount).into();
                let length = Length::Definite(definite);
                element.mr(length)
            }
            Operation::Gap(amount) => {
                let definite: DefiniteLength = rems(amount).into();
                element.gap(definite)
            }
            Operation::GapX(amount) => {
                let definite: DefiniteLength = rems(amount).into();
                element.gap_x(definite)
            }
            Operation::GapY(amount) => {
                let definite: DefiniteLength = rems(amount).into();
                element.gap_y(definite)
            }
            Operation::W(amount) => {
                let definite: DefiniteLength = rems(amount).into();
                let length = Length::Definite(definite);
                element.w(length)
            }
            Operation::H(amount) => {
                let definite: DefiniteLength = rems(amount).into();
                let length = Length::Definite(definite);
                element.h(length)
            }
            Operation::Size(amount) => {
                let definite: DefiniteLength = rems(amount).into();
                let length = Length::Definite(definite);
                element.size(length)
            }
            Operation::MinW(amount) => {
                let definite: DefiniteLength = rems(amount).into();
                let length = Length::Definite(definite);
                element.min_w(length)
            }
            Operation::MinH(amount) => {
                let definite: DefiniteLength = rems(amount).into();
                let length = Length::Definite(definite);
                element.min_h(length)
            }
            Operation::MaxW(amount) => {
                let definite: DefiniteLength = rems(amount).into();
                let length = Length::Definite(definite);
                element.max_w(length)
            }
            Operation::MaxH(amount) => {
                let definite: DefiniteLength = rems(amount).into();
                let length = Length::Definite(definite);
                element.max_h(length)
            }
        }
    }
}

/// A spacing suffix as rems: Tailwind's scale is quarter-rems.
fn step(suffix: &str) -> Option<f32> {
    STEPS
        .iter()
        .find(|(candidate, _)| *candidate == suffix)
        .map(|(_, quarters)| quarters / 4.0)
}

/// Every token the table accepts, for the test that keeps the table honest.
#[cfg(test)]
fn vocabulary() -> Vec<String> {
    let mut tokens: Vec<String> = FIXED.iter().map(|token| (*token).to_owned()).collect();
    for prefix in SPACED {
        for (suffix, _) in STEPS {
            tokens.push(format!("{prefix}_{suffix}"));
        }
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_token_in_the_vocabulary_applies_and_nothing_else_does() {
        for token in vocabulary() {
            assert!(
                parse(&token).is_some(),
                "{token} is in the vocabulary but does not apply"
            );
            let _ = parse(&token).unwrap().apply(gpui::div());
        }
        for bogus in [
            "sparkle",
            "p_7",
            "gap_",
            "w_9999",
            "flex-col",
            "text_4xl",
            "rounded_full",
        ] {
            assert!(parse(bogus).is_none(), "{bogus} should not apply");
        }
    }

    #[test]
    fn a_spacing_token_is_a_prefix_and_a_step_on_the_tailwind_scale() {
        assert_eq!(step("2"), Some(0.5));
        assert_eq!(step("0p5"), Some(0.125));
        assert_eq!(step("16"), Some(4.0));
        assert_eq!(step("7"), None);
        assert_eq!(step("full"), None);
    }

    #[test]
    fn unknown_tokens_are_refused_before_applying_style() {
        assert!(parse("no_such_token").is_none());
        let element = gpui::div();
        let _ = parse("flex").unwrap().apply(element);
    }

    #[test]
    fn later_parsed_utilities_override_earlier_values() {
        for (tokens, expected) in [
            (["font_bold", "font_medium"], FontWeight::MEDIUM),
            (["font_medium", "font_bold"], FontWeight::BOLD),
        ] {
            let tokens = tokens.map(|token| parse(token).unwrap());
            let mut element = apply_all(gpui::div(), &tokens);
            assert_eq!(
                element.style().text.as_ref().unwrap().font_weight,
                Some(expected)
            );
        }
    }

    #[test]
    fn apply_all_applies_every_token_in_order() {
        let tokens: Vec<Utility> = ["flex", "flex_col", "gap_2", "p_3", "w_full"]
            .iter()
            .map(|token| parse(token).unwrap())
            .collect();
        // No panic and a Div back is the whole promise here; what each token
        // sets is GPUI's, not ours.
        let _element = apply_all(gpui::div(), &tokens);
    }
}
