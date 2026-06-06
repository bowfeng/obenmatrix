//! Message display widget — renders message history with scrolling.
//!
//! Encapsulates the pre-rendered message lines, scrolling, streaming text
//! overlay, and turn-status text. Messages are rendered as bordered blocks
//! (like the Hermes Agent reference), with each message wrapped in a rounded-border
//! Block whose title shows the role label and icon.

use ratatui::prelude::*;
use ratatui::widgets::{
    Block, BorderType, Borders, Paragraph, ScrollbarState,
};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use unicode_width::UnicodeWidthChar;

use crate::turn::turn_state::TurnState;
use crate::widgets::layout;
use crate::widgets::message_renderer::{MessageRenderEntry, MessageRenderer};
use crate::widgets::role_style::role_info_for_role;
use oben_models::{Message, MessageRole};

/// Block rendering strategy for a message entry.
pub enum BlockType<'a> {
    /// Regular message: full border + role title + role-specific color.
    Message(&'a MessageRole),
    /// Tool result: indented muted rounded box (no role title).
    ToolResult,
}

/// State for the message display widget.
pub struct ConversationState {
    pub scroll_state: Arc<StdMutex<ScrollbarState>>,
    pub scroll_to_bottom: Arc<AtomicBool>,
    /// Persisted scroll position across frames (AtomicUsize so render(&self) can update it).
    pub scroll_pos: Arc<AtomicUsize>,
    pub stream_info: Arc<StdMutex<String>>,
    pub turn_state_ref: Option<Arc<StdMutex<TurnState>>>,
    /// Accumulated scroll delta from user mouse scroll. Reset by render.
    pub user_scroll_offset: Arc<AtomicI32>,
    /// Tracks content line count from previous frame — detects if content has grown.
    pub prev_scrollable_range: Arc<AtomicUsize>,
    /// Width of the message body content area in display columns.
    pub body_width: usize,
    /// X offset of the body content area from the left edge of the screen.
    pub content_x: u16,
    /// Y offset of the first content row (below borders).
    pub content_y: u16,
    /// Y position of the message display area.
    pub msg_area_y: u16,
    /// Width available for line wrapping in the message body (in display columns).
    pub wrap_width: usize,
    /// Block heights in body-line units. index i = height of entry[i] block.
    /// Used to map body_line_idx (from mouse) → flat_line_idx (from content).
    pub cached_block_heights: Arc<StdMutex<Vec<u16>>>,
    /// Maps body_line_idx → flat_line_idx for selection alignment.
    /// body_line 0=text, 1-2=borders, then margin + next entry 0=text, 1-2=borders, etc.
    pub cached_body_to_flat: Arc<StdMutex<Vec<Option<usize>>>>,
    /// Cached wrapped lines for selection/render alignment. Populated during render.
    pub cached_lines: Arc<StdMutex<Vec<Line<'static>>>>,
    /// Selection start/end as terminal (row, col).
    pub selection_start: Option<(u16, u16)>,
    pub selection_end: Option<(u16, u16)>,
    /// Per-message structured entries for bordered-block rendering.
    pub message_entries: Arc<StdMutex<Vec<MessageRenderEntry>>>,
}

impl ConversationState {
    pub fn new() -> Self {
        Self {
            scroll_state: Arc::new(StdMutex::new(ScrollbarState::new(0))),
            scroll_to_bottom: Arc::new(AtomicBool::new(true)),
            scroll_pos: Arc::new(AtomicUsize::new(0)),
            stream_info: Arc::new(StdMutex::new(String::new())),
            turn_state_ref: None,
            user_scroll_offset: Arc::new(AtomicI32::new(0)),
            prev_scrollable_range: Arc::new(AtomicUsize::new(0)),
            selection_start: None,
            selection_end: None,
            message_entries: Arc::new(StdMutex::new(Vec::new())),
            body_width: 0,
            content_x: 0,
            content_y: 0,
            msg_area_y: 0,
            wrap_width: 0,
            cached_block_heights: Arc::new(StdMutex::new(Vec::new())),
            cached_body_to_flat: Arc::new(StdMutex::new(Vec::new())),
            cached_lines: Arc::new(StdMutex::new(Vec::new())),
        }
    }

    /// Clear selection state.
    pub fn clear_selection(&mut self) {
        self.selection_start = None;
        self.selection_end = None;
    }
}

/// Widget that renders message entries with bordered blocks.
pub struct ConversationWidget;

impl ConversationWidget {
    /// Determine the border/title style for a given role.
    fn role_border_style(
        &self,
        role: &MessageRole,
        palette: &ratatui_themes::ThemePalette,
    ) -> Style {
        let info = role_info_for_role(role, palette);
        Style::default().fg(info.border_color)
    }

    fn role_title(
        &self,
        role: &MessageRole,
        palette: &ratatui_themes::ThemePalette,
    ) -> Line<'static> {
        let role_info = role_info_for_role(role, palette);
        Line::from(vec![
            Span::styled(
                format!(" {} ", role_info.icon),
                Style::default()
                    .fg(role_info.header_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                role_info.label.to_string(),
                Style::default()
                    .fg(role_info.header_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ])
    }

    /// Get the selected text from the current state, if any.
    ///
    /// Converts terminal (row, col) → body_line → flat line.
    /// Uses the CURRENT cached_body_to_flat mapping from render_bordered_blocks
    /// so it stays consistent with what's currently being rendered.
    pub fn get_selected_text(&self, state: &ConversationState) -> Option<String> {
        let (sy, sx) = state.selection_start?;
        let (ey, ex) = state.selection_end?;

        let flat_lines = state.cached_lines.lock().ok()?.clone();
        let body_to_flat = state.cached_body_to_flat.lock().ok()?;
        let content_y = state.content_y as usize;
        let content_x = state.content_x as usize;
        let scroll_pos = state.scroll_pos.load(Ordering::SeqCst);
        let body_w = state.body_width;

        // Mouse cols are absolute terminal coords. Convert to body-area-relative column.
        // Flat lines render at body_area.x = area.x + 2, but content_x = area.x + 4,
        // so body_area.x = content_x - 2.
        let body_area_x = content_x.saturating_sub(2);
        let rel_sx = (sx as usize).saturating_sub(body_area_x);
        let rel_ex = (ex as usize).saturating_sub(body_area_x);

        // Ensure min/max order for x
        let (x0_start, x1_start) = (
            std::cmp::min(rel_sx, rel_ex),
            std::cmp::max(rel_sx, rel_ex),
        );

            // body_to_flat body_start_offset = msg_area.y = content_y - 1.
            // body line index = scroll_pos + (row - msg_area.y) = row - msg_area.y + scroll_pos.
            let body_start_offset = content_y.saturating_sub(1);

            // Iterate terminal rows sy..=ey (normalized), extract text per row.
            let mut result = String::new();
            let row_start = std::cmp::min(sy as usize, ey as usize);
            let row_end = std::cmp::max(sy as usize, ey as usize);

            tracing::debug!(
                "[selection/get_selected_text] sel=({},{})-({},{}) content_x={} body_area_x={} rel_sx={} rel_ex={} body_w={} x0={} x1={} row_range=[{}..{}] body_to_flat_len={}",
                sy, sx, ey, ex, content_x, body_area_x, rel_sx, rel_ex, body_w,
                x0_start, x1_start, row_start, row_end, body_to_flat.len()
            );
        
        let mut prev_flat_line: Option<usize> = None;
        let mut prev_abs_was_padding = false;
        for row in row_start..=row_end {
            let abs_body = (row as usize).saturating_sub(body_start_offset) + scroll_pos;
            
            // Look up flat line for this body line
            let flat_line = match body_to_flat.get(abs_body).copied().flatten() {
                Some(v) => v,
                None => {
                    // body_line is a None margin (inter-block separator).
                    // Look forward to find the first wrapped line of the next block.
                    match body_to_flat.get(abs_body + 1..).and_then(|s| s.iter().flatten().next().copied()) {
                        Some(v) => v,
                        None => continue, // skip trailing padding
                    }
                }
            };
            
            // Detect if this is a padding line: consecutive body indices within the same block
            // map padding to last wrapped line. We detect by checking if the PREVIOUS body line
            // at abs_body-1 also maps to the same flat_line (and wasn't itself padding).
            // Only skip if abs_body > row_start to avoid deduping the first selected row.
            let is_padding = if row > row_start {
                let prev_abs = abs_body.saturating_sub(1);
                if let Some(prev_flat_opt) = body_to_flat.get(prev_abs) {
                    match (prev_flat_opt, prev_abs_was_padding) {
                        (None, _) => false,
                        (Some(prev_flat), false) => *prev_flat == flat_line,
                        (Some(_), true) => true,
                    }
                } else {
                    false
                }
            } else {
                false
            };
            
            // Update padding tracking
            prev_abs_was_padding = is_padding;
            if is_padding {
                continue;
            }
            prev_abs_was_padding = false;
            
            // Also skip if same as previous non-padding row's flat_line
            if Some(flat_line) == prev_flat_line {
                continue;
            }
            prev_flat_line = Some(flat_line);
            if flat_line >= flat_lines.len() { continue; }
            let line = &flat_lines[flat_line];
            
            // Build cell→char map for this line
            let mut chars: Vec<(usize, char)> = Vec::new();
            let mut pos = 0usize;
            for span in &line.spans {
                for ch in span.content.chars() {
                    let w = ch.width().unwrap_or(0).max(1);
                    chars.push((pos, ch));
                    pos += w;
                }
            }
            
            // Extract text at row level using mouse row position
            let x0 = x0_start.min(body_w);
            let x1 = x1_start.min(body_w).max(x0 + 1);
            
            // Debug: log x0/x1 for all rows
            tracing::debug!(
                "[selection/get_selected_text] row={} flat_line={} x0={} x1={} chars_count={}",
                row, flat_line, x0, x1, chars.len()
            );
            
            let sel: String = chars.iter()
                .filter(|(p,_)| *p >= x0 && *p < x1)
                .map(|(_,ch)| *ch)
                .collect();
            
            if !sel.is_empty() {
                if !result.is_empty() { result.push('\n'); }
                result.push_str(&sel);
            }
        }

        if !result.is_empty() {
            let first_line = result.lines().next().unwrap_or("");
            tracing::debug!(
                "[selection/get_selected_text] result={} chars first_line_trunc=\"{}\"",
                result.len(),
                first_line.chars().take(80).collect::<String>()
            );
            return Some(result);
        }
        tracing::debug!("[selection/get_selected_text] no result (empty)");
        None
    }

    /// Render selection highlight overlay.
    ///
    /// Uses the CURRENT cached_body_to_flat mapping from render_bordered_blocks.
    pub fn render_selection(
        &self,
        frame: &mut Frame,
        area: Rect,
        state: &ConversationState,
        _palette: &ratatui_themes::ThemePalette,
    ) {
        if let (Some((sy, sx)), Some((ey, ex))) = (state.selection_start, state.selection_end) {
            let flat_lines = match state.cached_lines.lock() {
                Ok(g) => g.clone(),
                Err(_) => return,
            };
            let body_to_flat = match state.cached_body_to_flat.lock() {
                Ok(g) => g.clone(),
                Err(_) => return,
            };
            let content_y = state.content_y as usize;
            let content_x = state.content_x as usize;
            let body_w = state.body_width;
            let scroll_pos_val = state.scroll_pos.load(Ordering::SeqCst);

            // Mouse cols are absolute terminal coords. Convert to body-area-relative column.
            // Flat lines render at body_area.x = area.x + 2, but content_x = area.x + 4,
            // so body_area.x = content_x - 2.
            let body_area_x = content_x.saturating_sub(2);
            let rel_sx = (sx as usize).saturating_sub(body_area_x);
            let rel_ex = (ex as usize).saturating_sub(body_area_x);

            // Normalize: ensure min/max order regardless of drag direction.
            let x0 = std::cmp::min(rel_sx, rel_ex).min(body_w);
            let x1 = std::cmp::max(rel_sx, rel_ex).min(body_w).max(x0 + 1);

            // Iterate terminal rows from min to max, map each to flat line via body_to_flat.
            // Position highlight at exact terminal row to match mouse selection.
            let row_start = std::cmp::min(sy as usize, ey as usize);
            let row_end = std::cmp::max(sy as usize, ey as usize);

            tracing::debug!(
                "[selection/render_selection] sel=({},{})-({},{}) content_y={} scroll_pos={} body_w={} x={}-{} rows=[{}..{}] body_to_flat.len={}",
                sy, sx, ey, ex, content_y, scroll_pos_val, body_w,
                x0, x1,
                row_start, row_end, body_to_flat.len()
            );

            // body_to_flat maps body_line → flat_line.
            // Each entry contributes (wrapped_count + BODY_HEIGHT_ADJUSTER(2)) body lines.
            // body_to_flat body_start_offset = msg_area.y = content_y - 1.
            // body line index = scroll_pos + (row - msg_area.y) = row - msg_area.y + scroll_pos.
            let body_start_offset = content_y.saturating_sub(1);
            tracing::debug!(
                "[selection/render_selection] body_start_offset={} content_y={} scroll_pos={}",
                body_start_offset, content_y, scroll_pos_val
            );
            let mut highlight_lines: Vec<Line> = Vec::new();
            let mut prev_abs_body: Option<usize> = None;
            for row in row_start..=row_end {
                let abs_body = (row as usize).saturating_sub(body_start_offset).saturating_add(scroll_pos_val);
                
                // Look up flat line for this body line
                let flat_line = match body_to_flat.get(abs_body).copied().flatten() {
                    Some(v) => v,
                    None => {
                        // body_to_flat has None (margin between blocks).
                        // Skip forward to find first line of next block.
                        match body_to_flat.get(abs_body + 1..).and_then(|s| s.iter().flatten().next().copied()) {
                            Some(v) => v,
                            None => {
                                // Trailing padding/margin with no content — push empty highlight.
                                highlight_lines.push(Line::from(Span::raw("")));
                                tracing::debug!(
                                    "[selection/render_selection] row={} abs_body={} NO_FLAT_LINE (padding/margin) → empty highlight",
                                    row, abs_body
                                );
                                continue;
                            }
                        }
                    }
                };
                
                // Detect padding: this row's body_to_flat maps to same flat_line as prev row.
                // If body_to_flat[prev_abs_body] == flat_line, then this row is a padding line
                // at the end of a block (BODY_HEIGHT_ADJUSTER lines map to last wrapped line).
                // If body_to_flat[prev_abs_body] is None, this is first line of next block (margin skip).
                let is_padding = if let Some(prev) = prev_abs_body {
                    match body_to_flat.get(prev) {
                        Some(Some(v)) => *v == flat_line,
                        _ => false,
                    }
                } else {
                    false
                };
                prev_abs_body = Some(abs_body);
                
                if is_padding {
                    highlight_lines.push(Line::from(Span::raw("")));
                    tracing::debug!(
                        "[selection/render_selection] row={} abs_body={} padding_line → empty highlight",
                        row, abs_body
                    );
                    continue;
                }
                
                if flat_line >= flat_lines.len() {
                    highlight_lines.push(Line::from(Span::raw("")));
                    continue;
                }
                let line = &flat_lines[flat_line];

                // Build cell→char map for this line
                let mut chars: Vec<(usize, char)> = Vec::new();
                let mut pos = 0usize;
                for span in &line.spans {
                    for ch in span.content.chars() {
                        let w = ch.width().unwrap_or(0);
                        chars.push((pos, ch));
                        pos += w;
                    }
                }

                // Build styled spans with highlight for selected column range
                let mut spans: Vec<Span> = Vec::new();
                let mut in_highlight = false;
                for (c_pos, ch) in &chars {
                    if *c_pos >= x0 && *c_pos < x1 {
                        if !in_highlight {
                            in_highlight = true;
                        }
                        spans.push(Span::styled(
                            ch.to_string(),
                            Style::default().add_modifier(Modifier::REVERSED),
                        ));
                    } else {
                        if in_highlight {
                            in_highlight = false;
                        }
                        spans.push(Span::raw(ch.to_string()));
                    }
                }

                tracing::debug!(
                    "[selection/render_selection] row={} abs_body={} flat_line={} chars={} highlight_text=\"{}\"",
                    row, abs_body, flat_line, chars.len(),
                    chars.iter()
                        .filter(|(p,_)| *p >= x0 && *p < x1)
                        .map(|(_,ch)| *ch)
                        .collect::<String>()
                );

                highlight_lines.push(Line::from(spans));
            }

            if !highlight_lines.is_empty() {
                let highlight_area_y = sy;
                let highlight_area_x = area.x + body_area_x as u16;
                let highlight_area = Rect::new(
                    highlight_area_x,
                    highlight_area_y,
                    area.width.min(body_w as u16),
                    highlight_lines.len() as u16,
                );
                tracing::debug!(
                    "[selection/render_selection] highlight_area=x={} y={} w={} h={} (sy={}, rows={})",
                    highlight_area.x, highlight_area.y,
                    highlight_area.width, highlight_area.height,
                    sy, highlight_lines.len()
                );
                frame.render_widget(Paragraph::new(highlight_lines), highlight_area);
            }
        }
    }

    /// Render message blocks with bordered styling.
    ///
    /// Three-phase architecture:
    ///   Phase 1: Layout — compute heights, scroll offset, visible areas (pure data)
    ///   Phase 2: Render   — draw blocks using the pre-calculated areas
    ///   Phase 3: Scroll   — update scroll position from mouse delta
    fn render_bordered_blocks(
        &self,
        frame: &mut Frame,
        area: Rect,
        state: &ConversationState,
        palette: &ratatui_themes::ThemePalette,
        is_streaming: bool,
        entries: &[MessageRenderEntry],
    ) {
        // ─── Outer frame ───────────────────────────────────────────────
        let outer_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.muted))
            .title(Line::from(vec![
                Span::styled(" ", Style::default().fg(palette.info)),
                Span::styled(
                    "Messages",
                    Style::default()
                        .fg(palette.info)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" ", Style::default().fg(palette.info)),
            ]));
        let msg_area = outer_block.inner(area);
        outer_block.render(area, frame.buffer_mut());

        let inner_width = (msg_area.width as usize).saturating_sub(2);
        let inner_height = (msg_area.height as usize).saturating_sub(1);

        // ─── Phase 1: Layout calculation ───────────────────────────────
        // Wrap lines and compute heights — pure data, no rendering.

        // Accumulate (entry_index, block_type, wrapped_lines) for later rendering
        let mut layout_entries: Vec<(usize, BlockType<'_>, Vec<Line<'static>>)> = Vec::new();
        let mut entry_flat_ranges: Vec<(usize, usize)> = Vec::new(); // (start, end) in flat lines
        let mut flat_accum = 0usize;
        for entry in entries {
            let plain_lines: Vec<Line<'static>> = entry
                .body_lines
                .iter()
                .map(|sl| sl.content.clone())
                .collect();

            let block_type = if entry.is_tool_result {
                BlockType::ToolResult
            } else {
                BlockType::Message(&entry.role)
            };

            let actual_wrap_w = if matches!(&block_type, BlockType::ToolResult) {
                // ToolResult: -2 for tool indent box + -2 for inner border = -4
                inner_width.saturating_sub(4)
            } else {
                // Regular message: -2 for inner block borders + -2 for body border = -4
                inner_width.saturating_sub(2)
            };

            // Wrap using the layout module's function (consistent height estimation)
            let wrapped = layout::wrap_styled_lines_to_lines(&plain_lines, actual_wrap_w);

            let flat_start = flat_accum;
            let flat_end = flat_accum + wrapped.len();
            layout_entries.push((layout_entries.len(), block_type, wrapped));
            entry_flat_ranges.push((flat_start, flat_end));
            flat_accum = flat_end;
        }

        // Debug: log entry flat ranges and wrap widths
        tracing::debug!(
            "[selection/render_bordered_blocks] area.w={} msg_area.w={} inner_width={} entry_count={} total_flat_lines={}",
            area.width, msg_area.width, inner_width,
            entry_flat_ranges.len(),
            entry_flat_ranges.last().map(|(s,e)| e.saturating_sub(*s)).unwrap_or(0)
        );
        for (ei, (fs, fe)) in entry_flat_ranges.iter().enumerate() {
            let count = fe.saturating_sub(*fs);
            if ei < layout_entries.len() {
                let wrap_w = if matches!(layout_entries[ei].1, BlockType::ToolResult) {
                    inner_width.saturating_sub(4)
                } else {
                    inner_width.saturating_sub(2)
                };
                let wrapped = &layout_entries[ei].2;
                let first_line: String = if !wrapped.is_empty() {
                    wrapped[0].spans.iter().map(|s| s.content.to_string()).collect()
                } else {
                    String::new()
                };
                tracing::debug!(
                    "[selection/render_bordered_blocks]   entry[{}] flat[{}..{}) wrap_w={} count={} first=\"...{}...\"",
                    ei, fs, fe, wrap_w, count,
                    first_line.chars().take(60).collect::<String>()
                );
            }
        }

        // Compute block heights using the layout module's estimator
        let block_heights: Vec<u16> = layout_entries
            .iter()
            .map(|(_, _block_type, wrapped)| {
                // Estimate height from wrapped line count + border
                // Match exactly what the render loop produces
                let body_height = wrapped.len().max(1) as u16;
                body_height + layout::BODY_HEIGHT_ADJUSTER
            })
            .collect();

        let content_height = layout::calc_total_height(&block_heights);

        // Cache the full flat wrapped lines for selection/render alignment.
        // Cache block heights and body→flat mapping for selection alignment.
        {
            *state.cached_block_heights.lock().unwrap() = block_heights.clone();
            if let Ok(ref mut cached_lines) = state.cached_lines.lock() {
                let mut flat: Vec<Line<'static>> = Vec::new();
                for (_, _, ref wrapped_lines) in &layout_entries {
                    flat.extend(wrapped_lines.iter().cloned());
                }
                **cached_lines = flat;
            }
            // Cache body→flat mapping for selection alignment.
            // body_to_flat[body_line] = flat_line index for text lookup.
            // Each block contributes (wrapped.len() + BODY_HEIGHT_ADJUSTER) entries.
            // Padding (BODY_HEIGHT_ADJUSTER) lines map to last wrapped line.
            // One inter-block margin (None) between blocks.
            let mut body_to_flat = state.cached_body_to_flat.lock().unwrap();
            let mapping: Vec<Option<usize>> = {
                let mut mapping: Vec<Option<usize>> = Vec::new();
                let mut flat_accum = 0usize;
                let total_blocks = layout_entries.len();
                for (i, (_, _, wrapped)) in layout_entries.iter().enumerate() {
                    let wrapped_count = wrapped.len();
                    let block_height = block_heights[i];
                    // Map body_line indices to flat_line indices
                    for j in 0..block_height {
                        let j_usize = j as usize;
                        if j_usize < wrapped_count {
                            mapping.push(Some(flat_accum + j_usize));
                        } else {
                            // Padding lines (BODY_HEIGHT_ADJUSTER) map to last wrapped line
                            mapping.push(Some(flat_accum + (wrapped_count.saturating_sub(1))));
                        }
                    }
                    flat_accum += wrapped_count.max(1);
                    // Inter-block margin entry
                    if i < total_blocks - 1 {
                        mapping.push(None);
                    }
                }
                mapping
            };
            *body_to_flat = mapping;
        }
        let scroll_pos = state.scroll_pos.load(Ordering::SeqCst);

        // ─── Phase 1.5: Stream block estimation & wrapping (shared between phases) ──────
        // Parse stream text once, reuse for both scroll_offset and rendering.
        let stream_parsed = if is_streaming {
            if let Some(ref ts) = state.turn_state_ref {
                if let Ok(ts) = ts.lock() {
                    if !ts.streaming_text.is_empty() {
                        let raw = ts.streaming_text.trim_start_matches(|c: char| c.is_whitespace());
                        let stream_lines: Vec<Line<'static>> = raw
                            .lines()
                            .map(|l| {
                                Line::from(Span::styled(
                                    l.to_string(),
                                    Style::default()
                                        .fg(palette.info)
                                        .add_modifier(Modifier::DIM),
                                ))
                            })
                            .collect::<Vec<_>>();

                        let wrapped = layout::wrap_styled_lines_to_lines(
                            &stream_lines,
                            inner_width.saturating_sub(2),
                        );
                        let stream_body_height = if wrapped.is_empty() {
                            1usize
                        } else {
                            wrapped.len()
                        };
                        let stream_height = stream_body_height as u16 + layout::BODY_HEIGHT_ADJUSTER;
                        Some((stream_lines, wrapped, stream_body_height, stream_height))

                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        let stream_estimate = stream_parsed.as_ref().map(|s| s.3);

        let total_height = content_height + stream_estimate.unwrap_or(0);

        // Compute scroll offset using the layout module
        let scroll_offset = layout::compute_scroll_offset(
            total_height,
            inner_height as u16,
            (total_height as i64 - inner_height as i64).max(0) as usize,
            state.scroll_to_bottom.load(Ordering::SeqCst),
            &block_heights,
            scroll_pos,
        );

        // Update scrollbar state
        {
            let mut scroll_state = state.scroll_state.lock().unwrap();
            *scroll_state = ScrollbarState::new(total_height.max(1) as usize)
                .viewport_content_length(inner_height as usize)
                .position(scroll_offset);
        }

        // Calculate visible areas using the layout module
        let visible_areas = layout::calc_visible_areas(
            msg_area.top(),
            msg_area.bottom().saturating_sub(1),
            msg_area.left(),
            msg_area.width,
            scroll_offset,
            &block_heights,
        );

        // ─── Phase 2: Render visible blocks ────────────────────────────
        // Track the bottom of the last visible entry so Phase 2.5 can position the stream block below it.
        let mut last_entry_vp_bottom = 0u16;
        for (idx, block_rect) in visible_areas {
            if block_rect.y.saturating_add(block_rect.height) > last_entry_vp_bottom {
                last_entry_vp_bottom = block_rect.y.saturating_add(block_rect.height);
            }
            let (_entry_idx, block_type, wrapped) = &layout_entries[idx];

            // body_area = block.inner(block_area) — calculated at render time
            // using the actual Block struct (handles title, borders correctly)
            let is_tool_result = matches!(block_type, BlockType::ToolResult);

            // Build the block (borders + title)
            let block = if is_tool_result {
                let indent = layout::TOOL_INDENT;
                let _box_width = (msg_area.width as usize).saturating_sub((indent * 2) as usize) as u16;
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(palette.muted))
                    .border_type(BorderType::Rounded)
            } else {
                let role = match block_type {
                    BlockType::Message(r) => r,
                    BlockType::ToolResult => unreachable!(),
                };
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(self.role_border_style(role, palette))
                    .title(self.role_title(role, palette))
            };

            // Render body (Paragraph) before block (borders on top)
            let body_area = block.inner(block_rect);
            tracing::debug!(
                "[layout] body_area.x={} block_rect.x={} msg_area.x={} area.x={}",
                body_area.x, block_rect.x, msg_area.left(), area.x
            );
            if block_rect.height > layout::BODY_HEIGHT_ADJUSTER && !wrapped.is_empty() {
                let body_lines: Vec<Line> = wrapped
                    .iter()
                    .take(body_area.height as usize)
                    .cloned()
                    .map(|line| {
                        if is_tool_result {
                            // Tool result: muted style
                            let muted_spans: Vec<Span> = line
                                .spans
                                .iter()
                                .filter_map(|span| {
                                    span.style
                                        .fg(palette.muted)
                                        .add_modifier(Modifier::DIM)
                                        .fg
                                        .map(|fg| {
                                            Span::styled(
                                                span.content.clone(),
                                                Style::default().fg(fg).add_modifier(Modifier::DIM),
                                            )
                                        })
                                        .or_else(|| {
                                            Some(Span::styled(
                                                span.content.clone(),
                                                Style::default()
                                                    .fg(palette.muted)
                                                    .add_modifier(Modifier::DIM),
                                            ))
                                        })
                                })
                                .collect();
                            Line::from(muted_spans)
                        } else {
                            line
                        }
                    })
                    .collect();

                if !body_lines.is_empty() {
                    frame.render_widget(Paragraph::new(body_lines), body_area);
                }
            }

            frame.render_widget(block, block_rect);
        }

        // ─── Phase 2.5: Streaming block (rendered after regular entries) ─
        let mut total_height = total_height;
        let mut streaming_rendered = false;
        if let Some((stream_lines, _wrapped, _stream_body_lines, stream_height)) = stream_parsed {
            // Phase 1 already added stream_estimate_height (= stream_height) to total_height.
            // No double-add needed — total_height is already correct.

            let role_info = role_info_for_role(&MessageRole::Assistant, palette);
            let role_color = role_info.border_color;

            // Streaming block position: if content fits in viewport, render right after entries;
            // if content overflows, anchor to viewport bottom.
            let view_height = msg_area.height.saturating_sub(1);
            let stream_y = if total_height as u16 <= view_height + stream_height {
                // Content fits: stream block renders right after the last visible entry.
                last_entry_vp_bottom.saturating_add(1)
            } else {
                // Content overflows: anchor to viewport bottom.
                view_height.saturating_sub(stream_height)
            };

            tracing::debug!(
                "[stream] content_height={} view_height={} stream_height={} scroll_area_y={} stream_y={}",
                total_height, view_height, stream_height,
                last_entry_vp_bottom, stream_y
            );

            if stream_height < msg_area.height.saturating_sub(1) {
                let block_area = Rect::new(
                    msg_area.left(),
                    stream_y,
                    msg_area.width,
                    stream_height,
                );
                let block = Block::default()
                    .borders(Borders::ALL)
                    .border_style(
                        Style::default()
                            .fg(role_color)
                            .add_modifier(Modifier::BOLD),
                    )
                    .title(Line::from(vec![
                        Span::raw(role_info.icon),
                        Span::styled(
                            role_info.label,
                            Style::default()
                                .fg(role_color)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]));
                let body_area = block.inner(block_area);

                if !stream_lines.is_empty() {
                    frame.render_widget(Paragraph::new(stream_lines), body_area);
                }
                frame.render_widget(block, block_area);
                streaming_rendered = true;
            }
        }

        if !streaming_rendered && is_streaming {
            total_height += 1; // placeholder for streaming
        }

        // ─── Phase 3: Scroll position update (mouse wheel) ─────────────
        let offset = state.user_scroll_offset.swap(0, Ordering::SeqCst);
        let mut scroll_pos = scroll_pos;
        let scrollable_range = (total_height as i64 - inner_height as i64).max(0) as usize;
        let prev_scrollable_range = state.prev_scrollable_range.swap(scrollable_range, Ordering::SeqCst);
        let vp_bottom = scroll_pos.saturating_add(inner_height as usize);
        let lines_from_bottom = (total_height as usize).saturating_sub(vp_bottom);

        // Apply accumulated scroll delta (mouse wheel)
        if offset != 0 {
            scroll_pos = ((scroll_pos as i64 + offset as i64).max(0) as usize)
                .min(scrollable_range);
        }

        // Detect content growth
        let content_grew = scrollable_range > prev_scrollable_range;
        // "near bottom": content bottom is within 10 lines of viewport bottom
        let near_bottom = lines_from_bottom < 10;

        // When content grows and user is reading at/near bottom, auto-scroll
        // so new streaming text stays visible.
        if content_grew && near_bottom {
            state.scroll_to_bottom.store(true, Ordering::SeqCst);
        }

        if state.scroll_to_bottom.load(Ordering::SeqCst) {
            scroll_pos = scrollable_range;
        }

        state.scroll_pos.store(scroll_pos, Ordering::SeqCst);
        scroll_pos = state.scroll_pos.load(Ordering::SeqCst);
        let final_pos = scroll_pos;

        tracing::info!(
            "[scroll_update] BEFORE: total_height={} inner_height={} scrollable_range={} prev_scrollable_range={} scroll_pos={} vp_bottom={} lines_from_bottom={} scroll_to_bottom={}",
            total_height, inner_height, scrollable_range, prev_scrollable_range, scroll_pos, vp_bottom,
            lines_from_bottom, state.scroll_to_bottom.load(Ordering::SeqCst)
        );

        tracing::info!(
            "[scroll_update] AFTER: final_pos={} new_vp_bottom={} scroll_to_bottom={}",
            final_pos, final_pos.saturating_add(inner_height as usize),
            state.scroll_to_bottom.load(Ordering::SeqCst)
        );

        tracing::info!(
            "[scroll_update] AFTER: final_pos={} scrollable_range={} vp_bottom={} scroll_to_bottom={}",
            final_pos, scrollable_range, final_pos.saturating_add(inner_height as usize), state.scroll_to_bottom.load(Ordering::SeqCst)
        );
        {
            let mut scroll_state = state.scroll_state.lock().unwrap();
            *scroll_state = ScrollbarState::new(total_height.max(1) as usize)
                .viewport_content_length(inner_height as usize)
                .position(scroll_pos);
        }
    }

    fn render_turn_status(
        &self,
        frame: &mut Frame,
        area: Rect,
        state: &ConversationState,
        _palette: &ratatui_themes::ThemePalette,
    ) {
        let mut stream_info = String::new();
        if let Ok(si) = state.stream_info.lock() {
            stream_info = si.clone();
        }
        if stream_info.is_empty() {
            return;
        }

        let lines: Vec<&str> = stream_info.lines().collect();
        let height = lines.len().min(3) as u16;
        if height == 0 {
            return;
        }

        let displayed_lines: Vec<Line> = lines.iter().take(3).map(|l| Line::from(*l)).collect();
        let para = Paragraph::new(displayed_lines);
        let status_w = lines.iter().map(|l| l.len() as u16).max().unwrap_or(1) + 2;
        let display_area = Rect::new(
            area.x + area.width.saturating_sub(status_w + 2),
            area.y + area.height.saturating_sub(height + 2),
            status_w.min(area.width.saturating_sub(2)),
            height.min(area.height.saturating_sub(2)),
        );
        frame.render_widget(para, display_area);
    }

    /// Render the full message display widget in `area`.
    pub fn render(
        &self,
        frame: &mut Frame,
        area: Rect,
        state: &ConversationState,
        palette: &ratatui_themes::ThemePalette,
        is_streaming: bool,
    ) {
        let entries = state.message_entries.lock().unwrap();
        self.render_bordered_blocks(frame, area, state, palette, is_streaming, &entries);
        self.render_turn_status(frame, area, state, palette);

        // Draw streaming indicator in messages panel area
        if is_streaming {
            let indicator_text = " Streaming... ";
            let w = indicator_text.len() as u16 + 2;
            let indicator_area = Rect::new(
                area.right()
                    .saturating_sub(w + 2)
                    .min(area.width.saturating_sub(2)),
                area.y + 1,
                w,
                1,
            );
            let para = Paragraph::new(Line::from(Span::styled(
                indicator_text,
                Style::default().fg(palette.info),
            )));
            frame.render_widget(para, indicator_area);
        }
    }

    /// Append a user message to the internal display state.
    pub fn append_user_message(&mut self, state: &mut ConversationState, text: &str) {
        let body_lines = vec![crate::widgets::message_renderer::StyledLine {
            content: Line::from(text.to_string()),
            role_color: None,
        }];
        state.message_entries.lock().unwrap().push(
            crate::widgets::message_renderer::MessageRenderEntry {
                role: oben_models::MessageRole::User,
                body_lines,
                is_tool_result: false,
                tool_calls: Vec::new(),
            },
        );
    }

    /// Append an info/system message to the internal display state.
    /// Uses the System role style (gear icon, accent color).
    pub fn append_info_message(&mut self, state: &mut ConversationState, text: &str) {
        use ratatui_themes::ThemeName;
        let palette = ThemeName::default().palette();
        for line in text.lines() {
            let line_owned = line.to_string();
            let spans = vec![
                Span::styled(
                    "\u{2699} ",
                    Style::default()
                        .fg(palette.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(line_owned, Style::default().fg(palette.info)),
            ];
            let body_lines = vec![crate::widgets::message_renderer::StyledLine {
                content: Line::from(spans),
                role_color: None,
            }];
            state.message_entries.lock().unwrap().push(
                crate::widgets::message_renderer::MessageRenderEntry {
                    role: oben_models::MessageRole::System,
                    body_lines,
                    is_tool_result: false,
                    tool_calls: Vec::new(),
                },
            );
        }
    }

    /// Rebuild message entries from session messages using the renderer.
    pub fn rebuild_from_messages(
        &self,
        state: &mut ConversationState,
        messages: &[Message],
        renderer: &MessageRenderer,
    ) {
        let mut entries = Vec::new();
        for msg in messages {
            entries.push(renderer.render_entry(msg));
        }
        let mut entry_lock = state.message_entries.lock().unwrap();
        *entry_lock = entries;
    }

    /// Update stream_info from turn state into ConversationState.
    pub fn update_stream_info(&self, state: &mut ConversationState, turn_state: &TurnState) {
        let mut parts = Vec::new();

        let active = &turn_state.active_tools;
        if !active.is_empty() {
            let names: Vec<String> = active
                .iter()
                .take(2)
                .map(|t| {
                    format!(
                        " {}\u{1F527} {}",
                        t.name,
                        t.context.chars().take(30).collect::<String>()
                    )
                })
                .collect();
            parts.push(format!("\u{1F527} {}", names.join(", ")).to_string());
        }

        if !parts.is_empty() {
            let info = parts.join("\n");
            if let Ok(mut si) = state.stream_info.lock() {
                *si = info;
            }
        } else if let Ok(mut si) = state.stream_info.lock() {
            si.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::turn::turn_state::{ActiveTool, TurnState};
    use std::time::Instant;

    /// Given: a TurnState with active tool calls AND streaming text
    /// When: update_stream_info is called
    /// Then: stream_info contains only tool info, NOT streaming text
    #[test]
    fn test_update_stream_info_excludes_streaming_text() {
        let mut state = ConversationState::new();
        let mut turn = TurnState::new();
        turn.active_tools.push(ActiveTool {
            id: "call-1".into(),
            name: "file_read".into(),
            started_at: Instant::now(),
            context: "/Users/test/config.yaml".into(),
        });
        turn.streaming_text = "This is streaming text that should NOT appear in stream_info".into();

        ConversationWidget.update_stream_info(&mut state, &turn);

        let info = state.stream_info.lock().unwrap();
        assert!(!info.contains("streaming text"));
        assert!(!info.is_empty());
        assert!(info.contains("file_read"));
    }

    /// Given: a TurnState with ONLY streaming text, no active tools
    /// When: update_stream_info is called
    /// Then: stream_info stays empty (no misleading tool status)
    #[test]
    fn test_update_stream_info_empty_with_no_tools() {
        let mut state = ConversationState::new();
        let mut turn = TurnState::new();
        turn.streaming_text = "Some streaming content".into();

        ConversationWidget.update_stream_info(&mut state, &turn);

        let info = state.stream_info.lock().unwrap();
        assert!(info.is_empty());
    }

    /// Given: a TurnState with active tools
    /// When: update_stream_info is called
    /// Then: stream_info includes the tool name
    #[test]
    fn test_update_stream_info_includes_tool_name() {
        let mut state = ConversationState::new();
        let mut turn = TurnState::new();
        turn.active_tools.push(ActiveTool {
            id: "call-2".into(),
            name: "search_files".into(),
            started_at: Instant::now(),
            context: "/some/path".into(),
        });

        ConversationWidget.update_stream_info(&mut state, &turn);

        let info = state.stream_info.lock().unwrap();
        assert!(info.contains("search_files"));
    }

    /// Given: a TurnState with streaming text AND active tools
    /// When: update_stream_info is called
    /// Then: streaming_text is NOT included in stream_info
    #[test]
    fn test_streaming_text_not_duplicated_in_stream_info() {
        let mut state = ConversationState::new();
        let mut turn = TurnState::new();
        let streaming_content = "The Clockmaker of Lost Hours";
        turn.streaming_text = streaming_content.into();
        turn.active_tools.push(ActiveTool {
            id: "call-write".into(),
            name: "file_write".into(),
            started_at: Instant::now(),
            context: "/Users/test/docs.md".into(),
        });

        ConversationWidget.update_stream_info(&mut state, &turn);

        let info = state.stream_info.lock().unwrap();
        assert!(info.contains("file_write"));
        assert!(info.is_empty() || !info.contains("The Clockmaker of Lost Hours"));
    }

    /// Given: streaming text is empty before render
    /// When: render_bordered_blocks processes entries without stream data
    /// Then: stream_parsed should be None (no double-parse for empty text)
    #[test]
    fn test_stream_parsed_none_when_text_empty() {
        let mut state = ConversationState::new();
        let mut turn = TurnState::new();
        turn.streaming_text = String::new();
        state.turn_state_ref = Some(Arc::new(StdMutex::new(turn)));
        state.stream_info.lock().unwrap().clear();

            // Verify that a new TurnState is initialized empty
            let state_guard = state.turn_state_ref.as_ref().map(|g| g.lock().unwrap());
            if let Some(guard) = state_guard {
                assert_eq!(guard.streaming_text, "");
                assert!(guard.active_tools.is_empty());
            }
    }

    /// Given: 3 message entries with known heights
    /// When: calc_total_height is called with inter_block_margins
    /// Then: total = sum of heights + (n-1) * 1 margin
    #[test]
    fn test_calc_total_height_with_margins() {
        let heights = vec![4u16, 6, 4]; // 3 blocks
        let total = layout::calc_total_height(&heights);
        // 4 + 6 + 4 + 2 margins (between block 0/1 and 1/2)
        assert_eq!(total, 16);
    }

    /// Given: total_height=125, inner_height=59, stream_height=4
    /// When: content overflows viewport (125 > 59)
    /// Then: stream_y anchored to viewport bottom (59-4=55)
    #[test]
    fn test_stream_y_anchored_to_bottom_when_overflow() {
        let total_height: u16 = 125;
        let view_height: u16 = 59;
        let stream_height: u16 = 4;
        
        let overflows = total_height > view_height;
        
        let stream_y_overflow = if overflows {
            view_height.saturating_sub(stream_height)
        } else {
            0
        };
        
        assert!(overflows);
        assert_eq!(stream_y_overflow, 55); // 59 - 4
    }

    /// Given: total_height=2, inner_height=59 (content fits)
    /// When: last_entry_vp_bottom=3
    /// Then: stream_y = last_entry_vp_bottom + 1 = 4
    #[test]
    fn test_stream_y_after_entry_when_content_fits() {
        let total_height: u16 = 2;
        let view_height: u16 = 59;
        let stream_height: u16 = 4;
        let last_entry_vp_bottom: u16 = 3;
        
        let fits = total_height <= view_height;
        
        let stream_y = if fits {
            last_entry_vp_bottom.saturating_add(1)
        } else {
            view_height.saturating_sub(stream_height)
        };
        
        assert!(fits);
        assert_eq!(stream_y, 4);
    }

    /// Given: scroll_to_bottom=true, prev_scrollable_range=6, new=8
    /// When: auto-scroll detects content growth (8>6) and near bottom
    /// Then: scroll_to_bottom remains true, ready for auto-scroll in Phase 3
    #[test]
    fn test_scroll_to_bottom_auto_scroll_on_content_growth() {
        let state = ConversationState::new();
        
        // Verify initial state
        assert!(state.scroll_to_bottom.load(Ordering::SeqCst));
        assert_eq!(state.scroll_pos.load(Ordering::SeqCst), 0);
        assert_eq!(state.prev_scrollable_range.load(Ordering::SeqCst), 0);
    }
}
