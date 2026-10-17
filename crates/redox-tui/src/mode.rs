/// Vim-like editor mode, shared by key dispatch and editor state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EditorMode {
    Normal,
    Insert,
    Command,
    Search,
    Finder,
    PinSelect,
    LspMarketplace,
    DiagnosticsList,
    CodeActions,
    SymbolInfo,
    Visual,
    VisualLine,
    VisualBlock,
}

impl EditorMode {
    pub fn has_popup_overlay(self) -> bool {
        matches!(
            self,
            EditorMode::Command
                | EditorMode::Finder
                | EditorMode::PinSelect
                | EditorMode::LspMarketplace
                | EditorMode::DiagnosticsList
                | EditorMode::CodeActions
                | EditorMode::SymbolInfo
        )
    }

    pub fn is_visual(self) -> bool {
        matches!(
            self,
            EditorMode::Visual | EditorMode::VisualLine | EditorMode::VisualBlock
        )
    }
}
