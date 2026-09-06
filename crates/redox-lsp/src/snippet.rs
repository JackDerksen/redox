use crate::completion::{CompletionCandidate, InsertTextFormat};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnippetExpansion {
    pub text: String,
    pub placeholders: Vec<SnippetPlaceholder>,
    pub cursor_offset: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnippetPlaceholder {
    pub tabstop: usize,
    pub start: usize,
    pub end: usize,
}

/// Expands an LSP snippet and derives missing call placeholders from completion
/// signature metadata when possible.
#[must_use]
pub fn completion_snippet_expansion(
    item: &CompletionCandidate,
    insert: &str,
) -> Option<SnippetExpansion> {
    let mut expansion = match item.insert_text_format {
        InsertTextFormat::PlainText => None,
        InsertTextFormat::Snippet => Some(expand(insert)),
    };
    let Some(parameters) = completion_parameter_placeholders(item) else {
        return expansion;
    };
    if parameters.is_empty() {
        return expansion;
    }
    let should_synthesize =
        expansion.as_ref().is_none_or(|current| {
            current.placeholders.is_empty() || snippet_placeholders_are_empty(current)
        }) && (matches!(item.kind.as_deref(), Some("function") | Some("method"))
            || insert_looks_like_call_target(insert));
    let should_replace_existing = expansion.as_ref().is_some_and(|current| {
        current.cursor_offset.is_some() || snippet_placeholders_are_empty(current)
    });
    if !should_synthesize {
        return expansion;
    }
    let call_text = expansion
        .as_ref()
        .map(|current| current.text.as_str())
        .unwrap_or(insert);
    let synthesized = synthesize_call_snippet(call_text, &parameters)?;
    if expansion.is_none() || should_replace_existing && !synthesized.text.is_empty() {
        expansion = Some(synthesized);
    }
    expansion
}

/// Expands LSP tabstops, placeholder defaults, and escapes into plain text plus
/// character-indexed placeholder ranges.
#[must_use]
pub fn expand(snippet: &str) -> SnippetExpansion {
    let mut output = String::new();
    let mut placeholders = Vec::new();
    let mut final_cursor = None;
    let mut characters = snippet.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\\' {
            if let Some(next) = characters.next() {
                output.push(next);
            }
            continue;
        }
        if character != '$' {
            output.push(character);
            continue;
        }

        match characters.peek().copied() {
            Some('{') => {
                let _ = characters.next();
                let mut body = String::new();
                let mut depth = 1usize;
                while let Some(next) = characters.next() {
                    match next {
                        '\\' => {
                            body.push(next);
                            if let Some(escaped) = characters.next() {
                                body.push(escaped);
                            }
                        }
                        '{' => {
                            depth = depth.saturating_add(1);
                            body.push(next);
                        }
                        '}' => {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                break;
                            }
                            body.push(next);
                        }
                        _ => body.push(next),
                    }
                }
                let expansion = expand_placeholder(&body);
                let start = output.chars().count();
                let end = start.saturating_add(expansion.text.chars().count());
                for mut placeholder in expansion.placeholders {
                    placeholder.start = placeholder.start.saturating_add(start);
                    placeholder.end = placeholder.end.saturating_add(start);
                    placeholders.push(placeholder);
                }
                if final_cursor.is_none() {
                    final_cursor = expansion
                        .cursor_offset
                        .map(|offset| start.saturating_add(offset));
                }
                output.push_str(&expansion.text);
                if let Some(tabstop) = placeholder_tabstop(&body)
                    && tabstop != 0
                {
                    placeholders.push(SnippetPlaceholder {
                        tabstop,
                        start,
                        end,
                    });
                }
            }
            Some(next) if next.is_ascii_digit() => {
                let mut digits = String::new();
                while let Some(digit) = characters
                    .peek()
                    .copied()
                    .filter(|character| character.is_ascii_digit())
                {
                    digits.push(digit);
                    let _ = characters.next();
                }
                if let Ok(tabstop) = digits.parse::<usize>() {
                    let at = output.chars().count();
                    if tabstop == 0 {
                        final_cursor.get_or_insert(at);
                    } else {
                        placeholders.push(SnippetPlaceholder {
                            tabstop,
                            start: at,
                            end: at,
                        });
                    }
                }
            }
            _ => output.push(character),
        }
    }
    placeholders.sort_by_key(|placeholder| (placeholder.tabstop, placeholder.start));
    placeholders
        .dedup_by_key(|placeholder| (placeholder.tabstop, placeholder.start, placeholder.end));
    SnippetExpansion {
        text: output,
        placeholders,
        cursor_offset: final_cursor,
    }
}

fn expand_placeholder(body: &str) -> SnippetExpansion {
    if let Some((tabstop, choices)) = body.split_once('|')
        && let Ok(tabstop) = tabstop.parse::<usize>()
        && let Some(choices) = choices.strip_suffix('|')
    {
        // ponytail: start with the first choice as an editable placeholder;
        // add a choice picker when the editor has a snippet-choice UI.
        let mut text = String::new();
        let mut characters = choices.chars();
        while let Some(character) = characters.next() {
            match character {
                ',' => break,
                '\\' => {
                    if let Some(escaped) = characters.next() {
                        text.push(escaped);
                    }
                }
                _ => text.push(character),
            }
        }
        let end = text.chars().count();
        return SnippetExpansion {
            text,
            placeholders: if tabstop == 0 {
                Vec::new()
            } else {
                vec![SnippetPlaceholder {
                    tabstop,
                    start: 0,
                    end,
                }]
            },
            cursor_offset: (tabstop == 0).then_some(end),
        };
    }
    let Some((tabstop, default)) = body.split_once(':') else {
        let tabstop = body.parse::<usize>().ok();
        return SnippetExpansion {
            text: String::new(),
            placeholders: tabstop
                .filter(|tabstop| *tabstop != 0)
                .map(|tabstop| {
                    vec![SnippetPlaceholder {
                        tabstop,
                        start: 0,
                        end: 0,
                    }]
                })
                .unwrap_or_default(),
            cursor_offset: (tabstop == Some(0)).then_some(0),
        };
    };
    if let Ok(tabstop) = tabstop.parse::<usize>() {
        let mut expansion = expand(default);
        if tabstop == 0 {
            expansion.cursor_offset = Some(expansion.text.chars().count());
        }
        return expansion;
    }
    SnippetExpansion {
        text: body.to_string(),
        placeholders: Vec::new(),
        cursor_offset: None,
    }
}

fn placeholder_tabstop(body: &str) -> Option<usize> {
    body.split_once(':')
        .map(|(tabstop, _)| tabstop)
        .unwrap_or(body)
        .parse()
        .ok()
}

fn completion_parameter_placeholders(item: &CompletionCandidate) -> Option<Vec<String>> {
    [
        Some(item.label.as_str()),
        item.label_detail.as_deref(),
        item.detail.as_deref(),
        item.documentation
            .as_ref()
            .map(|documentation| documentation.text.as_str()),
    ]
    .into_iter()
    .flatten()
    .find_map(parameters_from_signature_text)
}

fn insert_looks_like_call_target(insert: &str) -> bool {
    empty_call_parens(insert).is_some()
        || insert
            .chars()
            .all(|character| character == '_' || character.is_alphanumeric() || character == '.')
}

fn parameters_from_signature_text(text: &str) -> Option<Vec<String>> {
    let open = text.find('(')?;
    let close = matching_signature_paren(text, open)?;
    let parameters = &text[open.saturating_add(1)..close];
    Some(
        split_top_level_commas(parameters)
            .into_iter()
            .map(parameter_placeholder_text)
            .filter(|parameter| !parameter.is_empty())
            .collect(),
    )
}

fn matching_signature_paren(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (index, character) in text.char_indices().skip_while(|(index, _)| *index < open) {
        match character {
            '(' => depth = depth.saturating_add(1),
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_top_level_commas(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;
    for (index, character) in text.char_indices() {
        match character {
            '(' => paren_depth = paren_depth.saturating_add(1),
            ')' => paren_depth = paren_depth.saturating_sub(1),
            '[' => bracket_depth = bracket_depth.saturating_add(1),
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '{' => brace_depth = brace_depth.saturating_add(1),
            '}' => brace_depth = brace_depth.saturating_sub(1),
            ',' if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 => {
                parts.push(text[start..index].trim());
                start = index.saturating_add(character.len_utf8());
            }
            _ => {}
        }
    }
    let tail = text[start..].trim();
    if !tail.is_empty() {
        parts.push(tail);
    }
    parts
}

fn parameter_placeholder_text(parameter: &str) -> String {
    let parameter = parameter.trim();
    if parameter.is_empty() || parameter == "..." {
        return String::new();
    }
    let first = parameter
        .split_once([':', '='])
        .map_or(parameter, |(name, _)| name)
        .split_whitespace()
        .next()
        .unwrap_or(parameter)
        .trim_start_matches("...")
        .trim_start_matches('*')
        .trim_start_matches('&');
    let first = first.strip_suffix('?').unwrap_or(first);
    if is_parameter_name(first) {
        first.to_string()
    } else {
        parameter.to_string()
    }
}

fn is_parameter_name(text: &str) -> bool {
    let mut characters = text.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    (first == '_' || first.is_alphabetic())
        && characters.all(|character| character == '_' || character.is_alphanumeric())
        && !matches!(
            text,
            "func" | "map" | "chan" | "interface" | "struct" | "..." | "string" | "bool" | "int"
        )
}

fn synthesize_call_snippet(insert: &str, parameters: &[String]) -> Option<SnippetExpansion> {
    let parameter_snippet = parameters
        .iter()
        .enumerate()
        .map(|(index, parameter)| {
            format!(
                "${{{}:{}}}",
                index.saturating_add(1),
                escape_snippet_text(parameter)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");

    if let Some((open, close)) = replaceable_call_parens(insert) {
        let mut snippet = String::new();
        snippet.push_str(&insert[..open.saturating_add(1)]);
        snippet.push_str(&parameter_snippet);
        snippet.push_str(&insert[close..]);
        if !snippet.contains("$0") {
            snippet.push_str("$0");
        }
        return Some(expand(&snippet));
    }

    if insert
        .chars()
        .all(|character| character == '_' || character.is_alphanumeric() || character == '.')
    {
        return Some(expand(&format!("{insert}({parameter_snippet})$0")));
    }
    None
}

fn empty_call_parens(text: &str) -> Option<(usize, usize)> {
    let open = text.find('(')?;
    let close = matching_signature_paren(text, open)?;
    text[open.saturating_add(1)..close]
        .trim()
        .is_empty()
        .then_some((open, close))
}

fn replaceable_call_parens(text: &str) -> Option<(usize, usize)> {
    let open = text.find('(')?;
    let close = matching_signature_paren(text, open)?;
    let inner = text[open.saturating_add(1)..close].trim();
    (inner.is_empty()
        || inner
            .chars()
            .all(|character| character == ',' || character.is_whitespace()))
    .then_some((open, close))
}

fn snippet_placeholders_are_empty(expansion: &SnippetExpansion) -> bool {
    !expansion.placeholders.is_empty()
        && expansion
            .placeholders
            .iter()
            .all(|placeholder| placeholder.start == placeholder.end)
}

fn escape_snippet_text(text: &str) -> String {
    text.chars()
        .flat_map(|character| match character {
            '\\' | '$' | '}' => ['\\', character],
            _ => ['\0', character],
        })
        .filter(|character| *character != '\0')
        .collect()
}
