//! Modelica syntax highlighting for the code editor.
//!
//! A small standalone scanner (independent from the strict parser, so
//! it never fails on incomplete code) classifies the source into
//! comments, strings, numbers, keywords, types and built-in functions,
//! and produces an `egui::text::LayoutJob` with JetBrains-inspired
//! colors.

use crate::settings::Theme;
use bevy_egui::egui::text::{LayoutJob, TextFormat};
use bevy_egui::egui::{Color32, FontId};

/// Colors of the token classes for one theme.
struct CodeColors {
    text: Color32,
    keyword: Color32,
    type_name: Color32,
    builtin: Color32,
    number: Color32,
    string: Color32,
    comment: Color32,
}

/// IntelliJ-like editor colors (dark: Darcula New UI, light: IntelliJ Light).
fn colors(theme: Theme) -> CodeColors {
    match theme {
        Theme::Dark => CodeColors {
            text: Color32::from_rgb(188, 190, 196),      // #BCBEC4
            keyword: Color32::from_rgb(207, 142, 109),   // #CF8E6D
            type_name: Color32::from_rgb(232, 191, 106), // #E8BF6A
            builtin: Color32::from_rgb(86, 168, 245),    // #56A8F5
            number: Color32::from_rgb(42, 172, 184),     // #2AACB8
            string: Color32::from_rgb(106, 171, 115),    // #6AAB73
            comment: Color32::from_rgb(122, 126, 133),   // #7A7E85
        },
        Theme::Light => CodeColors {
            text: Color32::from_rgb(8, 8, 8),
            keyword: Color32::from_rgb(0, 51, 179), // #0033B3
            type_name: Color32::from_rgb(0, 98, 122), // #00627A
            builtin: Color32::from_rgb(125, 94, 193), // #7D5EC1
            number: Color32::from_rgb(23, 80, 235), // #1750EB
            string: Color32::from_rgb(6, 125, 23),  // #067D17
            comment: Color32::from_rgb(140, 140, 140), // #8C8C8C
        },
    }
}

const KEYWORDS: &[&str] = &[
    "model",
    "end",
    "equation",
    "parameter",
    "constant",
    "annotation",
    "if",
    "then",
    "elseif",
    "else",
    "and",
    "or",
    "not",
    "true",
    "false",
    "when",
    "for",
    "loop",
    "while",
    "function",
    "record",
    "block",
    "connector",
    "connect",
    "package",
    "extends",
    "import",
    "input",
    "output",
    "flow",
    "stream",
    "der",
    "initial",
    "within",
    "algorithm",
    "in",
    "protected",
    "type",
    "partial",
    "constant",
];

const TYPES: &[&str] = &["Real", "Integer", "Boolean", "String"];

const BUILTINS: &[&str] = &[
    "sin",
    "cos",
    "tan",
    "asin",
    "acos",
    "atan",
    "atan2",
    "sinh",
    "cosh",
    "tanh",
    "exp",
    "log",
    "log10",
    "sqrt",
    "abs",
    "sign",
    "min",
    "max",
    "time",
    "terminate",
];

/// Build a colored layout job for the given Modelica source.
pub fn highlight(source: &str, theme: Theme, font: FontId) -> LayoutJob {
    let c = colors(theme);
    let mut job = LayoutJob::default();
    let format = |color: Color32| TextFormat {
        font_id: font.clone(),
        color,
        ..Default::default()
    };
    let append = |job: &mut LayoutJob, text: &str, color: Color32| {
        if !text.is_empty() {
            job.append(text, 0.0, format(color));
        }
    };

    let bytes: Vec<char> = source.chars().collect();
    let mut i = 0;
    let mut plain_start = i;

    // Flush the pending unclassified run before a colored token.
    macro_rules! flush_plain {
        () => {
            if plain_start < i {
                let run: String = bytes[plain_start..i].iter().collect();
                append(&mut job, &run, c.text);
            }
        };
    }

    while i < bytes.len() {
        let ch = bytes[i];
        // Line comment.
        if ch == '/' && bytes.get(i + 1) == Some(&'/') {
            flush_plain!();
            let start = i;
            while i < bytes.len() && bytes[i] != '\n' {
                i += 1;
            }
            let run: String = bytes[start..i].iter().collect();
            append(&mut job, &run, c.comment);
            plain_start = i;
            continue;
        }
        // Block comment (tolerates a missing terminator).
        if ch == '/' && bytes.get(i + 1) == Some(&'*') {
            flush_plain!();
            let start = i;
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == '*' && bytes[i + 1] == '/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            let run: String = bytes[start..i].iter().collect();
            append(&mut job, &run, c.comment);
            plain_start = i;
            continue;
        }
        // String literal (tolerates a missing closing quote).
        if ch == '"' {
            flush_plain!();
            let start = i;
            i += 1;
            while i < bytes.len() && bytes[i] != '"' {
                if bytes[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            i = (i + 1).min(bytes.len());
            let run: String = bytes[start..i].iter().collect();
            append(&mut job, &run, c.string);
            plain_start = i;
            continue;
        }
        // Number.
        if ch.is_ascii_digit() {
            flush_plain!();
            let start = i;
            while i < bytes.len()
                && (bytes[i].is_ascii_digit()
                    || bytes[i] == '.'
                    || bytes[i] == 'e'
                    || bytes[i] == 'E'
                    || ((bytes[i] == '+' || bytes[i] == '-')
                        && matches!(bytes.get(i.wrapping_sub(1)), Some('e') | Some('E'))))
            {
                i += 1;
            }
            let run: String = bytes[start..i].iter().collect();
            append(&mut job, &run, c.number);
            plain_start = i;
            continue;
        }
        // Word: keyword, type, builtin or identifier.
        if ch.is_alphabetic() || ch == '_' {
            flush_plain!();
            let start = i;
            while i < bytes.len() && (bytes[i].is_alphanumeric() || bytes[i] == '_') {
                i += 1;
            }
            let word: String = bytes[start..i].iter().collect();
            let color = if KEYWORDS.contains(&word.as_str()) {
                c.keyword
            } else if TYPES.contains(&word.as_str()) {
                c.type_name
            } else if BUILTINS.contains(&word.as_str()) {
                c.builtin
            } else {
                c.text
            };
            append(&mut job, &word, color);
            plain_start = i;
            continue;
        }
        i += 1;
    }
    flush_plain!();
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(source: &str) -> LayoutJob {
        highlight(source, Theme::Dark, FontId::monospace(13.0))
    }

    #[test]
    fn reconstructs_source_exactly() {
        let source = "model M \"desc\" // c\n  Real x(start = 1e-3);\nequation\n  der(x) = -x; /* b */\nend M;\n";
        let j = job(source);
        let rebuilt: String = j
            .sections
            .iter()
            .map(|s| &j.text[s.byte_range.clone()])
            .collect();
        assert_eq!(rebuilt, source);
    }

    #[test]
    fn classifies_token_kinds() {
        let c = colors(Theme::Dark);
        let j = job("model Real sin 42 \"s\" // c");
        let color_of = |needle: &str| {
            j.sections
                .iter()
                .find(|s| j.text[s.byte_range.clone()].contains(needle))
                .map(|s| s.format.color)
                .unwrap()
        };
        assert_eq!(color_of("model"), c.keyword);
        assert_eq!(color_of("Real"), c.type_name);
        assert_eq!(color_of("sin"), c.builtin);
        assert_eq!(color_of("42"), c.number);
        assert_eq!(color_of("\"s\""), c.string);
        assert_eq!(color_of("// c"), c.comment);
    }

    #[test]
    fn tolerates_unterminated_constructs() {
        // Must not panic or lose text.
        for source in ["\"open string", "/* open comment", "1e", "x\\"] {
            let j = job(source);
            let rebuilt: String = j
                .sections
                .iter()
                .map(|s| &j.text[s.byte_range.clone()])
                .collect();
            assert_eq!(rebuilt, source);
        }
    }
}
