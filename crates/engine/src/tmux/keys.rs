//! Key input model: literal text vs named tmux keys, and the `<Name>` string
//! convention used by adapter buttons and UI input lines.
//!
//! `"y<Enter>"` → send `y` literally (`send-keys -l`), then the `Enter` key by
//! name. A `<…>` token that is not a valid tmux key name is sent literally,
//! angle brackets included, so arbitrary text can't be broken by the parser.

/// One batch of input for `send-keys`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyInput {
    /// Literal text, sent with `send-keys -l --`.
    Text(String),
    /// Named tmux keys (`Enter`, `C-c`, `Up`…), sent with `send-keys --`.
    Named(Vec<String>),
}

/// Is `name` a tmux key name we allow through `send-keys` (non-literal mode)?
/// Accepts optional `C-`/`M-`/`S-` modifier prefixes on a base key.
pub fn is_valid_key_name(name: &str) -> bool {
    let mut base = name;
    loop {
        let Some(rest) = base
            .strip_prefix("C-")
            .or_else(|| base.strip_prefix("M-"))
            .or_else(|| base.strip_prefix("S-"))
        else {
            break;
        };
        base = rest;
    }
    if base.len() == 1 {
        // Single printable char (letter, digit, punctuation) is a valid key.
        return base.chars().next().is_some_and(|c| c.is_ascii_graphic());
    }
    matches!(
        base,
        "Enter"
            | "Escape"
            | "Space"
            | "Tab"
            | "BTab"
            | "BSpace"
            | "Home"
            | "End"
            | "PageUp"
            | "PageDown"
            | "PPage"
            | "NPage"
            | "Up"
            | "Down"
            | "Left"
            | "Right"
            | "DC"
            | "IC"
            | "F1"
            | "F2"
            | "F3"
            | "F4"
            | "F5"
            | "F6"
            | "F7"
            | "F8"
            | "F9"
            | "F10"
            | "F11"
            | "F12"
    )
}

/// Parse the `<Name>` convention into `KeyInput` batches, merging adjacent
/// named keys into one batch to minimize round-trips.
pub fn parse_key_string(s: &str) -> Vec<KeyInput> {
    let mut out: Vec<KeyInput> = Vec::new();
    let mut literal = String::new();

    let flush_literal = |literal: &mut String, out: &mut Vec<KeyInput>| {
        if !literal.is_empty() {
            out.push(KeyInput::Text(std::mem::take(literal)));
        }
    };

    let mut rest = s;
    while let Some(open) = rest.find('<') {
        let (before, from_open) = rest.split_at(open);
        literal.push_str(before);
        match from_open[1..].find('>') {
            Some(close_rel) => {
                let name = &from_open[1..1 + close_rel];
                if is_valid_key_name(name) {
                    flush_literal(&mut literal, &mut out);
                    match out.last_mut() {
                        Some(KeyInput::Named(keys)) => keys.push(name.to_string()),
                        _ => out.push(KeyInput::Named(vec![name.to_string()])),
                    }
                } else {
                    // Not a key name — angle brackets were literal text.
                    literal.push('<');
                    literal.push_str(name);
                    literal.push('>');
                }
                rest = &from_open[1 + close_rel + 1..];
            }
            None => {
                // Unclosed '<' — literal.
                literal.push_str(from_open);
                rest = "";
            }
        }
    }
    literal.push_str(rest);
    flush_literal(&mut literal, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_literal_and_named() {
        assert_eq!(
            parse_key_string("y<Enter>"),
            vec![
                KeyInput::Text("y".into()),
                KeyInput::Named(vec!["Enter".into()])
            ]
        );
    }

    #[test]
    fn merges_adjacent_named_keys() {
        assert_eq!(
            parse_key_string("<Up><Up><Enter>"),
            vec![KeyInput::Named(vec![
                "Up".into(),
                "Up".into(),
                "Enter".into()
            ])]
        );
    }

    #[test]
    fn plain_text_passes_through() {
        assert_eq!(
            parse_key_string("hello world"),
            vec![KeyInput::Text("hello world".into())]
        );
    }

    #[test]
    fn invalid_token_stays_literal() {
        assert_eq!(
            parse_key_string("a < b and <notakey> here"),
            vec![KeyInput::Text("a < b and <notakey> here".into())]
        );
    }

    #[test]
    fn unclosed_angle_stays_literal() {
        assert_eq!(
            parse_key_string("std::Vec<T"),
            vec![KeyInput::Text("std::Vec<T".into())]
        );
    }

    #[test]
    fn modifier_combos_are_valid() {
        assert!(is_valid_key_name("C-c"));
        assert!(is_valid_key_name("M-Enter"));
        assert!(is_valid_key_name("C-M-x"));
        assert!(!is_valid_key_name("C-"));
        assert!(!is_valid_key_name("Bogus"));
        assert!(!is_valid_key_name(""));
    }

    #[test]
    fn newline_in_literal_is_fine() {
        // Buttons like keys = "y\n" ship the newline literally (tmux turns it
        // into a carriage-return keypress).
        assert_eq!(parse_key_string("y\n"), vec![KeyInput::Text("y\n".into())]);
    }
}
