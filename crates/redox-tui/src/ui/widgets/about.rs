use minui::Window;
use minui::widgets::Widget;

use crate::app::AboutPopup;
use crate::ui::widgets::popup::{
    PopupChrome, clip_text_to_cells, draw_anchored_popup_frame, popup_inner_size,
    popup_window_view, wrap_text_to_cells,
};
use crate::ui::{UiStyle, build_editor_status_bar};

pub fn draw_about_popup_view(
    state: &mut crate::app::EditorState,
    style: UiStyle,
    window: &mut dyn Window,
    popup: AboutPopup,
) -> minui::Result<()> {
    let (vw, vh) = window.get_size();
    let (inner_w, inner_h) = about_popup_inner_size(vw, vh, style);
    let layout = draw_anchored_popup_frame(
        window,
        vw,
        vh,
        inner_w,
        inner_h,
        &popup.title,
        PopupChrome::about(style),
    )?;
    let mut view = popup_window_view(window, layout);

    let left = 2u16.min(inner_w.saturating_sub(1));
    let max_line_w = inner_w.saturating_sub(left);
    write_line(&mut view, 1, left, "┏━┓", style.about.logo_red, max_line_w)?;
    write_line(
        &mut view,
        2,
        left,
        "Redox",
        style.about.logo_white,
        max_line_w,
    )?;
    write_line(
        &mut view,
        2,
        left.saturating_add(7),
        &format!("v{}", popup.version),
        style.about.text,
        inner_w.saturating_sub(left.saturating_add(7)),
    )?;
    write_line(
        &mut view,
        3,
        left,
        "  ┗━┛",
        style.about.logo_blue,
        max_line_w,
    )?;

    let mut row = 5u16;
    row = write_wrapped_block(
        &mut view,
        row,
        left,
        max_line_w,
        &popup.message,
        style.about.text,
    )?;
    row = row.saturating_add(1);
    row = write_wrapped_block(
        &mut view,
        row,
        left,
        max_line_w,
        &format!("GitHub: {}", popup.repo_url),
        style.about.text,
    )?;
    let _ = write_wrapped_block(
        &mut view,
        row,
        left,
        max_line_w,
        &format!("Crates.io: {}", popup.crates_url),
        style.about.text,
    )?;

    let status = build_editor_status_bar(state, style);
    status.draw(window)?;

    Ok(())
}

pub fn about_popup_inner_size(term_w: u16, term_h: u16, style: UiStyle) -> (u16, u16) {
    popup_inner_size(
        term_w,
        term_h,
        style.about.width_percent,
        style.about.height_percent,
        style.about.min_width,
        style.about.min_height,
    )
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

fn write_wrapped_block(
    view: &mut minui::widgets::WindowView<'_>,
    start_row: u16,
    col: u16,
    width: u16,
    text: &str,
    color: minui::ColorPair,
) -> minui::Result<u16> {
    if width == 0 {
        return Ok(start_row);
    }

    let mut row = start_row;
    let wrapped = wrap_text_to_cells(text, width as usize);
    for line in wrapped {
        if row >= view.height {
            break;
        }
        view.write_str_colored(row, col, &line, color)?;
        row = row.saturating_add(1);
    }
    Ok(row)
}
