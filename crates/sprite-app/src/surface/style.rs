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

/// Whether a token is in the vocabulary. Answered by applying it to a
/// throwaway element, so there is exactly one table to keep true.
pub fn is_known(token: &str) -> bool {
    apply(gpui::div(), token).is_ok()
}

/// Applies every token; the tokens were validated when the description was
/// parsed, so an unknown one here is left alone rather than failing a paint.
pub fn apply_all<E: Styled>(mut element: E, tokens: &[String]) -> E {
    for token in tokens {
        element = match apply(element, token) {
            Ok(styled) | Err(styled) => styled,
        };
    }
    element
}

/// Applies one token, or hands the element back untouched if the token is not
/// in the vocabulary.
pub fn apply<E: Styled>(element: E, token: &str) -> Result<E, E> {
    Ok(match token {
        "flex" => element.flex(),
        "flex_col" => element.flex_col(),
        "flex_row" => element.flex_row(),
        "flex_1" => element.flex_1(),
        "flex_none" => element.flex_none(),
        "flex_wrap" => element.flex_wrap(),
        "items_start" => element.items_start(),
        "items_center" => element.items_center(),
        "items_end" => element.items_end(),
        "justify_start" => element.justify_start(),
        "justify_center" => element.justify_center(),
        "justify_end" => element.justify_end(),
        "justify_between" => element.justify_between(),
        "relative" => element.relative(),
        "absolute" => element.absolute(),
        "overflow_hidden" => element.overflow_hidden(),
        "truncate" => element.truncate(),
        "w_full" => element.w_full(),
        "h_full" => element.h_full(),
        "size_full" => element.size_full(),
        "w_auto" => element.w_auto(),
        "h_auto" => element.h_auto(),
        "text_xs" => element.text_xs(),
        "text_sm" => element.text_sm(),
        "text_base" => element.text_base(),
        "text_lg" => element.text_lg(),
        "text_xl" => element.text_xl(),
        "text_2xl" => element.text_2xl(),
        "font_medium" => element.font_weight(FontWeight::MEDIUM),
        "font_bold" => element.font_weight(FontWeight::BOLD),
        "italic" => element.italic(),
        "underline" => element.underline(),
        "rounded_sm" => element.rounded_sm(),
        "rounded_md" => element.rounded_md(),
        "rounded_lg" => element.rounded_lg(),
        "rounded_xl" => element.rounded_xl(),
        "border_1" => element.border_1(),
        "border_2" => element.border_2(),
        "border_t_1" => element.border_t_1(),
        "border_b_1" => element.border_b_1(),
        "cursor_pointer" => element.cursor_pointer(),
        spaced => return apply_spaced(element, spaced),
    })
}

fn apply_spaced<E: Styled>(element: E, token: &str) -> Result<E, E> {
    let Some((prefix, suffix)) = token.rsplit_once('_') else {
        return Err(element);
    };
    let Some(quarter_rems) = step(suffix) else {
        return Err(element);
    };
    let definite: DefiniteLength = rems(quarter_rems).into();
    let length = Length::Definite(definite);
    Ok(match prefix {
        "p" => element.p(definite),
        "px" => element.px(definite),
        "py" => element.py(definite),
        "pt" => element.pt(definite),
        "pb" => element.pb(definite),
        "pl" => element.pl(definite),
        "pr" => element.pr(definite),
        "m" => element.m(length),
        "mx" => element.mx(length),
        "my" => element.my(length),
        "mt" => element.mt(length),
        "mb" => element.mb(length),
        "ml" => element.ml(length),
        "mr" => element.mr(length),
        "gap" => element.gap(definite),
        "gap_x" => element.gap_x(definite),
        "gap_y" => element.gap_y(definite),
        "w" => element.w(length),
        "h" => element.h(length),
        "size" => element.size(length),
        "min_w" => element.min_w(length),
        "min_h" => element.min_h(length),
        "max_w" => element.max_w(length),
        "max_h" => element.max_h(length),
        _ => return Err(element),
    })
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
                is_known(&token),
                "{token} is in the vocabulary but does not apply"
            );
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
            assert!(!is_known(bogus), "{bogus} should not apply");
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
    fn apply_hands_the_element_back_when_a_token_is_unknown() {
        let element = gpui::div();
        assert!(apply(element, "no_such_token").is_err());
        let element = gpui::div();
        assert!(apply(element, "flex").is_ok());
    }

    #[test]
    fn apply_all_applies_every_token_in_order() {
        let tokens: Vec<String> = ["flex", "flex_col", "gap_2", "p_3", "w_full"]
            .iter()
            .map(|token| (*token).to_owned())
            .collect();
        // No panic and a Div back is the whole promise here; what each token
        // sets is GPUI's, not ours.
        let _element = apply_all(gpui::div(), &tokens);
    }
}
