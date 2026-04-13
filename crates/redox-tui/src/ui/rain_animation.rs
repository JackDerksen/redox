use minui::{ColorPair, TabPolicy, Window, cell_width};
use redox_core::TextBuffer;

use super::helpers::apply_color_column;
use super::render::GraphemeCache;
use super::style::UiStyle;
use super::syntax::{VisibleLineSyntaxSpans, syntax_color_for_range};

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
    width: usize,
}

#[derive(Debug, Clone)]
enum RainCell {
    Empty,
    Head(RainParticle),
    Tail,
}

#[derive(Debug, Clone)]
pub struct RainAnimation {
    first_line: usize,
    width: usize,
    height: usize,
    frame: u64,
    rng_state: u64,
    grid: Vec<Vec<RainCell>>,
}

impl RainAnimation {
    pub fn capture(
        buffer: &TextBuffer,
        cache: &mut GraphemeCache,
        first_line: usize,
        scroll_x: usize,
        width: usize,
        height: usize,
        default_colors: ColorPair,
        style: UiStyle,
        syntax_spans: Option<VisibleLineSyntaxSpans<'_>>,
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
            grid: vec![vec![RainCell::Empty; width]; height],
        };

        for row in 0..height {
            let line_idx = first_line.saturating_add(row);
            if line_idx >= buffer.len_lines() {
                break;
            }

            let graphemes = cache.graphemes_for_line(buffer, line_idx);
            let start_g = skip_graphemes_by_cells(graphemes, scroll_x);
            let spans = syntax_spans.and_then(|rows| rows.get(row));
            let mut used_cells = 0usize;
            let mut byte_idx: usize = graphemes[..start_g].iter().map(|g| g.len()).sum();

            for grapheme in &graphemes[start_g..] {
                let grapheme_width = cell_width(grapheme, TabPolicy::Fixed(4)) as usize;
                let start_byte = byte_idx;
                let end_byte = byte_idx.saturating_add(grapheme.len());

                if grapheme_width == 0 {
                    byte_idx = end_byte;
                    continue;
                }
                if used_cells.saturating_add(grapheme_width) > width {
                    break;
                }

                if grapheme.as_ref() != "\t" && grapheme.as_ref() != " " {
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
                    let colors = apply_color_column(
                        base_colors,
                        color_column,
                        used_cells,
                        used_cells.saturating_add(grapheme_width),
                    );
                    let disperse_direction = animation.random_direction();
                    animation.place_particle(
                        row,
                        used_cells,
                        RainParticle {
                            glyph: grapheme.to_owned(),
                            colors,
                            disperse_direction,
                            processed: false,
                            width: grapheme_width,
                        },
                    );
                }

                byte_idx = end_byte;
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
            for cell in row {
                if let RainCell::Head(particle) = cell {
                    particle.processed = false;
                }
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

                let Some((particle_width, processed)) = self.head_state(row, col) else {
                    continue;
                };
                if processed {
                    continue;
                }

                if let RainCell::Head(particle) = &mut self.grid[row][col] {
                    particle.processed = true;
                }

                let (col, shifted) = self.apply_side_noise(row, col, particle_width);
                updated |= shifted;

                if !matches!(self.grid[row][col], RainCell::Head(_)) {
                    continue;
                }

                if self.range_empty(row as isize + 1, col as isize, particle_width) {
                    self.move_particle(row, col, row + 1, col);
                    updated = true;
                    continue;
                }

                updated |= self.disperse(row, col, particle_width);
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
                let RainCell::Head(particle) = &self.grid[row][col] else {
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

    fn disperse(&mut self, row: usize, col: usize, particle_width: usize) -> bool {
        let Some(mut direction) = self.head_direction(row, col) else {
            return false;
        };
        if direction != -1 && direction != 1 {
            direction = self.random_direction();
            if let RainCell::Head(particle) = &mut self.grid[row][col] {
                particle.disperse_direction = direction;
            }
        }

        for distance in 1..=DISPERSE_RATE {
            let target_col = col as isize + (direction as isize * distance as isize);
            if !self.range_empty(row as isize, target_col, particle_width) {
                self.flip_disperse_direction(row, col);
                break;
            }

            let target_col = target_col as usize;
            if self.range_empty(row as isize + 1, target_col as isize, particle_width) {
                self.move_particle(row, col, row + 1, target_col);
                return true;
            }
        }

        false
    }

    fn apply_side_noise(&mut self, row: usize, col: usize, particle_width: usize) -> (usize, bool) {
        if self.range_empty(row as isize + 1, col as isize, particle_width) {
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
        if !self.range_empty(row as isize, target_col as isize, particle_width) {
            return (col, false);
        }
        if !self.range_empty(row as isize + 1, target_col as isize, particle_width) {
            return (col, false);
        }

        self.move_particle(row, col, row, target_col);
        (target_col, true)
    }

    fn range_empty(&self, row: isize, col: isize, width: usize) -> bool {
        if row < 0 || col < 0 || width == 0 {
            return false;
        }

        let row = row as usize;
        let col = col as usize;
        if row >= self.height || col >= self.width || col.saturating_add(width) > self.width {
            return false;
        }

        self.grid[row][col..col + width]
            .iter()
            .all(|cell| matches!(cell, RainCell::Empty))
    }

    fn head_state(&self, row: usize, col: usize) -> Option<(usize, bool)> {
        match &self.grid[row][col] {
            RainCell::Head(particle) => Some((particle.width, particle.processed)),
            RainCell::Empty | RainCell::Tail => None,
        }
    }

    fn head_direction(&self, row: usize, col: usize) -> Option<i8> {
        match &self.grid[row][col] {
            RainCell::Head(particle) => Some(particle.disperse_direction),
            RainCell::Empty | RainCell::Tail => None,
        }
    }

    fn move_particle(&mut self, from_row: usize, from_col: usize, to_row: usize, to_col: usize) {
        let particle = self.take_particle(from_row, from_col);
        self.place_particle(to_row, to_col, particle);
    }

    fn take_particle(&mut self, row: usize, col: usize) -> RainParticle {
        let RainCell::Head(particle) = std::mem::replace(&mut self.grid[row][col], RainCell::Empty)
        else {
            panic!("attempted to move a non-head rain particle");
        };

        for offset in 1..particle.width {
            self.grid[row][col + offset] = RainCell::Empty;
        }

        particle
    }

    fn place_particle(&mut self, row: usize, col: usize, particle: RainParticle) {
        let width = particle.width;
        self.grid[row][col] = RainCell::Head(particle);
        for offset in 1..width {
            self.grid[row][col + offset] = RainCell::Tail;
        }
    }

    fn flip_disperse_direction(&mut self, row: usize, col: usize) {
        if let RainCell::Head(particle) = &mut self.grid[row][col] {
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

fn skip_graphemes_by_cells(graphemes: &[Box<str>], skip_cells: usize) -> usize {
    if skip_cells == 0 || graphemes.is_empty() {
        return 0;
    }

    let mut skipped = 0usize;
    for (idx, grapheme) in graphemes.iter().enumerate() {
        if skipped >= skip_cells {
            return idx;
        }
        skipped = skipped.saturating_add(cell_width(grapheme, TabPolicy::Fixed(4)) as usize);
    }

    graphemes.len()
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
            &mut GraphemeCache::new(8),
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

        assert!(
            animation.grid[3]
                .iter()
                .any(|cell| matches!(cell, RainCell::Head(_)))
        );
        assert!(
            animation.grid[0]
                .iter()
                .all(|cell| matches!(cell, RainCell::Empty))
        );
    }

    #[test]
    fn spaces_do_not_create_particles() {
        let buffer = TextBuffer::from_str("a b");
        let animation = RainAnimation::capture(
            &buffer,
            &mut GraphemeCache::new(8),
            0,
            0,
            3,
            1,
            ColorPair::new(Color::White, Color::Black),
            UiStyle::default(),
            None,
            None,
        );

        assert!(matches!(animation.grid[0][0], RainCell::Head(_)));
        assert!(matches!(animation.grid[0][1], RainCell::Empty));
        assert!(matches!(animation.grid[0][2], RainCell::Head(_)));
    }

    #[test]
    fn wide_glyphs_occupy_their_full_cell_width() {
        let buffer = TextBuffer::from_str("界a");
        let animation = RainAnimation::capture(
            &buffer,
            &mut GraphemeCache::new(8),
            0,
            0,
            4,
            1,
            ColorPair::new(Color::White, Color::Black),
            UiStyle::default(),
            None,
            None,
        );

        assert!(matches!(animation.grid[0][0], RainCell::Head(_)));
        assert!(matches!(animation.grid[0][1], RainCell::Tail));
        assert!(matches!(animation.grid[0][2], RainCell::Head(_)));
    }
}
