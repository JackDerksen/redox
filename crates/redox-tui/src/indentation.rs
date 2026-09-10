//! Infer space indentation from the current buffer, including formatter output.
//! Literal tabs retain their existing four-cell display width.

use redox_core::TextBuffer;

pub(crate) const DEFAULT_WIDTH: usize = 4;
const SAMPLE_LINES: usize = 1024;

pub(crate) fn detect_buffer_width(buffer: &TextBuffer) -> Option<usize> {
    detect((0..buffer.len_lines().min(SAMPLE_LINES)).map(|line| buffer.line_slice(line).chars()))
}

pub(crate) fn width_for_text(text: &str) -> usize {
    detect(text.lines().take(SAMPLE_LINES).map(str::chars)).unwrap_or(DEFAULT_WIDTH)
}

fn detect(lines: impl IntoIterator<Item = impl Iterator<Item = char>>) -> Option<usize> {
    let mut steps = [0_usize; 9];
    let mut levels = [0_usize; 9];
    let mut previous = 0_usize;
    for line in lines {
        // Only inspect prefixes; long lines and large files need no allocation.
        let Some((indent, first)) = line.take(256).enumerate().find(|(_, ch)| *ch != ' ') else {
            continue;
        };
        if first.is_whitespace() || matches!(first, '/' | '*' | '#') {
            continue;
        }
        if let Some(count) = steps.get_mut(indent.abs_diff(previous)) {
            *count += 1;
        }
        if let Some(count) = levels.get_mut(indent) {
            *count += 1;
        }
        previous = indent;
    }
    // A single aligned continuation or partially typed indent is not evidence.
    // Prefer the default on ties, then the smaller step. One-space comment
    // margins must not turn a whole document into one-space indentation.
    (2..=8)
        .filter(|&width| steps[width] >= 2 || levels[width] >= 2)
        .max_by_key(|&width| {
            (
                steps[width],
                levels[width],
                width == DEFAULT_WIDTH,
                8 - width,
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_dominant_steps_without_following_alignment_or_comment_margins() {
        for (text, expected) in [
            ("", 4),
            ("int main() {\n  return 0;\n}\n", 2),
            ("a {\n   b {\n      c;\n   }\n}\n", 3),
            ("a {\n        b;\n}\n", 8),
            ("a {\n    b(\n      aligned);\n    c;\n}\n", 4),
            ("/* comment\n * margin\n */\na {\n  b;\n}\n", 2),
            ("\t  if ready {", 4),
            ("a {\n\tb;\n}\n", 4),
            ("  a\n  b", 2),
        ] {
            assert_eq!(width_for_text(text), expected, "{text:?}");
            assert_eq!(
                detect_buffer_width(&TextBuffer::from_text(text)).unwrap_or(DEFAULT_WIDTH),
                expected
            );
        }
    }
}
