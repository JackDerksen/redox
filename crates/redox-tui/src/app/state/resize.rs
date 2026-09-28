//! Pane boundary geometry and resizing shared by keyboard and mouse input.

use minui::widgets::WidgetArea as Rect;

use super::{EditorState, PaneId, SplitAxis, SplitDirection, SplitNode, SplitSize, split_lengths};

#[derive(Debug, Clone, Copy)]
pub(super) enum SplitSide {
    First,
    Second,
}

#[derive(Debug, Clone)]
pub(super) enum ResizeTarget {
    Editor(Vec<SplitSide>),
    Terminal,
}

impl EditorState {
    pub(crate) fn minimum_editor_height(&self) -> u16 {
        self.split_root
            .minimum_size(SplitAxis::Horizontal)
            .saturating_add(1)
    }

    pub(crate) fn resize_active_pane(&mut self, direction: SplitDirection) {
        let (axis, amount) = match direction {
            SplitDirection::Left => (SplitAxis::Vertical, -1),
            SplitDirection::Right => (SplitAxis::Vertical, 1),
            SplitDirection::Down => (SplitAxis::Horizontal, -1),
            SplitDirection::Up => (SplitAxis::Horizontal, 1),
        };
        if self.terminal.is_focused() {
            if axis == SplitAxis::Horizontal {
                self.terminal.resize_height(amount);
                self.request_redraw();
            }
            return;
        }
        if !self.pane_options(self.active_pane).resizable {
            return;
        }
        let mut path = Vec::new();
        if !self.split_root.path_to_pane(self.active_pane, &mut path) {
            return;
        }
        for depth in (0..path.len()).rev() {
            let Some((node, area)) = self.split_root.at_path(&path[..depth], self.resize_area())
            else {
                continue;
            };
            let SplitNode::Split {
                axis: split_axis, ..
            } = node
            else {
                continue;
            };
            if *split_axis != axis || !self.can_resize_node(node) {
                continue;
            }
            let (first_area, _) = node.child_rects(area).expect("split has child rectangles");
            let first_length = match axis {
                SplitAxis::Vertical => first_area.width,
                SplitAxis::Horizontal => first_area.height,
            };
            let first_change = match path[depth] {
                SplitSide::First => amount,
                SplitSide::Second => -amount,
            };
            let requested = i32::from(first_length) + i32::from(first_change);
            self.resize_editor_split(&path[..depth], requested);
            return;
        }
        if axis == SplitAxis::Horizontal && self.terminal.is_visible() {
            self.terminal.resize_height(-amount);
            self.request_redraw();
        }
    }

    pub(super) fn resize_target_at(&self, column: u16, row: u16) -> Option<ResizeTarget> {
        let mut path = Vec::new();
        self.split_root
            .divider_at(self.resize_area(), column, row, &mut path)?;
        let (node, _) = self.split_root.at_path(&path, self.resize_area())?;
        self.can_resize_node(node)
            .then_some(ResizeTarget::Editor(path))
    }

    pub(super) fn drag_pane_resize(&mut self, target: &ResizeTarget, column: u16, row: u16) {
        match target {
            ResizeTarget::Terminal => {
                self.terminal.move_separator(row);
                self.request_redraw();
            }
            ResizeTarget::Editor(path) => {
                let Some((SplitNode::Split { axis, .. }, area)) =
                    self.split_root.at_path(path, self.resize_area())
                else {
                    return;
                };
                let length = match axis {
                    SplitAxis::Vertical => i32::from(column) - i32::from(area.x),
                    SplitAxis::Horizontal => i32::from(row) - i32::from(area.y),
                };
                self.resize_editor_split(path, length);
            }
        }
    }

    fn resize_area(&self) -> Rect {
        Rect {
            x: 0,
            y: 0,
            width: self.editor_area_width_cells as u16,
            height: self.editor_area_height_rows as u16,
        }
    }

    fn can_resize_node(&self, node: &SplitNode) -> bool {
        match node {
            SplitNode::Pane(id) => self.pane_options(*id).resizable,
            SplitNode::Split { first, second, .. } => {
                self.can_resize_node(first) && self.can_resize_node(second)
            }
        }
    }

    fn resize_editor_split(&mut self, path: &[SplitSide], requested: i32) {
        let area = self.resize_area();
        let Some((node, bounds)) = self.split_root.at_path(path, area) else {
            return;
        };
        if !self.can_resize_node(node) {
            return;
        }
        let Some((minimum, maximum)) = node.resize_limits(bounds) else {
            return;
        };
        let first_length = requested.clamp(i32::from(minimum), i32::from(maximum)) as u16;
        let mut node = &mut self.split_root;
        for side in path {
            let SplitNode::Split { first, second, .. } = node else {
                return;
            };
            node = match side {
                SplitSide::First => first,
                SplitSide::Second => second,
            };
        }
        if let SplitNode::Split { size, .. } = node {
            *size = SplitSize {
                first: Some(first_length),
                ..SplitSize::default()
            };
            self.refresh_active_split_viewport_size();
            self.request_redraw();
        }
    }
}

impl SplitNode {
    pub(super) fn child_rects(&self, area: Rect) -> Option<(Rect, Rect)> {
        let Self::Split { axis, size, .. } = self else {
            return None;
        };
        let total = match axis {
            SplitAxis::Vertical => area.width,
            SplitAxis::Horizontal => area.height,
        };
        let (first_length, _) = split_lengths(total, *size);
        let (minimum, maximum) = self.resize_limits(area)?;
        let first_length = first_length.clamp(minimum, maximum);
        let second_length = total.saturating_sub(1).saturating_sub(first_length);
        Some(match axis {
            SplitAxis::Vertical => (
                Rect {
                    width: first_length,
                    ..area
                },
                Rect {
                    x: area.x.saturating_add(first_length).saturating_add(1),
                    width: second_length,
                    ..area
                },
            ),
            SplitAxis::Horizontal => (
                Rect {
                    height: first_length,
                    ..area
                },
                Rect {
                    y: area.y.saturating_add(first_length).saturating_add(1),
                    height: second_length,
                    ..area
                },
            ),
        })
    }

    fn resize_limits(&self, area: Rect) -> Option<(u16, u16)> {
        let Self::Split {
            axis,
            first,
            second,
            ..
        } = self
        else {
            return None;
        };
        let available = match axis {
            SplitAxis::Vertical => area.width,
            SplitAxis::Horizontal => area.height,
        }
        .saturating_sub(1);
        let mut minimum = first.minimum_size(*axis);
        let mut second_minimum = second.minimum_size(*axis);
        // Relax practical minima only when the outer terminal cannot fit them.
        if u32::from(minimum) + u32::from(second_minimum) > u32::from(available) {
            minimum = minimum.min(available / 2);
            second_minimum = second_minimum.min(available / 2);
        }
        let maximum = available.saturating_sub(second_minimum);
        Some((minimum, maximum))
    }

    fn minimum_size(&self, axis: SplitAxis) -> u16 {
        match self {
            Self::Pane(_) => match axis {
                SplitAxis::Vertical => 12,
                SplitAxis::Horizontal => 3,
            },
            Self::Split {
                axis: split_axis,
                first,
                second,
                ..
            } => {
                let first = first.minimum_size(axis);
                let second = second.minimum_size(axis);
                if axis == *split_axis {
                    first.saturating_add(second).saturating_add(1)
                } else {
                    first.max(second)
                }
            }
        }
    }

    fn path_to_pane(&self, pane: PaneId, path: &mut Vec<SplitSide>) -> bool {
        match self {
            Self::Pane(id) => *id == pane,
            Self::Split { first, second, .. } => {
                for (side, child) in [(SplitSide::First, first), (SplitSide::Second, second)] {
                    path.push(side);
                    if child.path_to_pane(pane, path) {
                        return true;
                    }
                    path.pop();
                }
                false
            }
        }
    }

    fn at_path<'a>(&'a self, path: &[SplitSide], mut area: Rect) -> Option<(&'a Self, Rect)> {
        let mut node = self;
        for side in path {
            let (first_area, second_area) = node.child_rects(area)?;
            let Self::Split { first, second, .. } = node else {
                return None;
            };
            (node, area) = match side {
                SplitSide::First => (first, first_area),
                SplitSide::Second => (second, second_area),
            };
        }
        Some((node, area))
    }

    fn divider_at(
        &self,
        area: Rect,
        column: u16,
        row: u16,
        path: &mut Vec<SplitSide>,
    ) -> Option<()> {
        if column < area.x
            || row < area.y
            || column >= area.x.saturating_add(area.width)
            || row >= area.y.saturating_add(area.height)
        {
            return None;
        }
        let Self::Split {
            axis,
            first,
            second,
            ..
        } = self
        else {
            return None;
        };
        let (first_area, second_area) = self.child_rects(area)?;
        let on_divider = match axis {
            SplitAxis::Vertical => column == area.x + first_area.width,
            SplitAxis::Horizontal => row == area.y + first_area.height,
        };
        if on_divider {
            return Some(());
        }
        for (side, child, child_area) in [
            (SplitSide::First, first, first_area),
            (SplitSide::Second, second, second_area),
        ] {
            path.push(side);
            if child.divider_at(child_area, column, row, path).is_some() {
                return Some(());
            }
            path.pop();
        }
        None
    }
}
