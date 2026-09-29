//! Resolved text colours and formatting, independent of the drawing backend.

use minui::{Color, ColorPair, Style};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Underline {
    #[default]
    None,
    Single,
    Curl,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TextFormat {
    pub bold: bool,
    pub italic: bool,
    pub dim: bool,
    pub reverse: bool,
    pub strikethrough: bool,
    pub underline: Underline,
    pub underline_color: Option<Color>,
}

impl TextFormat {
    pub fn merge(self, overlay: Self) -> Self {
        Self {
            bold: self.bold || overlay.bold,
            italic: self.italic || overlay.italic,
            dim: self.dim || overlay.dim,
            reverse: self.reverse || overlay.reverse,
            strikethrough: self.strikethrough || overlay.strikethrough,
            underline: if overlay.underline == Underline::None {
                self.underline
            } else {
                overlay.underline
            },
            underline_color: overlay.underline_color.or(self.underline_color),
        }
    }

    /// Combine enabled decorations; the later role's underline takes precedence.
    pub fn apply_to(self, mut style: Style) -> Style {
        if self.bold {
            style = style.bold();
        }
        if self.italic {
            style = style.italic();
        }
        if self.dim {
            style = style.dim();
        }
        if self.reverse {
            style = style.reversed();
        }
        if self.strikethrough {
            style = style.strikethrough();
        }
        style = match self.underline {
            Underline::None => style,
            Underline::Single => style.underlined(),
            Underline::Curl => style.undercurled(),
        };
        if let Some(color) = self.underline_color {
            style = style.with_underline_color(color);
        }
        style
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextStyle {
    pub fg: Color,
    pub bg: Color,
    pub format: TextFormat,
}

impl TextStyle {
    pub fn new(fg: Color, bg: Color) -> Self {
        Self {
            fg,
            bg,
            format: TextFormat::default(),
        }
    }

    pub fn colors(self) -> ColorPair {
        ColorPair::new(self.fg, self.bg)
    }

    pub fn with_colors(self, fg: Color, bg: Color) -> Self {
        Self { fg, bg, ..self }
    }

    /// Keep semantic foreground colours while applying selection styling.
    pub fn selected(self, selection: Self) -> Self {
        Self {
            bg: selection.bg,
            format: self.format.merge(selection.format),
            ..self
        }
    }

    /// Overlay roles use transparent colours to inherit the text underneath.
    pub fn overlay(self, mut base: Style) -> Style {
        if let Some(colors) = base.colors {
            base = base.with_colors(ColorPair::new(
                if self.fg == Color::Transparent {
                    colors.fg
                } else {
                    self.fg
                },
                if self.bg == Color::Transparent {
                    colors.bg
                } else {
                    self.bg
                },
            ));
        }
        self.format.apply_to(base)
    }

    pub fn bold(mut self) -> Self {
        self.format.bold = true;
        self
    }

    pub fn italic(mut self) -> Self {
        self.format.italic = true;
        self
    }

    pub fn underlined(mut self) -> Self {
        self.format.underline = Underline::Single;
        self
    }
}

impl From<TextStyle> for Style {
    fn from(style: TextStyle) -> Self {
        style.format.apply_to(style.colors().into())
    }
}
