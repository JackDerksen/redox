use serde::{Deserialize, Serialize};
use serde_json::Value;

/// An LSP position. `character` counts UTF-16 code units.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub struct Position {
    pub line: u64,
    pub character: u64,
}

/// An LSP range whose end position is exclusive.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionTarget {
    pub uri: String,
    pub range: Range,
}

#[derive(Debug, Deserialize)]
struct Location {
    uri: String,
    range: Range,
}

#[derive(Debug, Deserialize)]
struct LocationLink {
    #[serde(rename = "targetUri")]
    target_uri: String,
    #[serde(rename = "targetSelectionRange")]
    target_selection_range: Range,
}

/// Parses the first definition target from either `Location` or
/// `LocationLink` response forms.
#[must_use]
pub fn parse_definition_response(message: &Value) -> Option<DefinitionTarget> {
    let result = message.get("result")?;
    if result.is_null() {
        return None;
    }

    let target = if let Some(entries) = result.as_array() {
        entries.first()?
    } else {
        result
    };
    parse_definition_target(target)
}

fn parse_definition_target(value: &Value) -> Option<DefinitionTarget> {
    if let Ok(location) = serde_json::from_value::<Location>(value.clone()) {
        return Some(DefinitionTarget {
            uri: location.uri,
            range: location.range,
        });
    }
    let link = serde_json::from_value::<LocationLink>(value.clone()).ok()?;
    Some(DefinitionTarget {
        uri: link.target_uri,
        range: link.target_selection_range,
    })
}

/// Converts an LSP UTF-16 column to a Rust character index.
#[must_use]
pub fn utf16_code_unit_to_char_col(line: &str, utf16_col: u32) -> usize {
    let mut consumed_utf16 = 0u32;
    let mut chars = 0usize;
    for character in line.chars() {
        if consumed_utf16 >= utf16_col {
            break;
        }
        consumed_utf16 = consumed_utf16.saturating_add(character.len_utf16() as u32);
        chars = chars.saturating_add(1);
    }
    chars
}

/// Converts a Rust character index to an LSP UTF-16 column.
#[must_use]
pub fn char_col_to_utf16(line: &str, char_col: usize) -> u32 {
    line.chars()
        .take(char_col)
        .map(|character| character.len_utf16() as u32)
        .fold(0u32, u32::saturating_add)
}
