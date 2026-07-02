//! Subagent detail sidebar widget — renders a vertical sidebar with a list
//! of subagents and a detail view for the selected one.

use crate::shared::SubagentInfo;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

// ─── Subagent list ────────────────────────────────────────────────────────

/// Renders the subagent list section of the sidebar.
pub fn render_subagent_list(
    frame: &mut Frame,
    area: Rect,
    subagents: &[SubagentInfo],
    selected_idx: Option<usize>,
) {
    let title = Line::from(vec![
        Span::styled("\u{25c6}", Style::default().fg(Color::Cyan).bold()),
        Span::raw(" Subagents"),
        Span::styled(
            format!(" ({})", subagents.len()),
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    frame.render_widget(block, area);
    let inner = area.inner(ratatui::layout::Margin::default());
    if inner.height == 0 {
        return;
    }

    let items: Vec<ListItem> = subagents
        .iter()
        .enumerate()
        .map(|(i, sub)| {
            let (icon, color) = match sub.status.as_str() {
                "running" => ("\u{00b7}", Color::Yellow),
                "completed" => ("*", Color::Green),
                "error" | "interrupted" => ("x", Color::Red),
                _ => ("o", Color::Gray),
            };

            let goal = if sub.goal.chars().count() > 25 {
                format!("{}...", sub.goal.chars().take(25).collect::<String>())
            } else {
                sub.goal.clone()
            };

            let parts = vec![
                Span::styled(icon, Style::default().fg(color)),
                Span::raw(" #"),
                Span::styled(format!("{}", sub.delegation_id), Style::default().fg(color)),
                Span::styled(
                    format!(" {}", sub.status),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw(" "),
                Span::styled(
                    goal,
                    Style::default().fg(Color::White).add_modifier(Modifier::DIM),
                ),
            ];

            let style = if selected_idx == Some(i) {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            ListItem::new(Line::from(parts).patch_style(style))
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

// ─── Subagent detail ──────────────────────────────────────────────────────

/// Renders the detail view for a single subagent.
pub fn render_subagent_detail(
    frame: &mut Frame,
    area: Rect,
    subagent: &SubagentInfo,
) {
    let status_color = match subagent.status.as_str().to_lowercase().as_str() {
        "running" => Color::Yellow,
        "completed" => Color::Green,
        "error" | "interrupted" => Color::Red,
        _ => Color::Gray,
    };

    let status_label = match subagent.status.as_str().to_lowercase().as_str() {
        "running" => "[running]",
        "completed" => "[done]",
        "error" | "interrupted" => "[failed]",
        _ => "[?]",
    };

    let goal_short: String = subagent.goal.chars().take(40).collect();
    let header = Line::styled(
        format!(
            " {} #{} -- {}",
            status_label, subagent.delegation_id, goal_short
        ),
        Style::default()
            .fg(status_color)
            .add_modifier(Modifier::BOLD),
    );

    let block = Block::default()
        .title(header)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(status_color));

    let body = block.inner(area);
    frame.render_widget(&block, area);

    let sections = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(4),
        Constraint::Length(4),
        Constraint::Length(4),
        Constraint::Min(0),
    ])
    .split(body);

    // --- Summary (0) ---
    render_paragraph(
        frame,
        sections[0],
        &subagent.summary,
        Style::default().fg(Color::DarkGray),
    );

    // --- Thinking (1) ---
    render_thinking_section(frame, sections[1], subagent);

    // --- Tool calls (2) ---
    render_tools_section(frame, sections[2], subagent);

    // --- Results (3) ---
    if !subagent.summary.is_empty() && subagent.summary != subagent.status {
        let rb = Block::default()
            .borders(Borders::TOP)
            .title(Line::styled(" Summary", Style::default().fg(Color::Green)));
        let inner = rb.inner(sections[3]);
        frame.render_widget(&rb, sections[3]);
        let summary_short: String = if subagent.summary.chars().count() > 200 {
            subagent.summary.chars().take(200).collect::<String>()
        } else {
            subagent.summary.clone()
        };
        frame.render_widget(Paragraph::new(summary_short), inner);
    } else {
        let rb = Block::default()
            .borders(Borders::TOP)
            .title(Line::styled(" Summary", Style::default().fg(Color::Green)));
        frame.render_widget(&rb, sections[3]);
    }

    // --- Details (4) ---
    let remaining = sections[4];
    if !remaining.is_empty() {
        let mut parts: Vec<String> = Vec::new();
        if let Some(ref start) = subagent.start_time {
            parts.push(format!("start: {}", start));
        }
        if let Some(ref end) = subagent.end_time {
            parts.push(format!("end: {}", end));
        }
        if !subagent.stats.duration.is_empty() {
            parts.push(format!("duration: {}", subagent.stats.duration));
        }
        if subagent.stats.token_count > 0 {
            parts.push(format!("{} tokens", subagent.stats.token_count));
        }
        if subagent.stats.cost_usd > 0.0 {
            parts.push(format!("${:.4}", subagent.stats.cost_usd));
        }
        if !subagent.parent_session_id.is_empty() {
            parts.push(format!(
                "parent: {}",
                subagent.parent_session_id.chars().take(16).collect::<String>()
            ));
        }

        if !parts.is_empty() {
            let db = Block::default()
                .borders(Borders::TOP)
                .title(Line::styled(" Details", Style::default().fg(Color::Blue)));
            let inner = db.inner(remaining);
            frame.render_widget(&db, remaining);
            let info: String = parts.join("  |  ");
            let info_short: String = if info.chars().count() > 60 {
                format!("{}...", info.chars().take(60).collect::<String>())
            } else {
                info
            };
            frame.render_widget(
                Paragraph::new(Line::styled(info_short, Style::default().fg(Color::Gray))),
                inner,
            );
        }
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────

fn render_paragraph(frame: &mut Frame, area: Rect, text: &str, style: Style) {
    frame.render_widget(Paragraph::new(Line::styled(text, style)), area);
}

fn render_thinking_section(frame: &mut Frame, area: Rect, subagent: &SubagentInfo) {
    let block = Block::default()
        .title(Line::styled(
            "\u{1f4ad} Thinking",
            Style::default().fg(Color::DarkGray),
        ));
    let inner = block.inner(area);
    frame.render_widget(&block, area);

    if let Some(ref thinking) = subagent.thinking {
        if !thinking.is_empty() {
            let lines: Vec<Line> = thinking
                .iter()
                .take(3)
                .map(|sl| {
                    Line::styled(
                        sl.content.to_string(),
                        Style::default().fg(Color::Gray).add_modifier(Modifier::DIM),
                    )
                })
                .collect();
            frame.render_widget(Paragraph::new(lines), inner);
        }
    }
}

fn render_tools_section(frame: &mut Frame, area: Rect, subagent: &SubagentInfo) {
    let block = Block::default()
        .title(Line::styled("\u{1f527} Tools", Style::default().fg(Color::DarkGray)));
    let inner = block.inner(area);
    frame.render_widget(&block, area);

    if !subagent.tool_calls.is_empty() {
        let tool_lines: Vec<Line> = subagent
            .tool_calls
            .iter()
            .take(3)
            .map(|tc| {
                Line::styled(format!("    \u{25cf} {}", tc), Style::default().fg(Color::Yellow))
            })
            .collect();
        frame.render_widget(Paragraph::new(tool_lines), inner);
    }
}
