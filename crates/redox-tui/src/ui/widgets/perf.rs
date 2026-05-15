use minui::Window;

use crate::app::{FramePerfStats, PerfPopup};
use crate::ui::widgets::popup::{
    PopupChrome, anchored_popup_origin, clip_text_to_cells, draw_popup_frame_at, popup_inner_size,
    popup_window_view,
};
use crate::ui::{STATUS_BAR_HEIGHT_CELLS, UiStyle};

const PERF_FRAME_BUDGET_MS: f32 = 1_000.0 / 60.0;

struct PerfRow {
    label: &'static str,
    value_ms: f32,
    warn_ms: f32,
    hot_ms: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PerfPopupLayout {
    pub x: u16,
    pub y: u16,
    pub inner_w: u16,
    pub inner_h: u16,
}

pub fn draw_perf_popup_view(
    style: UiStyle,
    window: &mut dyn Window,
    popup: PerfPopup,
) -> minui::Result<()> {
    let (vw, vh) = window.get_size();
    let frame = perf_popup_layout(vw, vh, style);
    let inner_w = frame.inner_w;
    let inner_h = frame.inner_h;
    let layout = draw_popup_frame_at(
        window,
        frame.x,
        frame.y,
        inner_w,
        inner_h,
        "performance",
        PopupChrome {
            border: style.perf.border,
            title: style.perf.title,
            fill: style.perf.text,
        },
    )?;
    let mut view = popup_window_view(window, layout);

    if inner_w == 0 || inner_h == 0 {
        return Ok(());
    }

    let left = 2u16.min(inner_w.saturating_sub(1));
    let max_w = inner_w.saturating_sub(left);
    let Some(stats) = popup.stats else {
        if inner_h > 1 {
            write_line(
                &mut view,
                1,
                left,
                "collecting frame stats...",
                style.perf.text,
                max_w,
            )?;
        }
        return Ok(());
    };

    if inner_h > 1 {
        let budget = format!("budget {:>4.1} ms", PERF_FRAME_BUDGET_MS);
        write_line(&mut view, 1, left, &budget, style.perf.dim, max_w)?;
    }
    if inner_h > 3 {
        write_line(&mut view, 3, left, "metric", style.perf.label, max_w)?;
        write_line(
            &mut view,
            3,
            left.saturating_add(10),
            "avg",
            style.perf.label,
            max_w,
        )?;
    }

    let bar_col = left.saturating_add(19);
    let bar_w = inner_w.saturating_sub(bar_col).saturating_sub(2);
    for (idx, row) in perf_rows(stats).iter().enumerate() {
        let y = 4u16.saturating_add(idx as u16);
        if y >= inner_h {
            break;
        }

        write_line(&mut view, y, left, row.label, style.perf.label, 8)?;
        let value = format!("{:>5.1}", row.value_ms);
        write_line(
            &mut view,
            y,
            left.saturating_add(10),
            &value,
            style.perf.value,
            6,
        )?;
        draw_bar(&mut view, y, bar_col, bar_w, row, style)?;
    }

    let events_row = 4u16.saturating_add(perf_rows(stats).len() as u16);
    if events_row < inner_h {
        let events = format!("events {:>3}", stats.event_count);
        write_line(&mut view, events_row, left, &events, style.perf.dim, max_w)?;
    }

    Ok(())
}

pub fn perf_popup_inner_size(term_w: u16, term_h: u16, style: UiStyle) -> (u16, u16) {
    let max_h = term_h.saturating_sub(STATUS_BAR_HEIGHT_CELLS);
    popup_inner_size(
        term_w,
        max_h,
        style.perf.width_percent,
        style.perf.height_percent,
        style.perf.min_width,
        style.perf.min_height,
    )
}

pub fn perf_popup_layout(term_w: u16, term_h: u16, style: UiStyle) -> PerfPopupLayout {
    let (inner_w, inner_h) = perf_popup_inner_size(term_w, term_h, style);
    let (x, y) = anchored_popup_origin(term_w, term_h, inner_w, inner_h);
    PerfPopupLayout {
        x,
        y,
        inner_w,
        inner_h,
    }
}

pub fn perf_popup_occludes_cursor(layout: PerfPopupLayout, x: u16, y: u16) -> bool {
    let popup_w = layout.inner_w.saturating_add(2);
    let popup_h = layout.inner_h.saturating_add(2);
    let x_end = layout.x.saturating_add(popup_w);
    let y_end = layout.y.saturating_add(popup_h);
    x >= layout.x && x < x_end && y >= layout.y && y < y_end
}

fn perf_rows(stats: FramePerfStats) -> [PerfRow; 10] {
    [
        PerfRow {
            label: "frame",
            value_ms: stats.frame_ms,
            warn_ms: PERF_FRAME_BUDGET_MS * 0.5,
            hot_ms: PERF_FRAME_BUDGET_MS * 0.85,
        },
        PerfRow {
            label: "flush",
            value_ms: stats.flush_ms,
            warn_ms: 1.0,
            hot_ms: 2.0,
        },
        PerfRow {
            label: "load",
            value_ms: stats.load_ms,
            warn_ms: 1.0,
            hot_ms: 3.0,
        },
        PerfRow {
            label: "snap",
            value_ms: stats.snapshot_ms,
            warn_ms: 1.0,
            hot_ms: 3.0,
        },
        PerfRow {
            label: "syn",
            value_ms: stats.syntax_ms,
            warn_ms: 1.0,
            hot_ms: 3.0,
        },
        PerfRow {
            label: "ovr",
            value_ms: stats.overlays_ms,
            warn_ms: 1.0,
            hot_ms: 3.0,
        },
        PerfRow {
            label: "line",
            value_ms: stats.lines_ms,
            warn_ms: 1.0,
            hot_ms: 3.0,
        },
        PerfRow {
            label: "stat",
            value_ms: stats.status_ms,
            warn_ms: 0.5,
            hot_ms: 1.0,
        },
        PerfRow {
            label: "input",
            value_ms: stats.input_ms,
            warn_ms: 0.5,
            hot_ms: 1.0,
        },
        PerfRow {
            label: "oth",
            value_ms: stats.other_ms,
            warn_ms: 1.0,
            hot_ms: 3.0,
        },
    ]
}

fn draw_bar(
    view: &mut minui::widgets::WindowView<'_>,
    row: u16,
    col: u16,
    width: u16,
    perf: &PerfRow,
    style: UiStyle,
) -> minui::Result<()> {
    if width == 0 {
        return Ok(());
    }

    let colour = if perf.value_ms >= perf.hot_ms {
        style.perf.hot
    } else if perf.value_ms >= perf.warn_ms {
        style.perf.warn
    } else {
        style.perf.good
    };
    let ratio = (perf.value_ms / perf.hot_ms.max(0.1)).clamp(0.0, 1.0);
    let filled = ((width as f32) * ratio).round() as usize;
    let width = width as usize;
    let filled = filled.min(width);

    view.write_str_colored(row, col, &".".repeat(width), style.perf.bar_bg)?;
    if filled > 0 {
        view.write_str_colored(row, col, &"#".repeat(filled), colour)?;
    }
    Ok(())
}

fn write_line(
    view: &mut minui::widgets::WindowView<'_>,
    row: u16,
    col: u16,
    text: &str,
    color: minui::ColorPair,
    width: u16,
) -> minui::Result<()> {
    if width == 0 {
        return Ok(());
    }

    let clipped = clip_text_to_cells(text, width as usize);
    view.write_str_colored(row, col, &clipped, color)
}

#[cfg(test)]
mod tests {
    use super::{PerfPopupLayout, perf_popup_occludes_cursor};

    #[test]
    fn perf_popup_occludes_cursor_inside_frame_bounds() {
        let layout = PerfPopupLayout {
            x: 20,
            y: 0,
            inner_w: 10,
            inner_h: 5,
        };

        assert!(perf_popup_occludes_cursor(layout, 20, 0));
        assert!(perf_popup_occludes_cursor(layout, 31, 6));
        assert!(!perf_popup_occludes_cursor(layout, 19, 0));
        assert!(!perf_popup_occludes_cursor(layout, 32, 6));
        assert!(!perf_popup_occludes_cursor(layout, 31, 7));
    }
}
