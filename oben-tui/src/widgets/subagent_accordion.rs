//! Subagent accordion — renders SubagentInfo as collapsible blocks.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::shared::SubagentInfo;

/// Expands and collapses subagent blocks based on delegation_id.
pub struct SubagentAccordion {
    expanded: std::collections::HashSet<u32>,
}

impl SubagentAccordion {
    pub fn new() -> Self {
        Self {
            expanded: std::collections::HashSet::new(),
        }
    }

    /// Toggle expansion state for a subagent by delegation_id.
    pub fn toggle(&mut self, delegation_id: u32) {
        if self.expanded.contains(&delegation_id) {
            self.expanded.remove(&delegation_id);
        } else {
            self.expanded.insert(delegation_id);
        }
    }

    /// Check if a subagent is expanded.
    pub fn is_expanded(&self, delegation_id: u32) -> bool {
        self.expanded.contains(&delegation_id)
    }

    /// Render subagents as collapsible accordion blocks.
    ///
    /// Called within a pre-allocated `area` (set by the parent ChatPanel layout).
    pub fn render(&self, frame: &mut Frame, area: Rect, subagents: &[SubagentInfo]) {
        if subagents.is_empty() {
            return;
        }

        let count = subagents.len();
        let header = Line::from(vec![
            Span::styled("◆", Style::default().fg(Color::Cyan).bold()),
            Span::raw(" Subagent"),
            Span::styled(
                format!(" ({count})"),
                Style::default().fg(Color::DarkGray),
            ),
        ]);

        let block = Block::default()
            .title(header)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray));

        frame.render_widget(block, area);

        // Build lines for each subagent
        let mut lines: Vec<Line> = Vec::new();

        for sub in subagents.iter() {
            let is_expanded = self.is_expanded(sub.delegation_id);

            // Status row
            let status_color = match sub.status.as_str() {
                "running" => Color::Yellow,
                "completed" => Color::Green,
                "error" => Color::Red,
                _ => Color::Gray,
            };

            let status_marker = match sub.status.as_str() {
                "running" => "[running]",
                "completed" => "[done]",
                "error" => "[failed]",
                _ => "[?]",
            };

            lines.push(Line::from(vec![
                Span::styled(if is_expanded { "▾" } else { "▸" }, Style::default().fg(Color::DarkGray)),
                Span::styled(format!(" {status_marker} #{} ", sub.delegation_id), Style::default().fg(status_color)),
            ]));

            // Goal row
            let goal = if sub.goal.chars().count() > 60 {
                format!("{}...", sub.goal.chars().take(60).collect::<String>())
            } else {
                sub.goal.clone()
            };
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(goal, Style::default().fg(Color::White).add_modifier(Modifier::DIM)),
            ]));

            // Expanded content
            if is_expanded {
                if let Some(ref t) = sub.start_time {
                    lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled("start: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(t, Style::default().fg(Color::Blue)),
                    ]));
                }
                if let Some(ref t) = sub.end_time {
                    lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled("end: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(t, Style::default().fg(Color::Blue)),
                    ]));
                }
                if !sub.summary.is_empty() {
                    let resp = if sub.summary.chars().count() > 100 {
                        format!("{}...", sub.summary.chars().take(100).collect::<String>())
                    } else {
                        sub.summary.clone()
                    };
                    lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled("response: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(resp, Style::default().fg(Color::Gray)),
                    ]));
                }
                if !sub.children.is_empty() {
                    lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(
                            format!("({} child subagent(s))", sub.children.len()),
                            Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
                        ),
                    ]));
                }
                // Child subagents
                for child in &sub.children {
                    let child_is_expanded = self.is_expanded(child.delegation_id);
                    lines.push(Line::from(vec![
                        Span::raw("    "),
                        Span::styled(if child_is_expanded { "▾" } else { "▸" }, Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM)),
                        Span::styled(" [child] ", Style::default().fg(Color::Blue).add_modifier(Modifier::DIM)),
                        Span::styled(child.goal.chars().take(40).collect::<String>(), Style::default().fg(Color::Cyan)),
                    ]));
                }
            }
        }

        let paragraph = Paragraph::new(lines);
        let inner = area.inner(ratatui::layout::Margin::default());
        if !inner.is_empty() {
            frame.render_widget(paragraph, inner);
        }
    }
}

impl Default for SubagentAccordion {
    fn default() -> Self {
        Self::new()
    }
}
