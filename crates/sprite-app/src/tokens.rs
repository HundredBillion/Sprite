//! Semantic colour tokens: the named roles a Surface or the terminal grid
//! refers to instead of a literal colour.
//!
//! A theme changes a colour by name; a program adds a role by name. The two
//! never need to know about each other, which is why a plugin can ship a
//! colour the theme has never heard of and still be restyled later.

use std::collections::BTreeMap;

use sprite_term::Rgb;

use crate::config::Colors;

/// The terminal's default background when neither a theme nor a program has
/// set one; also the fallback fill for an unknown token.
pub const DEFAULT_BACKGROUND: u32 = 0x101014;
/// The terminal's default text colour; also the fallback for an unknown token
/// used as text.
pub const DEFAULT_FOREGROUND: u32 = 0xd8d8e0;

/// Every token Sprite knows without being told: name, default, description.
///
/// The sixteen ANSI defaults are xterm's, which is what most programs were
/// written against; a theme that wants libghostty's exact shades sets them.
pub const BUILT_IN: [(&str, u32, &str); 19] = [
    (
        "terminal.background",
        DEFAULT_BACKGROUND,
        "The terminal's default background",
    ),
    (
        "terminal.foreground",
        DEFAULT_FOREGROUND,
        "The terminal's default text colour",
    ),
    (
        "terminal.cursor",
        DEFAULT_FOREGROUND,
        "The cursor, when a program has not coloured it",
    ),
    ("ansi.0", 0x000000, "ANSI colour 0, black"),
    ("ansi.1", 0xcd0000, "ANSI colour 1, red"),
    ("ansi.2", 0x00cd00, "ANSI colour 2, green"),
    ("ansi.3", 0xcdcd00, "ANSI colour 3, yellow"),
    ("ansi.4", 0x0000ee, "ANSI colour 4, blue"),
    ("ansi.5", 0xcd00cd, "ANSI colour 5, magenta"),
    ("ansi.6", 0x00cdcd, "ANSI colour 6, cyan"),
    ("ansi.7", 0xe5e5e5, "ANSI colour 7, white"),
    ("ansi.8", 0x7f7f7f, "ANSI colour 8, bright black"),
    ("ansi.9", 0xff0000, "ANSI colour 9, bright red"),
    ("ansi.10", 0x00ff00, "ANSI colour 10, bright green"),
    ("ansi.11", 0xffff00, "ANSI colour 11, bright yellow"),
    ("ansi.12", 0x5c5cff, "ANSI colour 12, bright blue"),
    ("ansi.13", 0xff00ff, "ANSI colour 13, bright magenta"),
    ("ansi.14", 0x00ffff, "ANSI colour 14, bright cyan"),
    ("ansi.15", 0xffffff, "ANSI colour 15, bright white"),
];

/// What a token is being used for, which decides its fallback when unknown:
/// a misspelt text colour dims one label rather than blanking it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    Text,
    Fill,
}

impl Role {
    pub fn fallback(self) -> &'static str {
        match self {
            Role::Text => "terminal.foreground",
            Role::Fill => "terminal.background",
        }
    }
}

/// A token a program registered: its default and what it is for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Registered {
    pub default: Rgb,
    pub description: String,
}

/// What a registration did.
// No caller registers a token yet; a program does that once it can open a
// Surface.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Registration {
    New,
    /// The name existed with this default already; a program registering on
    /// every start pays nothing for it.
    Same,
}

/// A name registered again with a different default. The first stands, so no
/// colour depends on which program started first.
// No caller registers a token yet; a program does that once it can open a
// Surface.
#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenConflict {
    pub name: String,
    pub standing: Rgb,
}

/// A resolved colour, and whether the name was actually known.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Resolved {
    pub color: Rgb,
    pub known: bool,
}

/// The one table Sprite resolves colour names through at draw time.
///
/// Precedence, highest first: the theme's value for the name, then the
/// program's registered default, then the built-in default. Session-scoped:
/// registrations vanish when Sprite exits, theme overrides persist in the
/// configuration file and apply the moment a matching token exists.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TokenRegistry {
    registered: BTreeMap<String, Registered>,
    theme: BTreeMap<String, Rgb>,
}

impl gpui::Global for TokenRegistry {}

impl TokenRegistry {
    pub fn new(colors: &Colors) -> Self {
        let mut registry = Self::default();
        registry.apply_theme(colors);
        registry
    }

    /// Replaces the theme layer from the configuration's colours.
    ///
    /// The existing `[colors]` keys are the overrides for the built-in tokens,
    /// so a file written before tokens existed keeps meaning what it meant.
    pub fn apply_theme(&mut self, colors: &Colors) {
        self.theme.clear();
        for (name, color) in [
            ("terminal.background", colors.background),
            ("terminal.foreground", colors.foreground),
            ("terminal.cursor", colors.cursor),
        ] {
            if let Some(color) = color {
                self.theme.insert(name.to_owned(), color);
            }
        }
        for (index, color) in &colors.palette {
            if *index < 16 {
                self.theme.insert(format!("ansi.{index}"), *color);
            }
        }
        for (name, color) in &colors.tokens {
            self.theme.insert(name.clone(), *color);
        }
    }

    // No caller registers a token yet; a program does that once it can open
    // a Surface.
    #[allow(dead_code)]
    pub fn register(
        &mut self,
        name: &str,
        default: Rgb,
        description: &str,
    ) -> Result<Registration, TokenConflict> {
        let standing = built_in_default(name).or_else(|| {
            self.registered
                .get(name)
                .map(|registered| registered.default)
        });
        match standing {
            Some(standing) if standing == default => Ok(Registration::Same),
            Some(standing) => Err(TokenConflict {
                name: name.to_owned(),
                standing,
            }),
            None => {
                self.registered.insert(
                    name.to_owned(),
                    Registered {
                        default,
                        description: description.to_owned(),
                    },
                );
                Ok(Registration::New)
            }
        }
    }

    pub fn is_known(&self, name: &str) -> bool {
        self.lookup(name).is_some()
    }

    pub fn resolve(&self, name: &str, role: Role) -> Resolved {
        match self.lookup(name) {
            Some(color) => Resolved { color, known: true },
            None => Resolved {
                color: self
                    .lookup(role.fallback())
                    .expect("the fallback tokens are built in"),
                known: false,
            },
        }
    }

    fn lookup(&self, name: &str) -> Option<Rgb> {
        self.theme
            .get(name)
            .copied()
            .or_else(|| {
                self.registered
                    .get(name)
                    .map(|registered| registered.default)
            })
            .or_else(|| built_in_default(name))
    }
}

/// The built-in default for a name, if it is one of Sprite's own tokens.
pub fn built_in_default(name: &str) -> Option<Rgb> {
    BUILT_IN
        .iter()
        .find(|(candidate, _, _)| *candidate == name)
        .map(|(_, packed, _)| unpack(*packed))
}

/// `0xrrggbb` as a colour.
pub fn unpack(packed: u32) -> Rgb {
    Rgb {
        r: (packed >> 16) as u8,
        g: (packed >> 8) as u8,
        b: packed as u8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Colors;

    fn rgb(packed: u32) -> Rgb {
        unpack(packed)
    }

    #[test]
    fn a_built_in_token_resolves_to_its_default_when_nothing_overrides_it() {
        let registry = TokenRegistry::new(&Colors::default());
        let resolved = registry.resolve("terminal.background", Role::Fill);
        assert_eq!(
            resolved,
            Resolved {
                color: rgb(DEFAULT_BACKGROUND),
                known: true
            }
        );
        assert_eq!(registry.resolve("ansi.4", Role::Text).color, rgb(0x0000ee));
    }

    #[test]
    fn the_theme_overrides_a_built_in_token_by_its_existing_colors_key() {
        let colors = Colors {
            background: Some(rgb(0x123456)),
            palette: vec![(4, rgb(0xabcdef))],
            ..Colors::default()
        };
        let registry = TokenRegistry::new(&colors);
        assert_eq!(
            registry.resolve("terminal.background", Role::Fill).color,
            rgb(0x123456)
        );
        assert_eq!(registry.resolve("ansi.4", Role::Text).color, rgb(0xabcdef));
        // Palette slots above 15 are the terminal's; they are not tokens.
        assert!(!registry.is_known("ansi.16"));
    }

    #[test]
    fn a_program_registered_token_is_known_and_the_theme_can_override_it() {
        let mut registry = TokenRegistry::new(&Colors::default());
        assert_eq!(
            registry.register("scm.added", rgb(0x00ff00), "Added lines"),
            Ok(Registration::New)
        );
        assert_eq!(
            registry.resolve("scm.added", Role::Text).color,
            rgb(0x00ff00)
        );

        let colors = Colors {
            tokens: vec![("scm.added".to_owned(), rgb(0x40a02b))],
            ..Colors::default()
        };
        registry.apply_theme(&colors);
        assert_eq!(
            registry.resolve("scm.added", Role::Text).color,
            rgb(0x40a02b)
        );
    }

    #[test]
    fn registering_the_same_default_again_is_a_no_op_and_a_different_one_is_refused() {
        let mut registry = TokenRegistry::new(&Colors::default());
        registry
            .register("scm.added", rgb(0x00ff00), "Added lines")
            .expect("first");
        assert_eq!(
            registry.register("scm.added", rgb(0x00ff00), "A different description"),
            Ok(Registration::Same)
        );
        assert_eq!(
            registry.register("scm.added", rgb(0xff0000), "Added lines"),
            Err(TokenConflict {
                name: "scm.added".to_owned(),
                standing: rgb(0x00ff00)
            })
        );
        // The first registration stands.
        assert_eq!(
            registry.resolve("scm.added", Role::Text).color,
            rgb(0x00ff00)
        );
    }

    #[test]
    fn a_built_in_name_cannot_be_re_registered_with_another_default() {
        let mut registry = TokenRegistry::new(&Colors::default());
        assert_eq!(
            registry.register("terminal.foreground", rgb(DEFAULT_FOREGROUND), ""),
            Ok(Registration::Same)
        );
        assert!(
            registry
                .register("terminal.foreground", rgb(0x000000), "")
                .is_err()
        );
    }

    #[test]
    fn an_unknown_token_falls_back_by_role_and_says_so() {
        let registry = TokenRegistry::new(&Colors::default());
        let text = registry.resolve("svgtree.iconn", Role::Text);
        assert_eq!(
            text,
            Resolved {
                color: rgb(DEFAULT_FOREGROUND),
                known: false
            }
        );
        let fill = registry.resolve("svgtree.iconn", Role::Fill);
        assert_eq!(
            fill,
            Resolved {
                color: rgb(DEFAULT_BACKGROUND),
                known: false
            }
        );
    }

    #[test]
    fn a_theme_only_token_counts_as_known() {
        let colors = Colors {
            tokens: vec![("demo.label".to_owned(), rgb(0xc0caf5))],
            ..Colors::default()
        };
        let registry = TokenRegistry::new(&colors);
        assert!(registry.is_known("demo.label"));
        assert_eq!(
            registry.resolve("demo.label", Role::Text).color,
            rgb(0xc0caf5)
        );
    }
}
