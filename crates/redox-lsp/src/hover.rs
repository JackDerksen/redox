use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolInfoBlock {
    pub kind: SymbolInfoKind,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolInfoKind {
    PlainText,
    Markdown,
    Code { language: Option<String> },
}

/// Parses and normalises an LSP hover response into display-independent
/// content blocks.
#[must_use]
pub fn parse_hover_response(message: &Value) -> Vec<SymbolInfoBlock> {
    let Some(result) = message.get("result") else {
        return Vec::new();
    };
    if result.is_null() {
        return Vec::new();
    }
    let Some(contents) = result.get("contents") else {
        return Vec::new();
    };
    normalize_blocks(contents_blocks(contents))
}

fn contents_blocks(value: &Value) -> Vec<SymbolInfoBlock> {
    if let Some(text) = value.as_str() {
        return vec![SymbolInfoBlock {
            kind: SymbolInfoKind::Markdown,
            text: text.to_string(),
        }];
    }
    if let Some(array) = value.as_array() {
        return array.iter().flat_map(contents_blocks).collect();
    }
    if let Some(kind) = value.get("kind").and_then(Value::as_str) {
        let text = value
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let kind = match kind {
            "markdown" => SymbolInfoKind::Markdown,
            _ => SymbolInfoKind::PlainText,
        };
        return vec![SymbolInfoBlock { kind, text }];
    }
    if value.get("language").is_some() || value.get("value").is_some() {
        return vec![marked_string_block(value)];
    }
    Vec::new()
}

fn marked_string_block(value: &Value) -> SymbolInfoBlock {
    let language = value
        .get("language")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    if let Some(text) = value.as_str() {
        return SymbolInfoBlock {
            kind: SymbolInfoKind::Markdown,
            text: text.to_string(),
        };
    }
    let text = value
        .get("value")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    SymbolInfoBlock {
        kind: SymbolInfoKind::Code { language },
        text,
    }
}

fn normalize_blocks(blocks: Vec<SymbolInfoBlock>) -> Vec<SymbolInfoBlock> {
    blocks
        .into_iter()
        .filter_map(|block| {
            let SymbolInfoBlock { kind, text } = block;
            let text = match &kind {
                SymbolInfoKind::Code { .. } => {
                    trim_blank_edges(&trim_trailing_whitespace_lines(&text))
                }
                SymbolInfoKind::Markdown | SymbolInfoKind::PlainText => {
                    collapse_blank_lines(&trim_trailing_whitespace_lines(&text))
                }
            };
            (!text.is_empty()).then_some(SymbolInfoBlock { kind, text })
        })
        .collect()
}

fn trim_trailing_whitespace_lines(text: &str) -> String {
    text.lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

fn trim_blank_edges(text: &str) -> String {
    text.lines()
        .skip_while(|line| line.trim().is_empty())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .skip_while(|line| line.trim().is_empty())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n")
}

fn collapse_blank_lines(text: &str) -> String {
    let mut lines = Vec::new();
    let mut last_was_blank = true;
    for line in text.lines() {
        let is_blank = line.trim().is_empty();
        if is_blank {
            if !last_was_blank {
                lines.push(String::new());
            }
        } else {
            lines.push(line.to_string());
        }
        last_was_blank = is_blank;
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.join("\n")
}
