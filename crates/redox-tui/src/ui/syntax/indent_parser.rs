//! Incremental Tree-sitter parses for smart indentation on the UI thread.

use redox_core::{TextBuffer, TextDiff};
use tree_sitter::{InputEdit, Parser, Point, Tree};

use super::{SyntaxHighlighter, SyntaxLanguage, language_config_for, tree_edit};

/// Keeps the last tree so each smart-indent action reparses only what changed.
#[derive(Default)]
pub(crate) struct IndentParser {
    parser: Option<(SyntaxLanguage, Parser)>,
    previous: Option<PreviousParse>,
}

struct PreviousParse {
    language: SyntaxLanguage,
    source: TextBuffer,
    tree: Tree,
    /// False when `tree` was only edited to match `source`, not reparsed.
    parsed: bool,
}

impl std::fmt::Debug for IndentParser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndentParser").finish_non_exhaustive()
    }
}

impl IndentParser {
    /// Start from the highlighter's tree, which is kept edited to match its buffer.
    pub(crate) fn seed_from(&mut self, highlighter: &SyntaxHighlighter) {
        if let Some((language, source, tree)) = highlighter.parse_seed() {
            self.previous = Some(PreviousParse {
                language,
                source: source.clone(),
                tree: tree.clone(),
                parsed: false,
            });
        }
    }

    /// Parse `source`, the full text of `buffer`.
    pub(super) fn parse_buffer(
        &mut self,
        buffer: &TextBuffer,
        source: &str,
        language: SyntaxLanguage,
    ) -> Option<Tree> {
        let mut old_tree = None;
        if let Some(mut previous) = self.previous.take()
            && previous.language == language
        {
            match TextDiff::between(&previous.source, buffer) {
                None if previous.parsed => {
                    let tree = previous.tree.clone();
                    self.previous = Some(previous);
                    return Some(tree);
                }
                None => {}
                Some(change) => previous
                    .tree
                    .edit(&tree_edit(&previous.source, buffer, &change)),
            }
            old_tree = Some(previous.tree);
        }
        let tree = self.parser(language)?.parse(source, old_tree.as_ref())?;
        self.previous = Some(PreviousParse {
            language,
            source: buffer.clone(),
            tree: tree.clone(),
            parsed: true,
        });
        Some(tree)
    }

    /// Parse `source`, which is `tree`'s text after `edit`.
    pub(super) fn reparse_edited(
        &mut self,
        tree: &Tree,
        edit: &InputEdit,
        source: &str,
        language: SyntaxLanguage,
    ) -> Option<Tree> {
        let mut old_tree = tree.clone();
        old_tree.edit(edit);
        self.parser(language)?.parse(source, Some(&old_tree))
    }

    fn parser(&mut self, language: SyntaxLanguage) -> Option<&mut Parser> {
        if self
            .parser
            .as_ref()
            .is_none_or(|(current, _)| *current != language)
        {
            let mut parser = Parser::new();
            parser
                .set_language(&(language_config_for(language)?.grammar)())
                .ok()?;
            self.parser = Some((language, parser));
        }
        self.parser.as_mut().map(|(_, parser)| parser)
    }
}

/// The edit replacing `before[start..old_end]` with `inserted`.
pub(super) fn string_edit(before: &str, start: usize, old_end: usize, inserted: &str) -> InputEdit {
    let start_position = point_at(before, start);
    InputEdit {
        start_byte: start,
        old_end_byte: old_end,
        new_end_byte: start + inserted.len(),
        start_position,
        old_end_position: point_at(before, old_end),
        new_end_position: advance_point(start_position, inserted),
    }
}

fn point_at(text: &str, byte: usize) -> Point {
    advance_point(Point::new(0, 0), &text[..byte])
}

fn advance_point(start: Point, text: &str) -> Point {
    match text.rfind('\n') {
        Some(last) => Point::new(
            start.row + text.bytes().filter(|&byte| byte == b'\n').count(),
            text.len() - last - 1,
        ),
        None => Point::new(start.row, start.column + text.len()),
    }
}

#[cfg(test)]
mod tests {
    use redox_core::TextBuffer;
    use tree_sitter::Point;

    use super::{IndentParser, SyntaxHighlighter, SyntaxLanguage, string_edit};

    #[test]
    fn string_edit_reports_tree_sitter_points() {
        let edit = string_edit("ab\ncd\nef", 4, 7, "x\ny\nzz");
        assert_eq!(edit.start_position, Point::new(1, 1));
        assert_eq!(edit.old_end_position, Point::new(2, 1));
        assert_eq!(edit.new_end_position, Point::new(3, 2));
        assert_eq!(edit.new_end_byte, 10);
    }

    #[test]
    fn reused_trees_match_fresh_parses_after_edits() {
        let mut parser = IndentParser::default();
        let mut buffer = TextBuffer::from_text("fn main() {\n    let x = 1;\n}\n");
        let source = buffer.to_string();
        let _ = parser.parse_buffer(&buffer, &source, SyntaxLanguage::Rust);
        for (pos, text) in [
            (redox_core::Pos::new(1, 14), "\n    if x > 0 {"),
            (redox_core::Pos::new(2, 18), "\n        x;\n    }"),
            (redox_core::Pos::new(0, 0), "/* open "),
        ] {
            let _ = buffer.insert(pos, text);
            let source = buffer.to_string();
            let reused = parser
                .parse_buffer(&buffer, &source, SyntaxLanguage::Rust)
                .unwrap();
            let fresh = super::super::parse_tree(&source, SyntaxLanguage::Rust).unwrap();
            assert_eq!(
                reused.root_node().to_sexp(),
                fresh.root_node().to_sexp(),
                "{source}"
            );

            let edited = format!("{source}\n*/");
            let edit = string_edit(&source, source.len(), source.len(), "\n*/");
            let reused = parser
                .reparse_edited(&reused, &edit, &edited, SyntaxLanguage::Rust)
                .unwrap();
            let fresh = super::super::parse_tree(&edited, SyntaxLanguage::Rust).unwrap();
            assert_eq!(reused.root_node().to_sexp(), fresh.root_node().to_sexp());
        }
    }

    #[test]
    fn seeding_from_a_rebased_highlight_tree_matches_a_fresh_parse() {
        let mut buffer = TextBuffer::from_text("fn main() {\n    let x = 1;\n}\n");
        let mut highlighter = SyntaxHighlighter::default();
        highlighter.replace_cache(SyntaxHighlighter::compute_cache(
            &buffer,
            SyntaxLanguage::Rust,
        ));
        let _ = buffer.insert(redox_core::Pos::new(1, 14), "\n    while x {\n    }");
        highlighter.rebase_cache(&buffer);
        let _ = buffer.insert(redox_core::Pos::new(0, 0), "// ");

        let mut parser = IndentParser::default();
        parser.seed_from(&highlighter);
        let source = buffer.to_string();
        let reused = parser
            .parse_buffer(&buffer, &source, SyntaxLanguage::Rust)
            .unwrap();
        let fresh = super::super::parse_tree(&source, SyntaxLanguage::Rust).unwrap();
        assert_eq!(reused.root_node().to_sexp(), fresh.root_node().to_sexp());
    }
}
