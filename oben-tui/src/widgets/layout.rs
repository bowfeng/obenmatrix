//! Layout calculation functions for the message conversation widget.
//!
//! Pure functions — no rendering, no Frame. Given input data and viewport width,
//! they produce layout info: heights, scroll positions, and visible Rect areas.
//!
//! This separation means:
//!   - Change padding/border/gap → only these functions change
//!   - Change scroll behavior → only scroll() changes
//!   - Change block rendering → only the render loop changes
//!   - No more "+2/-1" constant juggling scattered across a monolithic function
use crate::widgets::conversation::BlockType;
use crate::widgets::message_renderer::StyledLine;
use ratatui::prelude::*;
use textwrap::wrap as textwrap_wrap;
use unicode_width::UnicodeWidthStr;

// ── Layout constants (single source of truth) ──────────────────────────

/// Height of a block body in lines (border only, no title).
/// Used to compute body_area.height = actual_block_height - 2.
pub const BODY_HEIGHT_ADJUSTER: u16 = 2;

/// Extra vertical margin between consecutive message blocks.
pub const INTER_BLOCK_MARGIN: u16 = 1;

/// Indent (columns) for tool result boxes from the left edge.
pub const TOOL_INDENT: u16 = 2;

// ── Line wrapping ─────────────────────────────────────────────────────

/// Wrap styled lines to a given column width, returning wrapped line count.
pub fn calc_wrapped_line_count(lines: &[Line<'static>], width: usize) -> usize {
    if width == 0 {
        return lines.len();
    }
    let mut count = 0;
    for line in lines {
        let total_width: usize = line.spans.iter().map(|s| s.content.width()).sum();
        if total_width == 0 || total_width <= width {
            count += 1;
        } else if line.spans.len() == 1 {
            let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
            count += textwrap_wrap(&text, width)
                .into_iter()
                .map(|wrapped| wrapped.as_ref().len())
                .sum::<usize>();
        } else {
            let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
            count += textwrap_wrap(&text, width)
                .into_iter()
                .map(|wrapped| wrapped.as_ref().len())
                .sum::<usize>();
        }
    }
    count
}

/// Wrap styled lines into actual Line<'static> vectors for rendering.
/// Behaves identically to the old wrap_styled_lines_to_lines method.
pub fn wrap_styled_lines_to_lines(
    lines: &[Line<'static>],
    inner_width: usize,
) -> Vec<Line<'static>> {
    if inner_width == 0 {
        return lines.iter().cloned().collect();
    }
    let mut result = Vec::new();
    for line in lines {
        let total_width: usize = line.spans.iter().map(|s| s.content.width()).sum();
        if total_width == 0 || total_width <= inner_width {
            result.push(line.clone());
        } else if line.spans.len() == 1 {
            let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
            result.extend(
                textwrap_wrap(&text, inner_width)
                    .into_iter()
                    .map(|wrapped| {
                        Line::from(Span::styled(wrapped.into_owned(), line.spans[0].style))
                    }),
            );
        } else {
            let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
            let first_style = line.spans.first().map(|s| s.style).unwrap_or_default();
            result.extend(
                textwrap_wrap(&text, inner_width)
                    .into_iter()
                    .map(|wrapped| Line::from(Span::styled(wrapped.into_owned(), first_style))),
            );
        }
    }
    result
}

// ── Block height estimation ───────────────────────────────────────────

/// Estimate rendered height (in terminal rows) for a block.
/// This must match exactly what render_bordered_blocks renders.
pub fn estimate_block_height(
    body_lines: &[StyledLine],
    block_type: &BlockType<'_>,
    inner_width: usize,
) -> u16 {
    let plain_lines: Vec<Line<'static>> = body_lines.iter().map(|sl| sl.content.clone()).collect();

    if plain_lines.is_empty() {
        return match block_type {
            BlockType::Message(_) => 1 + BODY_HEIGHT_ADJUSTER, // min body + borders
            BlockType::ToolResult => 1 + BODY_HEIGHT_ADJUSTER,
        };
    }

    // Determine body content width based on block type
    let body_content_width = match block_type {
        BlockType::Message(_) => inner_width.saturating_sub(2), // -2 for block borders
        BlockType::ToolResult => {
            // Tool result: indented + inner border = -4 total
            inner_width.saturating_sub(4).saturating_sub(2)
        }
    };

    // Number of lines the body content wraps to
    let body_line_count = calc_wrapped_line_count(&plain_lines, body_content_width);

    // Total block height = body lines + border
    body_line_count as u16 + BODY_HEIGHT_ADJUSTER
}

/// Calculate total height of all entries including inter-block margins.
/// Only adds margins between blocks (n-1), NOT after the last block.
pub fn calc_total_height(heights: &[u16]) -> u16 {
    heights.iter().sum::<u16>() + heights.len().saturating_sub(1) as u16
}

// ── Scroll offset controller ──────────────────────────────────────────

/// Compute scroll offset based on viewport position.
///
/// Returns offset in row-count units. The formula:
///   scrollable_range = total_height - viewport_height (min 0)
///   if at_bottom: scrollable_range
///   else: use current position
pub fn compute_scroll_offset(
    _total_height: u16,
    _viewport_height: u16,
    scrollable_range: usize,
    scroll_to_bottom: bool,
    _entry_heights: &[u16],
    manual_scroll_pos: usize,
) -> usize {
    if scroll_to_bottom {
        scrollable_range
    } else {
        manual_scroll_pos
    }
}

/// Calculate which blocks are visible in the current scroll position,
/// and return their Rect areas.
///
/// Returns a vector of (entry_index, Rect) pairs for blocks that:
///   - Overlap with the visible area [scroll_offset, scroll_offset + viewport_height)
///   - Start before msg_area_bottom
///
/// The visible area in content-coordinate space is determined by scroll_offset
/// (the top line shown) and the viewport height (how many lines are visible).
/// A block is visible if its [y, y + block_height) overlaps with the viewport.
pub fn calc_visible_areas(
    msg_area_top: u16,
    msg_area_bottom: u16,
    msg_area_left: u16,
    msg_area_width: u16,
    scroll_offset: usize,
    entry_heights: &[u16],
) -> Vec<(usize, Rect)> {
    let mut areas = Vec::new();
    let mut y: usize = 0;

    for (idx, &block_height) in entry_heights.iter().enumerate() {
        let block_total_height: usize = (block_height + INTER_BLOCK_MARGIN) as usize;
        let y_end: usize = y + block_total_height;

        // Skip if block is completely above the scroll position
        if y_end <= scroll_offset {
            y = y.saturating_add(block_total_height);
            continue;
        }

        // Compute viewport-relative offset from scroll position (content lines relative to viewport top)
        let vp_offset: usize = y.saturating_sub(scroll_offset);

        // Absolute Y in terminal coordinates = msg_area_top + viewport-relative offset
        let abs_y: usize = msg_area_top as usize + vp_offset;

        // Stop if block starts at or past the viewport bottom
        if abs_y >= msg_area_bottom as usize {
            break;
        }

        // Clamp height to fit within viewport
        let available_height: usize = (msg_area_bottom as usize).saturating_sub(abs_y);
        let actual_height: u16 = (block_height as usize).min(available_height).max(1) as u16;

        areas.push((
            idx,
            Rect::new(msg_area_left, abs_y as u16, msg_area_width, actual_height),
        ));

        // Advance to next block (absolute content coordinate)
        y = y.saturating_add(block_total_height);
    }

    areas
}
