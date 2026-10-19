use super::document::Block;
use std::{ops::Range, sync::OnceLock};
use syntect::{
    easy::HighlightLines, highlighting::ThemeSet, parsing::SyntaxSet, util::LinesWithEndings,
};

#[derive(Clone, Debug, Default)]
pub(super) struct Highlights {
    pub dark: Vec<(Range<usize>, u32)>,
    pub light: Vec<(Range<usize>, u32)>,
}

pub(super) fn prepare(blocks: &mut [Block]) {
    static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
    static THEMES: OnceLock<ThemeSet> = OnceLock::new();
    let mut remaining = 256 * 1024;
    let mut remaining_spans = 20_000;
    for block in blocks.iter_mut().filter(|block| block.code) {
        if block.language.is_empty()
            || block.text.len() > remaining.min(64 * 1024)
            || block.text.lines().any(|line| line.len() > 4096)
        {
            continue;
        }
        remaining -= block.text.len();
        let syntaxes = SYNTAXES.get_or_init(SyntaxSet::load_defaults_newlines);
        let token = match block.language.as_str() {
            "typescript" | "ts" | "tsx" | "jsx" => "js",
            "shell" | "console" | "zsh" => "sh",
            other => other,
        };
        let Some(syntax) = syntaxes.find_syntax_by_token(token) else {
            continue;
        };
        let themes = THEMES.get_or_init(ThemeSet::load_defaults);
        let mut highlights = Highlights::default();
        for (name, output) in [
            ("base16-ocean.dark", &mut highlights.dark),
            ("InspiredGitHub", &mut highlights.light),
        ] {
            let Some(theme) = themes.themes.get(name) else {
                continue;
            };
            let mut highlighter = HighlightLines::new(syntax, theme);
            let mut offset = 0;
            for line in LinesWithEndings::from(&block.text) {
                let Ok(regions) = highlighter.highlight_line(line, syntaxes) else {
                    break;
                };
                for (style, text) in regions {
                    if remaining_spans == 0 {
                        break;
                    }
                    remaining_spans -= 1;
                    let foreground = style.foreground;
                    let color = (u32::from(foreground.r) << 16)
                        | (u32::from(foreground.g) << 8)
                        | u32::from(foreground.b);
                    output.push((offset..offset + text.len(), color));
                    offset += text.len();
                }
                if remaining_spans == 0 {
                    break;
                }
            }
        }
        block.highlights = highlights;
    }
}
