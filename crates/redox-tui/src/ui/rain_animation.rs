use minui::prelude::TabPolicy;
use minui::{ColorPair, Window, cell_width};
use redox_core::TextBuffer;
use unicode_segmentation::UnicodeSegmentation;

use super::helpers::apply_color_column;
use super::style::UiStyle;
use super::syntax::{LineSyntaxSpan, syntax_color_for_range};

// ======================================================================================
// Credit to https://github.com/Eandrju/cellular-automaton.nvim for the inspiration here.
// One of my absolute favourite Neovim plugins!
// ======================================================================================

const SIDE_NOISE_PERCENT: u32 = 5;
const DISPERSE_RATE: usize = 7;
const RNG_MULTIPLIER: u64 = 6364136223846793005;
const RNG_INCREMENT: u64 = 1442695040888963407;

#[derive(Debug, Clone)]
struct RainParticle {
    glyph: Box<str>,
    colors: ColorPair,
    disperse_direction: i8,
    processed: bool,
}

#[derive(Debug, Clone)]
pub struct RainAnimation {
    first_line: usize,
    width: usize,
    height: usize,
    frame: u64,
    rng_state: u64,
    grid: Vec<Vec<Option<RainParticle>>>,
}

impl RainAnimation {
    pub fn capture(
        buffer: &TextBuffer,
        first_line: usize,
        scroll_x: usize,
        width: usize,
        height: usize,
        default_colors: ColorPair,
        style: UiStyle,
        syntax_spans: Option<&[Vec<LineSyntaxSpan>]>,
        color_column: Option<(usize, minui::Color)>,
    ) -> Self {
        let mut animation = Self {
            first_line,
            width,
            height,
            frame: 0,
            rng_state: 0x9e37_79b9_7f4a_7c15
                ^ (first_line as u64)
                ^ ((scroll_x as u64) << 17)
                ^ ((width as u64) << 33)
                ^ ((height as u64) << 49),
            grid: vec![vec![None; width]; height],
        };

        for row in 0..height {
            let line_idx = first_line.saturating_add(row);
            if line_idx >= buffer.len_lines() {
                break;
            }

            let source_line = buffer.line_string(line_idx);
            let spans = syntax_spans.and_then(|rows| rows.get(row).map(Vec::as_slice));
            let mut line_cells = 0usize;
            let mut used_cells = 0usize;
            let mut byte_idx = 0usize;

            for grapheme in source_line.graphemes(true) {
                let grapheme_width = cell_width(grapheme, TabPolicy::Fixed(4)) as usize;
                let start_cell = line_cells;
                let end_cell = line_cells.saturating_add(grapheme_width);
                let start_byte = byte_idx;
                let end_byte = byte_idx.saturating_add(grapheme.len());

                line_cells = end_cell;
                byte_idx = end_byte;

                if end_cell <= scroll_x {
                    continue;
                }
                if start_cell < scroll_x {
                    continue;
                }
                if used_cells.saturating_add(grapheme_width) > width {
                    break;
                }

                if grapheme_width == 1 && grapheme != "\t" && grapheme != " " {
                    let base_colors = spans
                        .map(|line_spans| {
                            syntax_color_for_range(
                                default_colors,
                                style,
                                line_spans,
                                start_byte,
                                end_byte,
                            )
                        })
                        .unwrap_or(default_colors);
                    let colors =
                        apply_color_column(base_colors, color_column, start_cell, end_cell);
                    animation.grid[row][used_cells] = Some(RainParticle {
                        glyph: grapheme.to_owned().into_boxed_str(),
                        colors,
                        disperse_direction: animation.random_direction(),
                        processed: false,
                    });
                }

                used_cells = used_cells.saturating_add(grapheme_width);
            }
        }

        animation
    }

    pub fn first_line(&self) -> usize {
        self.first_line
    }

    pub fn update(&mut self) -> bool {
        if self.width == 0 || self.height <= 1 {
            return false;
        }

        self.frame = self.frame.wrapping_add(1);
        for row in &mut self.grid {
            for particle in row.iter_mut().flatten() {
                particle.processed = false;
            }
        }

        let mut updated = false;
        for row in (0..self.height.saturating_sub(1)).rev() {
            for step in 0..self.width {
                let col = if (self.frame + row as u64).is_multiple_of(2) {
                    step
                } else {
                    self.width.saturating_sub(1).saturating_sub(step)
                };

                let Some(particle) = self.grid[row][col].as_ref() else {
                    continue;
                };
                if particle.processed {
                    continue;
                }

                if let Some(particle) = self.grid[row][col].as_mut() {
                    particle.processed = true;
                }

                let (col, shifted) = self.apply_side_noise(row, col);
                updated |= shifted;

                if self.grid[row][col].is_none() {
                    continue;
                }

                if self.cell_empty(row as isize + 1, col as isize) {
                    self.swap_cells(row, col, row + 1, col);
                    updated = true;
                    continue;
                }

                updated |= self.disperse(row, col);
            }
        }

        updated
    }

    pub fn draw(
        &self,
        window: &mut dyn Window,
        row_offset: u16,
        col_offset: u16,
        max_width: usize,
        max_height: usize,
    ) -> minui::Result<()> {
        let draw_height = self.height.min(max_height);
        let draw_width = self.width.min(max_width);

        for row in 0..draw_height {
            for col in 0..draw_width {
                let Some(particle) = self.grid[row][col].as_ref() else {
                    continue;
                };
                window.write_str_colored(
                    row_offset.saturating_add(row as u16),
                    col_offset.saturating_add(col as u16),
                    &particle.glyph,
                    particle.colors,
                )?;
            }
        }

        Ok(())
    }

    fn disperse(&mut self, row: usize, col: usize) -> bool {
        let Some(mut direction) = self.grid[row][col]
            .as_ref()
            .map(|particle| particle.disperse_direction)
        else {
            return false;
        };
        if direction != -1 && direction != 1 {
            direction = self.random_direction();
            if let Some(particle) = self.grid[row][col].as_mut() {
                particle.disperse_direction = direction;
            }
        }

        for distance in 1..=DISPERSE_RATE {
            let target_col = col as isize + (direction as isize * distance as isize);
            if !self.cell_empty(row as isize, target_col) {
                self.flip_disperse_direction(row, col);
                break;
            }

            let target_col = target_col as usize;
            if self.cell_empty(row as isize + 1, target_col as isize) {
                self.swap_cells(row, col, row + 1, target_col);
                return true;
            }
        }

        false
    }

    fn apply_side_noise(&mut self, row: usize, col: usize) -> (usize, bool) {
        if self.cell_empty(row as isize + 1, col as isize) {
            return (col, false);
        }

        let roll = self.next_rand_percent();
        let target_col = if roll < SIDE_NOISE_PERCENT {
            col.checked_add(1).filter(|target| *target < self.width)
        } else if roll < SIDE_NOISE_PERCENT * 2 {
            col.checked_sub(1)
        } else {
            None
        };

        let Some(target_col) = target_col else {
            return (col, false);
        };
        if !self.cell_empty(row as isize, target_col as isize) {
            return (col, false);
        }
        if !self.cell_empty(row as isize + 1, target_col as isize) {
            return (col, false);
        }

        self.swap_cells(row, col, row, target_col);
        (target_col, true)
    }

    fn cell_empty(&self, row: isize, col: isize) -> bool {
        if row < 0 || col < 0 {
            return false;
        }

        let row = row as usize;
        let col = col as usize;
        row < self.height && col < self.width && self.grid[row][col].is_none()
    }

    fn swap_cells(&mut self, row_a: usize, col_a: usize, row_b: usize, col_b: usize) {
        if row_a == row_b {
            self.grid[row_a].swap(col_a, col_b);
            return;
        }

        let (top, bottom) = if row_a < row_b {
            let (top, bottom) = self.grid.split_at_mut(row_b);
            (&mut top[row_a], &mut bottom[0])
        } else {
            let (top, bottom) = self.grid.split_at_mut(row_a);
            (&mut bottom[0], &mut top[row_b])
        };
        std::mem::swap(&mut top[col_a], &mut bottom[col_b]);
    }

    fn flip_disperse_direction(&mut self, row: usize, col: usize) {
        if let Some(particle) = self.grid[row][col].as_mut() {
            particle.disperse_direction *= -1;
        }
    }

    fn random_direction(&mut self) -> i8 {
        if self.next_rand_percent().is_multiple_of(2) {
            -1
        } else {
            1
        }
    }

    fn next_rand_percent(&mut self) -> u32 {
        self.rng_state = self
            .rng_state
            .wrapping_mul(RNG_MULTIPLIER)
            .wrapping_add(RNG_INCREMENT);
        ((self.rng_state >> 32) % 100) as u32
    }
}

#[cfg(test)]
mod tests {
    use minui::{Color, ColorPair};

    use super::*;

    #[test]
    fn particles_fall_to_bottom_row() {
        let buffer = TextBuffer::from_str("a");
        let mut animation = RainAnimation::capture(
            &buffer,
            0,
            0,
            4,
            4,
            ColorPair::new(Color::White, Color::Black),
            UiStyle::default(),
            None,
            None,
        );

        for _ in 0..8 {
            let _ = animation.update();
        }

        assert!(animation.grid[3].iter().any(Option::is_some));
        assert!(animation.grid[0].iter().all(Option::is_none));
    }

    #[test]
    fn spaces_do_not_create_particles() {
        let buffer = TextBuffer::from_str("a b");
        let animation = RainAnimation::capture(
            &buffer,
            0,
            0,
            3,
            1,
            ColorPair::new(Color::White, Color::Black),
            UiStyle::default(),
            None,
            None,
        );

        assert!(animation.grid[0][0].is_some());
        assert!(animation.grid[0][1].is_none());
        assert!(animation.grid[0][2].is_some());
    }
}
