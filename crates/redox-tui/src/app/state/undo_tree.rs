use std::collections::HashMap;
use std::time::UNIX_EPOCH;

use redox_core::{
    BufferId, Pos, TextBuffer, TextDiff, UndoHistory, UndoNodeId, UndoRecord, UndoTreeEntry,
    motion::Motion,
};

use super::{BufferViewState, EditorMode, EditorState, PaneId, PaneOptions, SplitAxis, SplitSize};
use crate::ui::style::UndoTreeStyle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndoTreeSurfaceRole {
    Tree,
    Preview,
}

#[derive(Debug, Clone)]
pub(super) struct UndoTreeState {
    pub(super) buffer_id: BufferId,
    pub(super) diff_buffer_id: BufferId,
    pub(super) source_buffer_id: BufferId,
    pub(super) pane_id: PaneId,
    pub(super) diff_pane_id: PaneId,
    pub(super) selected_node: UndoNodeId,
    pub(super) display_rows: Vec<Option<UndoNodeId>>,
    rendered_at_ms: u128,
}

impl EditorState {
    pub(super) fn undo_tree_is_active(&self) -> bool {
        self.undo_tree
            .as_ref()
            .is_some_and(|tree| tree.buffer_id == self.session.active_id())
    }

    pub(super) fn command_toggle_undo_tree(&mut self) {
        if self.close_undo_tree_if_open() {
            return;
        }
        self.open_undo_tree();
    }

    pub(super) fn refresh_undo_tree_for_buffer(&mut self, source_buffer_id: BufferId) {
        let Some(tree) = self.undo_tree.as_ref() else {
            return;
        };
        if tree.source_buffer_id != source_buffer_id {
            return;
        }
        self.refresh_undo_tree_surface(true);
    }

    pub(super) fn undo_tree_open_selected(&mut self, viewport_width_cells: usize, text_vh: usize) {
        let Some(tree) = self.undo_tree.clone() else {
            return;
        };
        let target = tree.selected_node;
        let source_buffer_id = tree.source_buffer_id;
        if !self
            .panes
            .iter()
            .any(|pane| pane.buffer_id == source_buffer_id)
        {
            self.set_status("undo tree source is not visible");
            return;
        }

        let cursor = {
            let Some(buffer) = self.session.buffer_mut(source_buffer_id) else {
                self.set_status("undo tree source is not available");
                return;
            };
            let view = self.views.entry(source_buffer_id).or_default();
            view.undo_history.restore(buffer, target)
        };
        let Some(cursor) = cursor else {
            self.set_status("undo tree node unavailable");
            let _ = self.activate_pane(tree.pane_id);
            return;
        };

        {
            let view = self.views.entry(source_buffer_id).or_default();
            let Some(buffer) = self.session.buffer(source_buffer_id) else {
                self.set_status("undo tree source is not available");
                return;
            };
            view.cursor.cursor = buffer.clamp_pos(cursor);
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
            view.visual_anchor = None;
        }
        self.mode = EditorMode::Normal;
        let _ = self.session.recompute_buffer_dirty(source_buffer_id);
        self.invalidate_buffer_render_caches(source_buffer_id);
        self.refresh_undo_tree_surface(false);
        let _ = self.activate_pane(tree.pane_id);
        self.clear_status();
    }

    pub(super) fn clamp_undo_tree_cursor(&mut self) {
        let Some(tree) = self.undo_tree.as_mut() else {
            return;
        };
        if tree.buffer_id != self.session.active_id() {
            return;
        }
        let row_count = tree.display_rows.len().max(1);
        let view = self.views.entry(tree.buffer_id).or_default();
        let line = view.cursor.cursor.line.min(row_count.saturating_sub(1));
        let selected = undo_tree_node_for_line(&tree.display_rows, line);
        if let Some((line, node)) = selected {
            view.cursor.cursor = Pos::new(line, 0);
            tree.selected_node = node;
        }
        self.refresh_undo_tree_surface(false);
    }

    pub(super) fn apply_undo_tree_motion(&mut self, motion: Motion, count: usize) -> bool {
        let Some(tree) = self.undo_tree.as_ref() else {
            return false;
        };
        if tree.buffer_id != self.session.active_id() {
            return false;
        }

        match motion {
            Motion::Up => {
                self.move_undo_tree_selection_by_rows(-(count.max(1) as isize));
                true
            }
            Motion::Down => {
                self.move_undo_tree_selection_by_rows(count.max(1) as isize);
                true
            }
            Motion::Left => {
                self.move_undo_tree_selection_to_parent(count.max(1));
                true
            }
            Motion::Right => {
                self.move_undo_tree_selection_to_newest_child(count.max(1));
                true
            }
            _ => false,
        }
    }

    pub fn undo_tree_surface_role(&self, buffer_id: BufferId) -> Option<UndoTreeSurfaceRole> {
        let tree = self.undo_tree.as_ref()?;
        if tree.buffer_id == buffer_id {
            Some(UndoTreeSurfaceRole::Tree)
        } else if tree.diff_buffer_id == buffer_id {
            Some(UndoTreeSurfaceRole::Preview)
        } else {
            None
        }
    }

    pub fn pane_draws_as_active(&self, pane_id: PaneId) -> bool {
        if pane_id == self.active_pane {
            return true;
        }
        let Some(tree) = self.undo_tree.as_ref() else {
            return false;
        };
        pane_id == tree.diff_pane_id && self.active_pane == tree.pane_id
    }

    fn open_undo_tree(&mut self) {
        if self.active_buffer_is_surface() {
            self.set_status("undo tree opens from an editor buffer");
            return;
        }
        let source_buffer_id = self.session.active_id();
        let undo_tree_style = UndoTreeStyle::default();
        let source_pane_id = self.active_pane;
        self.sync_active_pane_view();
        let Some(editor_pane_id) = self.split_active_pane_with_options(
            SplitAxis::Vertical,
            PaneOptions::editor(),
            SplitSize::first_percent(
                undo_tree_style.width_percent,
                undo_tree_style.min_width,
                undo_tree_style.max_width,
            ),
        ) else {
            return;
        };
        let surface_id = self.session.open_ui_buffer("undo-tree", "");
        let pane_id = source_pane_id;

        if let Some(pane) = self.panes.iter_mut().find(|pane| pane.id == pane_id) {
            pane.buffer_id = surface_id;
            pane.view = BufferViewState::default();
            pane.options = PaneOptions::ui();
        }
        self.views.entry(surface_id).or_default();
        let _ = self.activate_pane(pane_id);
        let Some(diff_pane_id) = self.split_active_pane_with_options(
            SplitAxis::Horizontal,
            PaneOptions {
                accessible: false,
                ..PaneOptions::ui()
            },
            SplitSize::second_percent(
                undo_tree_style.preview_height_percent,
                undo_tree_style.preview_min_height,
                undo_tree_style.preview_max_height,
            ),
        ) else {
            let _ = self.activate_pane(pane_id);
            self.close_active_split();
            let _ = self.session.close_buffer(surface_id);
            self.views.remove(&surface_id);
            return;
        };
        let diff_buffer_id = self.session.open_ui_buffer("undo-tree-diff", "");
        if let Some(pane) = self.panes.iter_mut().find(|pane| pane.id == diff_pane_id) {
            pane.buffer_id = diff_buffer_id;
            pane.view = BufferViewState::default();
            pane.options = PaneOptions {
                accessible: false,
                ..PaneOptions::ui()
            };
        }
        self.views.entry(diff_buffer_id).or_default();
        self.undo_tree = Some(UndoTreeState {
            buffer_id: surface_id,
            diff_buffer_id,
            source_buffer_id,
            pane_id,
            diff_pane_id,
            selected_node: self
                .views
                .get(&source_buffer_id)
                .map(|view| view.undo_history.current())
                .unwrap_or(0),
            display_rows: Vec::new(),
            rendered_at_ms: current_time_ms(),
        });
        self.refresh_undo_tree_surface(true);
        let _ = self.activate_pane(pane_id);
        let _ = self.activate_pane_with_recent(editor_pane_id, false);
        let _ = self.activate_pane(pane_id);
        self.mode = EditorMode::Normal;
        self.clear_status();
    }

    fn close_undo_tree_if_open(&mut self) -> bool {
        let Some(tree) = self.undo_tree.clone() else {
            return false;
        };
        self.close_undo_tree_panel(tree)
    }

    pub(super) fn close_undo_tree_panel(&mut self, tree: UndoTreeState) -> bool {
        let _ = self.activate_pane(tree.diff_pane_id);
        if self.panes.len() > 1 {
            self.close_active_split();
        }
        let _ = self.session.close_buffer(tree.diff_buffer_id);
        self.views.remove(&tree.diff_buffer_id);

        let _ = self.activate_pane(tree.pane_id);
        if self.panes.len() > 1 {
            self.close_active_split();
        }
        let _ = self.session.close_buffer(tree.buffer_id);
        self.views.remove(&tree.buffer_id);
        self.undo_tree = None;
        self.clear_status();
        true
    }

    fn refresh_undo_tree_surface(&mut self, update_clock: bool) {
        let Some(tree) = self.undo_tree.as_ref().cloned() else {
            return;
        };
        let Some(source_view) = self.views.get(&tree.source_buffer_id) else {
            return;
        };
        let rendered_at_ms = if update_clock {
            current_time_ms()
        } else {
            tree.rendered_at_ms
        };
        let entries = source_view.undo_history.tree_entries();
        let rendered = render_undo_tree(&entries, rendered_at_ms);
        let selected_node = if update_clock {
            source_view.undo_history.current()
        } else if rendered
            .display_rows
            .iter()
            .any(|node| *node == Some(tree.selected_node))
        {
            tree.selected_node
        } else {
            source_view.undo_history.current()
        };
        let preview_change = self
            .session
            .buffer(tree.source_buffer_id)
            .and_then(|buffer| {
                undo_tree_preview_change(buffer, &source_view.undo_history, selected_node)
            });
        let diff_text = undo_tree_diff_text(selected_node, preview_change.as_ref());

        if let Some(buffer) = self.session.buffer_mut(tree.buffer_id) {
            *buffer = TextBuffer::from_str(&rendered.text);
        }
        if let Some(buffer) = self.session.buffer_mut(tree.diff_buffer_id) {
            *buffer = TextBuffer::from_str(&diff_text);
        }
        if let Some(tree) = self.undo_tree.as_mut() {
            tree.display_rows = rendered.display_rows;
            tree.selected_node = selected_node;
            tree.rendered_at_ms = rendered_at_ms;
        }
        let selected_row = self
            .undo_tree
            .as_ref()
            .and_then(|tree| {
                tree.display_rows
                    .iter()
                    .position(|node| *node == Some(selected_node))
            })
            .unwrap_or(0);
        let scroll_y = undo_tree_scroll_top_for_row(
            self.views
                .get(&tree.buffer_id)
                .map(|view| view.cursor.scroll_y_lines)
                .unwrap_or(0),
            selected_row,
            self.undo_tree_pane_height(tree.pane_id),
            self.undo_tree
                .as_ref()
                .map(|tree| tree.display_rows.len())
                .unwrap_or(0),
        );
        let cursor = {
            let view = self.views.entry(tree.buffer_id).or_default();
            view.cursor.cursor = Pos::new(selected_row, 0);
            view.cursor.scroll_y_lines = scroll_y;
            view.cursor.clone()
        };
        if let Some(pane) = self.panes.iter_mut().find(|pane| pane.id == tree.pane_id) {
            pane.view.cursor = cursor;
        }
    }

    fn undo_tree_pane_height(&self, pane_id: PaneId) -> usize {
        self.pane_rects(
            self.editor_area_width_cells as u16,
            self.editor_area_height_rows as u16,
        )
        .into_iter()
        .find(|rect| rect.pane_id == pane_id)
        .map(|rect| rect.height as usize)
        .unwrap_or(self.editor_area_height_rows)
    }

    fn move_undo_tree_selection_by_rows(&mut self, delta: isize) {
        let Some(tree) = self.undo_tree.as_ref() else {
            return;
        };
        let selectable_rows = tree
            .display_rows
            .iter()
            .enumerate()
            .filter_map(|(row, node)| node.map(|node| (row, node)))
            .collect::<Vec<_>>();
        if selectable_rows.is_empty() {
            return;
        }
        let current_index = selectable_rows
            .iter()
            .position(|(_, node)| *node == tree.selected_node)
            .unwrap_or(0);
        let target_index = current_index
            .saturating_add_signed(delta)
            .min(selectable_rows.len().saturating_sub(1));
        self.select_undo_tree_node(selectable_rows[target_index].1);
    }

    fn move_undo_tree_selection_to_parent(&mut self, count: usize) {
        for _ in 0..count {
            let Some(target) = self.undo_tree_parent_of_selected() else {
                break;
            };
            self.select_undo_tree_node(target);
        }
    }

    fn move_undo_tree_selection_to_newest_child(&mut self, count: usize) {
        for _ in 0..count {
            let Some(target) = self.undo_tree_newest_child_of_selected() else {
                break;
            };
            self.select_undo_tree_node(target);
        }
    }

    fn undo_tree_parent_of_selected(&self) -> Option<UndoNodeId> {
        let tree = self.undo_tree.as_ref()?;
        let source_view = self.views.get(&tree.source_buffer_id)?;
        source_view
            .undo_history
            .tree_entries()
            .into_iter()
            .find(|entry| entry.id == tree.selected_node)?
            .parent
    }

    fn undo_tree_newest_child_of_selected(&self) -> Option<UndoNodeId> {
        let tree = self.undo_tree.as_ref()?;
        let source_view = self.views.get(&tree.source_buffer_id)?;
        source_view
            .undo_history
            .tree_entries()
            .into_iter()
            .filter(|entry| entry.parent == Some(tree.selected_node))
            .max_by_key(|entry| entry.sequence)
            .map(|entry| entry.id)
    }

    fn select_undo_tree_node(&mut self, node: UndoNodeId) {
        let Some(tree) = self.undo_tree.as_mut() else {
            return;
        };
        tree.selected_node = node;
        self.refresh_undo_tree_surface(false);
    }
}

fn undo_tree_scroll_top_for_row(
    current_top: usize,
    selected_row: usize,
    viewport_height: usize,
    row_count: usize,
) -> usize {
    if viewport_height == 0 || row_count == 0 {
        return current_top;
    }
    let max_top = row_count.saturating_sub(viewport_height);
    let mut top = current_top.min(max_top);
    if selected_row < top {
        top = selected_row;
    } else if selected_row >= top.saturating_add(viewport_height) {
        top = selected_row.saturating_sub(viewport_height.saturating_sub(1));
    }
    top.min(max_top)
}

#[derive(Debug, Clone)]
struct RenderedUndoTree {
    text: String,
    display_rows: Vec<Option<UndoNodeId>>,
}

#[derive(Debug, Clone)]
struct RenderTreeNode {
    entry: UndoTreeEntry,
    children: Vec<RenderTreeNode>,
}

#[derive(Debug, Clone)]
struct UndoTreePreviewChange {
    before: TextBuffer,
    after: TextBuffer,
    diff: TextDiff,
}

const UNDO_TREE_NODE_GLYPH: char = '●';
const UNDO_TREE_VERTICAL_GLYPH: char = '│';
const UNDO_TREE_HORIZONTAL_GLYPH: char = '─';
const UNDO_TREE_SLOT_SPACING: usize = 1;

#[derive(Debug, Clone)]
struct RenderGraphRow {
    entry: Option<UndoTreeEntry>,
}

#[derive(Debug, Clone, Copy, Default)]
struct RenderGraphCell {
    node: bool,
    up: bool,
    down: bool,
    left: bool,
    right: bool,
}

fn render_undo_tree(entries: &[UndoTreeEntry], rendered_at_ms: u128) -> RenderedUndoTree {
    let Some(mut root) = undo_tree_render_root(entries) else {
        return RenderedUndoTree {
            text: String::new(),
            display_rows: Vec::new(),
        };
    };
    order_undo_tree_render_node(&mut root);

    let mut columns = HashMap::new();
    let column_count = assign_undo_tree_columns(&root, 0, &mut columns).max(1);

    let rows = undo_tree_graph_rows(entries);
    let redo_target = undo_tree_redo_target(entries);
    let label_width = undo_tree_label_width(entries);

    let mut node_rows = HashMap::new();
    for (row_index, row) in rows.iter().enumerate() {
        if let Some(entry) = &row.entry {
            node_rows.insert(entry.id, row_index);
        }
    }

    let mut graph = vec![
        vec![RenderGraphCell::default(); undo_tree_graph_cell_width(column_count)];
        rows.len()
    ];
    draw_undo_tree_edges(&root, &columns, &node_rows, &mut graph);
    draw_undo_tree_nodes(&rows, &columns, &mut graph);

    let graph_rows = graph
        .into_iter()
        .map(undo_tree_graph_row_to_string)
        .collect::<Vec<_>>();
    let tree_width = graph_rows
        .iter()
        .map(|row| undo_tree_graph_width(row))
        .max()
        .unwrap_or(0);
    let mut lines = Vec::with_capacity(rows.len());
    let mut display_rows = Vec::with_capacity(rows.len());
    for (row, graph_row) in rows.into_iter().zip(graph_rows) {
        match row.entry {
            Some(entry) => {
                let node_id = entry.id;
                lines.push(undo_tree_node_line(
                    &entry,
                    rendered_at_ms,
                    &graph_row,
                    tree_width,
                    redo_target,
                    label_width,
                ));
                display_rows.push(Some(node_id));
            }
            None => {
                lines.push(format_undo_tree_connector_line(&graph_row));
                display_rows.push(None);
            }
        }
    }

    RenderedUndoTree {
        text: lines.join("\n") + if lines.is_empty() { "" } else { "\n" },
        display_rows,
    }
}

fn undo_tree_render_root(entries: &[UndoTreeEntry]) -> Option<RenderTreeNode> {
    let entries_by_id = entries
        .iter()
        .map(|entry| (entry.id, entry.clone()))
        .collect::<HashMap<_, _>>();
    let mut children_by_parent: HashMap<UndoNodeId, Vec<UndoNodeId>> = HashMap::new();
    for entry in entries {
        if let Some(parent) = entry.parent {
            children_by_parent.entry(parent).or_default().push(entry.id);
        }
    }
    for children in children_by_parent.values_mut() {
        children.sort_by_key(|id| {
            entries_by_id
                .get(id)
                .map(|entry| entry.sequence)
                .unwrap_or(0)
        });
    }

    let root_id = entries.iter().find(|entry| entry.parent.is_none())?.id;
    build_undo_render_node(root_id, &entries_by_id, &children_by_parent)
}

fn build_undo_render_node(
    id: UndoNodeId,
    entries_by_id: &HashMap<UndoNodeId, UndoTreeEntry>,
    children_by_parent: &HashMap<UndoNodeId, Vec<UndoNodeId>>,
) -> Option<RenderTreeNode> {
    let entry = entries_by_id.get(&id)?.clone();
    let children = children_by_parent
        .get(&id)
        .into_iter()
        .flat_map(|children| children.iter())
        .filter_map(|child| build_undo_render_node(*child, entries_by_id, children_by_parent))
        .collect();
    Some(RenderTreeNode { entry, children })
}

fn order_undo_tree_branch_children(branch: &mut [RenderTreeNode]) {
    branch.sort_by_key(|node| std::cmp::Reverse(node.entry.sequence));
}

fn order_undo_tree_render_node(node: &mut RenderTreeNode) {
    for child in &mut node.children {
        order_undo_tree_render_node(child);
    }
    if node.children.len() > 1 {
        order_undo_tree_branch_children(&mut node.children);
    }
}

fn assign_undo_tree_columns(
    node: &RenderTreeNode,
    next_column: usize,
    columns: &mut HashMap<UndoNodeId, usize>,
) -> usize {
    if node.children.is_empty() {
        columns.insert(node.entry.id, next_column);
        return next_column.saturating_add(1);
    }

    let node_column = next_column;
    let mut next_column = next_column;
    for child in &node.children {
        next_column = assign_undo_tree_columns(child, next_column, columns);
    }
    columns.insert(node.entry.id, node_column);
    next_column
}

fn undo_tree_graph_rows(entries: &[UndoTreeEntry]) -> Vec<RenderGraphRow> {
    let mut entries = entries.to_vec();
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.sequence));

    let mut rows = Vec::with_capacity(entries.len());
    for entry in entries {
        if entry.child_count > 1 {
            rows.push(RenderGraphRow { entry: None });
        }
        rows.push(RenderGraphRow { entry: Some(entry) });
    }
    rows
}

fn undo_tree_redo_target(entries: &[UndoTreeEntry]) -> Option<UndoNodeId> {
    let current = entries.iter().find(|entry| entry.is_current)?.id;
    entries
        .iter()
        .filter(|entry| entry.parent == Some(current))
        .max_by_key(|entry| entry.sequence)
        .map(|entry| entry.id)
}

fn undo_tree_label_width(entries: &[UndoTreeEntry]) -> usize {
    entries
        .iter()
        .filter(|entry| entry.id != 0)
        .map(|entry| entry.sequence.to_string().len())
        .max()
        .unwrap_or(1)
}

fn draw_undo_tree_edges(
    node: &RenderTreeNode,
    columns: &HashMap<UndoNodeId, usize>,
    node_rows: &HashMap<UndoNodeId, usize>,
    graph: &mut [Vec<RenderGraphCell>],
) {
    for child in &node.children {
        draw_undo_tree_edge(node.entry.id, child.entry.id, columns, node_rows, graph);
        draw_undo_tree_edges(child, columns, node_rows, graph);
    }
}

fn draw_undo_tree_edge(
    parent: UndoNodeId,
    child: UndoNodeId,
    columns: &HashMap<UndoNodeId, usize>,
    node_rows: &HashMap<UndoNodeId, usize>,
    graph: &mut [Vec<RenderGraphCell>],
) {
    let (Some(parent_row), Some(child_row), Some(parent_column), Some(child_column)) = (
        node_rows.get(&parent).copied(),
        node_rows.get(&child).copied(),
        columns.get(&parent).copied(),
        columns.get(&child).copied(),
    ) else {
        return;
    };
    if child_row >= parent_row {
        return;
    }

    let parent_col = undo_tree_slot_col(parent_column);
    let child_col = undo_tree_slot_col(child_column);
    if parent_col == child_col {
        for row in child_row.saturating_add(1)..parent_row {
            add_undo_tree_vertical(graph, row, child_col);
        }
        return;
    }

    let connector_row = parent_row.saturating_sub(1);
    for row in child_row.saturating_add(1)..connector_row {
        add_undo_tree_vertical(graph, row, child_col);
    }
    add_undo_tree_branch(graph, connector_row, parent_col, child_col);
}

fn draw_undo_tree_nodes(
    rows: &[RenderGraphRow],
    columns: &HashMap<UndoNodeId, usize>,
    graph: &mut [Vec<RenderGraphCell>],
) {
    for (row_index, row) in rows.iter().enumerate() {
        let Some(entry) = &row.entry else {
            continue;
        };
        let Some(column) = columns.get(&entry.id).copied() else {
            continue;
        };
        if let Some(cell) = graph
            .get_mut(row_index)
            .and_then(|row| row.get_mut(undo_tree_slot_col(column)))
        {
            cell.node = true;
        }
    }
}

fn add_undo_tree_vertical(graph: &mut [Vec<RenderGraphCell>], row: usize, col: usize) {
    if let Some(cell) = graph.get_mut(row).and_then(|row| row.get_mut(col)) {
        cell.up = true;
        cell.down = true;
    }
}

fn add_undo_tree_branch(
    graph: &mut [Vec<RenderGraphCell>],
    row: usize,
    parent_col: usize,
    child_col: usize,
) {
    let (start, end) = if parent_col <= child_col {
        (parent_col, child_col)
    } else {
        (child_col, parent_col)
    };
    for col in start..=end {
        let Some(cell) = graph.get_mut(row).and_then(|row| row.get_mut(col)) else {
            continue;
        };
        if col > start {
            cell.left = true;
        }
        if col < end {
            cell.right = true;
        }
        if col == parent_col {
            cell.up = true;
            cell.down = true;
        }
        if col == child_col {
            cell.up = true;
        }
    }
}

fn undo_tree_graph_cell_width(column_count: usize) -> usize {
    undo_tree_slot_col(column_count.saturating_sub(1)).saturating_add(1)
}

fn undo_tree_slot_col(slot_index: usize) -> usize {
    slot_index.saturating_mul(UNDO_TREE_SLOT_SPACING)
}

fn undo_tree_graph_row_to_string(row: Vec<RenderGraphCell>) -> String {
    row.into_iter()
        .map(undo_tree_graph_cell_to_char)
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn undo_tree_graph_cell_to_char(cell: RenderGraphCell) -> char {
    if cell.node {
        return UNDO_TREE_NODE_GLYPH;
    }

    match (cell.up, cell.down, cell.left, cell.right) {
        (false, false, false, false) => ' ',
        (_, _, false, false) => UNDO_TREE_VERTICAL_GLYPH,
        (false, false, _, _) => UNDO_TREE_HORIZONTAL_GLYPH,
        (false, true, false, true) => '┌',
        (true, false, true, false) => '┘',
        (true, false, false, true) => '└',
        (false, true, true, false) => '┐',
        (true, true, false, true) => '├',
        (true, true, true, false) => '┤',
        (true, false, true, true) => '┴',
        (false, true, true, true) => '┬',
        (true, true, true, true) => '┼',
    }
}

fn undo_tree_graph_width(tree: &str) -> usize {
    tree.trim_end().chars().count()
}

fn undo_tree_node_line(
    entry: &UndoTreeEntry,
    rendered_at_ms: u128,
    tree: &str,
    tree_width: usize,
    redo_target: Option<UndoNodeId>,
    label_width: usize,
) -> String {
    let label = undo_tree_node_label(entry, redo_target, label_width);
    let time = if entry.id == 0 {
        None
    } else {
        Some(relative_time_at(entry.created_at_ms, rendered_at_ms))
    };
    let tree = tree.trim_end();
    let label_gap = tree_width
        .saturating_sub(undo_tree_graph_width(tree))
        .saturating_add(if label.is_marked { 1 } else { 2 });
    let time = time
        .map(|time| {
            if label.is_marked {
                format!(" {time}")
            } else {
                format!("  {time}")
            }
        })
        .unwrap_or_default();
    format!("  {tree}{}{}{time}", " ".repeat(label_gap), label.text)
}

struct UndoTreeNodeLabel {
    text: String,
    is_marked: bool,
}

fn undo_tree_node_label(
    entry: &UndoTreeEntry,
    redo_target: Option<UndoNodeId>,
    label_width: usize,
) -> UndoTreeNodeLabel {
    if entry.id == 0 {
        return if entry.is_current {
            UndoTreeNodeLabel {
                text: ">original<".to_string(),
                is_marked: true,
            }
        } else {
            UndoTreeNodeLabel {
                text: "original".to_string(),
                is_marked: false,
            }
        };
    }

    let number = entry.sequence.to_string();
    if entry.is_current {
        UndoTreeNodeLabel {
            text: undo_tree_marked_node_label(&number, label_width, '>', '<'),
            is_marked: true,
        }
    } else if redo_target == Some(entry.id) {
        UndoTreeNodeLabel {
            text: undo_tree_marked_node_label(&number, label_width, '{', '}'),
            is_marked: true,
        }
    } else {
        UndoTreeNodeLabel {
            text: format!("{number:>label_width$}"),
            is_marked: false,
        }
    }
}

fn undo_tree_marked_node_label(
    number: &str,
    label_width: usize,
    left_marker: char,
    right_marker: char,
) -> String {
    let padding = " ".repeat(label_width.saturating_sub(number.len()));
    format!("{padding}{left_marker}{number}{right_marker}")
}

fn format_undo_tree_connector_line(tree: &str) -> String {
    format!("  {}", tree.trim_end())
}

#[cfg(test)]
mod render_tests {
    use super::*;
    use redox_core::Edit;

    #[test]
    fn slot_renderer_keeps_newest_first_rows_with_connected_branch() {
        let entries = vec![
            test_entry(0, None, 0, false, 1),
            test_entry(1, Some(0), 1, false, 1),
            test_entry(2, Some(1), 2, false, 2),
            test_entry(3, Some(2), 3, false, 0),
            test_entry(4, Some(2), 4, true, 0),
        ];

        let rendered = render_undo_tree(&entries, 1_000);
        let lines = rendered.text.lines().collect::<Vec<_>>();

        assert_eq!(
            rendered.display_rows,
            vec![Some(4), Some(3), None, Some(2), Some(1), Some(0)]
        );
        assert!(lines[0].contains(">4<"), "{lines:?}");
        assert!(lines[2].contains("├┘"), "{lines:?}");
        assert!(lines.iter().all(|line| !line.contains('\\')), "{lines:?}");
        assert!(lines.iter().all(|line| !line.contains('/')), "{lines:?}");
    }

    #[test]
    fn slot_renderer_marks_redo_target_and_aligns_wide_labels() {
        let entries = (0..=11)
            .map(|id| {
                test_entry(
                    id,
                    if id == 0 { None } else { Some(id - 1) },
                    id as u64,
                    id == 8,
                    if id == 11 { 0 } else { 1 },
                )
            })
            .collect::<Vec<_>>();

        let rendered = render_undo_tree(&entries, 1_000);
        let lines = rendered.text.lines().collect::<Vec<_>>();

        assert!(lines[0].contains("11"), "{lines:?}");
        assert!(lines[1].contains("10"), "{lines:?}");
        assert!(lines.iter().any(|line| line.contains(" {9} ")), "{lines:?}");
        assert!(lines.iter().any(|line| line.contains(" >8< ")), "{lines:?}");
        assert!(lines.iter().all(|line| !line.contains("{ 9}")), "{lines:?}");
        assert!(lines.iter().all(|line| !line.contains("> 8<")), "{lines:?}");

        let timestamp_col = char_position(lines[0], '(');
        assert!(
            lines
                .iter()
                .take(4)
                .all(|line| char_position(line, '(') == timestamp_col)
        );
    }

    #[test]
    fn original_preview_separates_title_from_empty_message() {
        assert_eq!(
            undo_tree_diff_text(0, None),
            "Original state\n\nNo edit is recorded for this point.\n"
        );
    }

    #[test]
    fn diff_preview_trims_shared_indent_and_expands_changed_line() {
        let before = TextBuffer::from_str("        word");
        let after = TextBuffer::from_str("        wordasdf");
        let diff = TextDiff::between(&before, &after).expect("missing diff");
        let (deleted, inserted) = diff_preview_lines(&before, &after, &diff);

        assert_eq!(deleted, vec!["word"]);
        assert_eq!(inserted, vec!["wordasdf"]);
    }

    #[test]
    fn undo_tree_preview_coalesces_delete_then_insert_at_same_position() {
        let mut history = UndoHistory::default();
        let mut buffer = TextBuffer::from_str("word old tail");

        let checkpoint = history.checkpoint(buffer.clone(), Pos::new(0, 5));
        let _ = buffer.apply_edit(Edit::delete(5..8));
        assert!(history.record_if_changed(checkpoint, &buffer, Pos::new(0, 5)));

        let checkpoint = history.checkpoint(buffer.clone(), Pos::new(0, 5));
        let _ = buffer.apply_edit(Edit::insert(5, "new"));
        assert!(history.record_if_changed(checkpoint, &buffer, Pos::new(0, 8)));

        let preview = undo_tree_preview_change(&buffer, &history, history.current())
            .expect("missing preview");
        let (deleted, inserted) =
            diff_preview_lines(&preview.before, &preview.after, &preview.diff);

        assert_eq!(deleted, vec!["word old tail"]);
        assert_eq!(inserted, vec!["word new tail"]);
    }

    fn test_entry(
        id: UndoNodeId,
        parent: Option<UndoNodeId>,
        sequence: u64,
        is_current: bool,
        child_count: usize,
    ) -> UndoTreeEntry {
        UndoTreeEntry {
            id,
            parent,
            sequence,
            created_at_ms: 0,
            is_current,
            child_count,
        }
    }

    fn char_position(line: &str, needle: char) -> Option<usize> {
        line.chars().position(|ch| ch == needle)
    }
}

fn undo_tree_preview_change(
    current_buffer: &TextBuffer,
    history: &UndoHistory,
    selected_node: UndoNodeId,
) -> Option<UndoTreePreviewChange> {
    let entries = history.tree_entries();
    let entry = undo_tree_entry(&entries, selected_node)?;
    let record = history.record_for_node(selected_node)?;
    let (before_node, diff) = coalesced_preview_diff(&entries, history, entry, record)
        .unwrap_or_else(|| {
            (
                entry.parent.unwrap_or(0),
                TextDiff {
                    start_char: record.diff.start_char,
                    deleted: record.diff.deleted.clone(),
                    inserted: record.diff.inserted.clone(),
                },
            )
        });
    Some(UndoTreePreviewChange {
        before: materialize_undo_node(current_buffer, history, before_node)?,
        after: materialize_undo_node(current_buffer, history, selected_node)?,
        diff,
    })
}

fn coalesced_preview_diff(
    entries: &[UndoTreeEntry],
    history: &UndoHistory,
    entry: &UndoTreeEntry,
    record: &UndoRecord,
) -> Option<(UndoNodeId, TextDiff)> {
    let parent_id = entry.parent?;
    let parent = undo_tree_entry(entries, parent_id)?;
    let grandparent_id = parent.parent?;
    let parent_record = history.record_for_node(parent_id)?;
    if !parent_record.diff.inserted.is_empty()
        || parent_record.diff.deleted.is_empty()
        || !record.diff.deleted.is_empty()
        || record.diff.inserted.is_empty()
        || parent_record.diff.start_char != record.diff.start_char
    {
        return None;
    }

    Some((
        grandparent_id,
        TextDiff {
            start_char: parent_record.diff.start_char,
            deleted: parent_record.diff.deleted.clone(),
            inserted: record.diff.inserted.clone(),
        },
    ))
}

fn undo_tree_entry(entries: &[UndoTreeEntry], node_id: UndoNodeId) -> Option<&UndoTreeEntry> {
    entries.iter().find(|entry| entry.id == node_id)
}

fn materialize_undo_node(
    current_buffer: &TextBuffer,
    history: &UndoHistory,
    node_id: UndoNodeId,
) -> Option<TextBuffer> {
    let mut history = history.clone();
    let mut buffer = current_buffer.clone();
    history.restore(&mut buffer, node_id)?;
    Some(buffer)
}

fn undo_tree_diff_text(
    selected_node: UndoNodeId,
    preview_change: Option<&UndoTreePreviewChange>,
) -> String {
    let mut text = String::new();
    if let Some(change) = preview_change {
        let (deleted_lines, inserted_lines) =
            diff_preview_lines(&change.before, &change.after, &change.diff);
        text.push_str(&format!("Node {selected_node}\n\n"));
        push_preview_lines(&mut text, &deleted_lines);
        text.push_str("---\n");
        push_preview_lines(&mut text, &inserted_lines);
    } else {
        text.push_str("Original state\n\nNo edit is recorded for this point.\n");
    }
    text
}

fn diff_preview_lines(
    before: &TextBuffer,
    after: &TextBuffer,
    diff: &TextDiff,
) -> (Vec<String>, Vec<String>) {
    let mut deleted_lines =
        preview_lines_for_buffer(before, diff.start_char, diff.deleted.chars().count());
    let mut inserted_lines =
        preview_lines_for_buffer(after, diff.start_char, diff.inserted.chars().count());
    let trim = matching_preview_indent(&deleted_lines, &inserted_lines).unwrap_or(0);
    if trim > 0 {
        strip_preview_indent(&mut deleted_lines, trim);
        strip_preview_indent(&mut inserted_lines, trim);
    }
    (
        empty_preview_lines(deleted_lines),
        empty_preview_lines(inserted_lines),
    )
}

fn preview_lines_for_buffer(
    buffer: &TextBuffer,
    start_char: usize,
    changed_len: usize,
) -> Vec<String> {
    let (start_line, end_line) = preview_line_span(buffer, start_char, changed_len);
    (start_line..=end_line)
        .map(|line| buffer.line_string(line))
        .collect()
}

fn empty_preview_lines(lines: Vec<String>) -> Vec<String> {
    if lines.is_empty() || lines.iter().all(|line| line.is_empty()) {
        vec!["<empty>".to_string()]
    } else {
        lines
    }
}

fn preview_line_span(buffer: &TextBuffer, start_char: usize, changed_len: usize) -> (usize, usize) {
    let max_chars = buffer.len_chars();
    let start = start_char.min(max_chars);
    let end = start.saturating_add(changed_len).min(max_chars);
    let start_line = buffer.char_to_line(start);
    let end_char = if end > start {
        end.saturating_sub(1)
    } else {
        start
    };
    let end_line = buffer.char_to_line(end_char);
    (start_line.min(end_line), start_line.max(end_line))
}

fn matching_preview_indent(deleted_lines: &[String], inserted_lines: &[String]) -> Option<usize> {
    let deleted_indent = minimum_preview_indent(deleted_lines);
    let inserted_indent = minimum_preview_indent(inserted_lines);
    match (deleted_indent, inserted_indent) {
        (Some(deleted), Some(inserted)) if deleted == inserted => Some(deleted),
        (Some(deleted), None) => Some(deleted),
        (None, Some(inserted)) => Some(inserted),
        _ => None,
    }
}

fn minimum_preview_indent(lines: &[String]) -> Option<usize> {
    lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            line.chars()
                .take_while(|ch| matches!(ch, ' ' | '\t'))
                .count()
        })
        .min()
}

fn strip_preview_indent(lines: &mut [String], indent: usize) {
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        *line = strip_leading_preview_indent(line, indent);
    }
}

fn strip_leading_preview_indent(line: &str, indent: usize) -> String {
    let mut skipped = 0usize;
    for (byte_index, ch) in line.char_indices() {
        if skipped < indent && matches!(ch, ' ' | '\t') {
            skipped += 1;
            continue;
        }
        return line[byte_index..].to_string();
    }
    String::new()
}

fn push_preview_lines(text: &mut String, lines: &[String]) {
    for line in lines {
        text.push_str(line);
        text.push('\n');
    }
}

fn undo_tree_node_for_line(
    display_rows: &[Option<UndoNodeId>],
    line: usize,
) -> Option<(usize, UndoNodeId)> {
    if let Some(Some(node)) = display_rows.get(line) {
        return Some((line, *node));
    }
    for next in line.saturating_add(1)..display_rows.len() {
        if let Some(node) = display_rows[next] {
            return Some((next, node));
        }
    }
    (0..line.min(display_rows.len()))
        .rev()
        .find_map(|previous| display_rows[previous].map(|node| (previous, node)))
}

fn relative_time_at(created_at_ms: u128, rendered_at_ms: u128) -> String {
    let elapsed_seconds = rendered_at_ms.saturating_sub(created_at_ms) / 1000;
    match elapsed_seconds {
        0 | 1 => "(1 second ago)".to_string(),
        2..=59 => format!("({elapsed_seconds} seconds ago)"),
        60..=119 => "(1 minute ago)".to_string(),
        120..=3599 => format!("({} minutes ago)", elapsed_seconds / 60),
        3600..=7199 => "(1 hour ago)".to_string(),
        7200..=86_399 => format!("({} hours ago)", elapsed_seconds / 3600),
        _ => format!("({} days ago)", elapsed_seconds / 86_400),
    }
}

fn current_time_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}
