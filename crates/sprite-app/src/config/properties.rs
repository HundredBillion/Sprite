use super::*;
use proptest::{
    prelude::*,
    test_runner::{Config, RngSeed},
};

fn text() -> impl Strategy<Value = String> {
    proptest::collection::vec(prop_oneof![
        3 => any::<char>(),
        3 => proptest::sample::select(vec!['\0', '\u{1}', '\u{8}', '\t', '\n', '\r', '\u{1f}', '\u{7f}', '"', '\\', '.', ' ', 'é', '界', '🦀']),
    ], 0..20).prop_map(|chars| chars.into_iter().collect())
}
fn rgb() -> impl Strategy<Value = sprite_term::Rgb> {
    any::<[u8; 3]>().prop_map(|[r, g, b]| sprite_term::Rgb { r, g, b })
}
fn number() -> impl Strategy<Value = f32> {
    prop_oneof![
        any::<u32>().prop_map(f32::from_bits),
        -100.0f32..200.0,
        proptest::sample::select(vec![
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            -0.0,
            0.0,
            1.0,
            2.0,
            6.0,
            14.0,
            64.0,
            72.0
        ]),
    ]
}
fn underline() -> impl Strategy<Value = Option<sprite_term::UnderlineStyle>> {
    use sprite_term::UnderlineStyle::*;
    proptest::option::of(proptest::sample::select(vec![
        None, Single, Double, Curly, Dotted, Dashed,
    ]))
}
fn style() -> impl Strategy<Value = HighlightStyle> {
    (
        proptest::option::of(rgb()),
        proptest::option::of(rgb()),
        any::<Option<bool>>(),
        any::<Option<bool>>(),
        underline(),
    )
        .prop_map(
            |(color, background, bold, italic, underline)| HighlightStyle {
                color,
                background,
                bold,
                italic,
                underline,
            },
        )
}
fn settings() -> impl Strategy<Value = Settings> {
    (
        (text(), number(), number()),
        (
            proptest::option::of(rgb()),
            proptest::option::of(rgb()),
            proptest::option::of(rgb()),
            proptest::collection::vec((any::<u8>(), rgb()), 0..16),
            proptest::collection::vec((text(), rgb()), 0..8),
        ),
        proptest::collection::vec((text(), style()), 0..8),
        (proptest::option::of(0u8..4), any::<Option<bool>>()),
        number(),
        (
            text(),
            proptest::option::of(proptest::collection::vec(text(), 0..8)),
            text(),
        ),
        any::<usize>(),
        (any::<bool>(), any::<u64>(), any::<usize>()),
        any::<bool>(),
    )
        .prop_map(
            |(
                font,
                colors,
                highlights,
                cursor,
                padding,
                shell,
                scrollback,
                graphics,
                observation,
            )| {
                use sprite_term::CursorStyle;
                Settings {
                    font: Font {
                        family: NonBlank::new(font.0),
                        size: FontSize::new(font.1),
                        line_height: LineHeight::new(font.2),
                    },
                    colors: Colors {
                        background: colors.0,
                        foreground: colors.1,
                        cursor: colors.2,
                        palette: colors.3.into(),
                        tokens: colors.4.into(),
                    },
                    highlights: Highlights::from_groups(highlights),
                    cursor: Cursor {
                        style: cursor.0.map(|index| {
                            [
                                CursorStyle::Block,
                                CursorStyle::Bar,
                                CursorStyle::Underline,
                                CursorStyle::BlockHollow,
                            ][index as usize]
                        }),
                        blink: cursor.1,
                    },
                    grid: Grid {
                        padding: Padding::new(padding),
                    },
                    shell: Shell {
                        program: NonBlank::new(shell.0),
                        args: shell.1,
                        startup_directory: NonBlank::new(shell.2),
                    },
                    scrollback: Scrollback {
                        bytes: ScrollbackBytes::new(scrollback),
                    },
                    graphics: Graphics {
                        enabled: graphics.0,
                        storage_bytes: StorageBytes::new(graphics.1),
                        texture_bytes: TextureBytes::new(graphics.2),
                    },
                    pane_observation: PaneObservation {
                        enabled: observation,
                    },
                }
            },
        )
}
fn config() -> Config {
    Config {
        cases: 128,
        max_shrink_iters: 4096,
        rng_seed: std::env::var("PROPTEST_RNG_SEED")
            .ok()
            .and_then(|seed| seed.parse().ok())
            .map(RngSeed::Fixed)
            .unwrap_or(RngSeed::Fixed(0x5350_5249_5445_0048)),
        ..Config::default()
    }
}
proptest! {
    #![proptest_config(config())]
    #[test]
    fn whole_settings_round_trip(settings in settings()) {
        let encoded = settings.to_toml();
        let (decoded, complaints) = Settings::parse_candidate(&encoded).map_err(|error| TestCaseError::fail(format!("{error}\n{encoded}")))?;
        prop_assert!(complaints.0.is_empty(), "{:?}\n{}", complaints, encoded);
        prop_assert_eq!(decoded, settings);
    }

    #[test]
    fn drawable_metrics_survive_construction_and_font_actions(size in number(), height in number(), padding in number(), deltas in proptest::collection::vec(number(), 0..32)) {
        let mut size = FontSize::new(size);
        let height = LineHeight::new(height);
        let padding = Padding::new(padding);
        for delta in deltas.into_iter().chain([0.0]) {
            size = FontSize::new(size.get() + delta);
            prop_assert!(size.get().is_finite() && (6.0..=72.0).contains(&size.get()));
            prop_assert!(height.get().is_finite() && (1.0..=2.0).contains(&height.get()));
            prop_assert!(padding.get().is_finite() && (0.0..=64.0).contains(&padding.get()));
            let pixels = Font::cell_height(size.get(), height.get());
            prop_assert!(pixels.is_finite() && (6.0..=144.0).contains(&pixels));
        }
    }

    #[test]
    fn malformed_fields_preserve_siblings(value in proptest::sample::select(vec!["false", "[]", "{}", "\"oops\""]), size in 6u32..73, rgb in rgb()) {
        let input = format!("[font]\nfamily = []\nsize = {size}\nline_height = {value}\n[colors]\nbackground = {}\nforeground = 123\n[graphics]\nenabled = false\ntexture_bytes = []\n", quote(&hex(rgb)));
        let (settings, complaints) = Settings::parse_candidate(&input).unwrap();
        prop_assert_eq!(settings.font.size.get(), size as f32);
        prop_assert_eq!(settings.font.line_height, LineHeight::default());
        prop_assert_eq!(settings.colors.background, Some(rgb));
        prop_assert!(!settings.graphics.enabled);
        for key in ["font.family", "font.line_height", "colors.foreground", "graphics.texture_bytes"] {
            prop_assert!(complaints.0.iter().any(|message| message.contains(key)), "missing {key}: {:?}", complaints);
        }
    }

    #[test]
    fn malformed_sections_preserve_other_sections(section in proptest::sample::select(vec!["font", "colors", "highlights", "cursor", "shell", "scrollback", "graphics", "pane_observation"]), value in proptest::sample::select(vec!["false", "[]", "7", "\"oops\""]), padding in 0u32..65) {
        let (settings, complaints) = Settings::parse_candidate(&format!("{section} = {value}\n[grid]\npadding = {padding}\n")).unwrap();
        prop_assert_eq!(settings.grid.padding.get(), padding as f32);
        prop_assert!(complaints.0.iter().any(|message| message.contains(section)));
    }
}

#[test]
fn canonical_names_and_palette_indices_are_sorted_unique_and_last_wins() {
    let red = sprite_term::Rgb { r: 255, g: 0, b: 0 };
    let blue = sprite_term::Rgb { r: 0, g: 0, b: 255 };
    let mut settings = Settings::default();
    settings.colors.palette = vec![(255, red), (1, red), (1, blue)].into();
    settings.colors.tokens = vec![("z".into(), red), ("".into(), red), ("z".into(), blue)].into();
    settings.highlights = Highlights::from_groups(vec![
        (
            "x".into(),
            HighlightStyle {
                bold: Some(true),
                ..Default::default()
            },
        ),
        (
            "x".into(),
            HighlightStyle {
                italic: Some(false),
                ..Default::default()
            },
        ),
    ]);
    assert_eq!(
        settings.colors.palette.to_vec(),
        vec![(1, blue), (255, red)]
    );
    assert_eq!(settings.colors.tokens[1].1, blue);
    assert_eq!(settings.highlights.get("x").unwrap().bold, None);
    assert_eq!(
        Settings::parse_candidate(&settings.to_toml()).unwrap(),
        (settings, Complaints::default())
    );
    let parsed = Settings::parse_candidate("[colors.palette]\n'01' = '#ff0000'\n'1' = '#0000ff'\n")
        .unwrap()
        .0;
    assert_eq!(parsed.colors.palette.to_vec(), vec![(1, blue)]);
}

#[test]
fn escaping_and_whitespace_are_lossless_in_keys_and_preferences() {
    let hostile = " \0\u{1}\u{8}\u{c}\n\r\t\u{1f}\u{7f}é界🦀\\\". ";
    let mut settings = Settings::default();
    settings.font.family = NonBlank::new(hostile.into());
    settings.shell = Shell {
        program: NonBlank::new(hostile.into()),
        args: Some(vec![hostile.into(), "".into()]),
        startup_directory: NonBlank::new(hostile.into()),
    };
    settings.highlights =
        Highlights::from_groups(vec![(hostile.into(), HighlightStyle::default())]);
    settings.colors.tokens = vec![(hostile.into(), sprite_term::Rgb { r: 1, g: 2, b: 3 })].into();
    assert_eq!(
        Settings::parse_candidate(&settings.to_toml()).unwrap(),
        (settings, Complaints::default())
    );
}

#[test]
fn unknown_keys_and_bad_highlight_siblings_are_reported_independently() {
    let (settings, complaints) = Settings::parse_candidate("unknown = 5\n[font]\nsize = 20\nunknown = 3\n[highlights]\nGood = { bold = true, color = false, unknown = 1 }\nBad = 42\n").unwrap();
    assert_eq!(settings.font.size, 20.0);
    assert_eq!(settings.highlights.get("Good").unwrap().bold, Some(true));
    assert_eq!(complaints.0.len(), 5);
    for key in [
        "unknown",
        "font.unknown",
        "highlights.Good.color",
        "highlights.Good.unknown",
        "highlights.Bad",
    ] {
        assert!(
            complaints.0.iter().any(|message| message.contains(key)),
            "{key}: {complaints:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn non_utf8_cli_commands_reach_session_config_unchanged() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let program = OsString::from_vec(b"/tmp/program-\xff".to_vec());
    let argument = OsString::from_vec(b"arg-\xfe".to_vec());
    let invocation =
        crate::cli::parse_arguments([OsString::from("-e"), program.clone(), argument.clone()])
            .unwrap();
    let crate::cli::Invocation::Window(args) = invocation else {
        panic!("window command");
    };
    let command = args.command.unwrap();
    let (executable, arguments) = command.split_first().unwrap();
    let session = sprite_term::SessionConfig::command(executable, arguments.to_vec());
    assert_eq!(session.program.as_os_str(), program);
    assert_eq!(session.args, vec![argument]);

    let preference = Shell {
        program: NonBlank::new(" /tmp/é ".into()),
        args: Some(vec!["界\\\"".into()]),
        startup_directory: NonBlank::new(" /tmp/界 ".into()),
    }
    .session_preference();
    assert_eq!(preference.program.unwrap().as_os_str(), " /tmp/é ");
    assert_eq!(preference.args.unwrap(), vec![OsString::from("界\\\"")]);
    assert_eq!(
        preference.startup_directory.unwrap().as_os_str(),
        " /tmp/界 "
    );
}
