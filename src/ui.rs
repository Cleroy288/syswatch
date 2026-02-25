use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Axis, Block, Borders, Chart, Dataset, GraphType, Paragraph, Row, Table,
};

use crate::app::App;

// ── Main layout ─────────────────────────────────────────────

pub fn draw(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(12), // top panel
            Constraint::Min(8),    // process table
        ])
        .split(f.area());

    draw_top_panel(f, app, chunks[0]);
    draw_process_table(f, app, chunks[1]);
}

// ── Top panel: stats │ chart │ counts ───────────────────────

fn draw_top_panel(f: &mut Frame, app: &mut App, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(24),
            Constraint::Min(30),
            Constraint::Length(24),
        ])
        .split(area);

    draw_cpu_stats(f, app, cols[0]);
    draw_cpu_chart(f, app, cols[1]);
    draw_system_counts(f, app, cols[2]);
}

fn draw_cpu_stats(f: &mut Frame, app: &App, area: Rect) {
    let text = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  System:  "),
            Span::styled(
                format!("{:>6.2}%", app.system_pct),
                Style::default().fg(Color::Red),
            ),
        ]),
        Line::from("  ─────────────────"),
        Line::from(vec![
            Span::raw("  User:    "),
            Span::styled(
                format!("{:>6.2}%", app.user_pct),
                Style::default().fg(Color::Cyan),
            ),
        ]),
        Line::from("  ─────────────────"),
        Line::from(vec![
            Span::raw("  Idle:    "),
            Span::styled(
                format!("{:>6.2}%", app.idle_pct),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
    ];

    let widget = Paragraph::new(text).block(bordered(""));
    f.render_widget(widget, area);
}

fn draw_cpu_chart(f: &mut Frame, app: &App, area: Rect) {
    let sys_data: Vec<(f64, f64)> = app.system_history.iter().copied().collect();
    let usr_data: Vec<(f64, f64)> = app.user_history.iter().copied().collect();

    let datasets = vec![
        Dataset::default()
            .name("System")
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(Color::Red))
            .data(&sys_data),
        Dataset::default()
            .name("User")
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(Color::Cyan))
            .data(&usr_data),
    ];

    let bounds = app.history_bounds();

    let x_axis = Axis::default()
        .style(Style::default().fg(Color::DarkGray))
        .bounds(bounds);

    let y_axis = Axis::default()
        .style(Style::default().fg(Color::DarkGray))
        .bounds([0.0, 100.0])
        .labels(["0%", "50%", "100%"]);

    let chart = Chart::new(datasets)
        .block(bordered(" CPU LOAD ").title_alignment(Alignment::Center))
        .x_axis(x_axis)
        .y_axis(y_axis);

    f.render_widget(chart, area);
}

fn draw_system_counts(f: &mut Frame, app: &App, area: Rect) {
    let used_gb = app.used_memory as f64 / 1_073_741_824.0;
    let total_gb = app.total_memory as f64 / 1_073_741_824.0;

    let text = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  Threads:    "),
            Span::styled(
                format!("{:>6}", fmt_thousands(app.thread_count)),
                Style::default().fg(Color::Magenta),
            ),
        ]),
        Line::from("  ─────────────────"),
        Line::from(vec![
            Span::raw("  Processes:  "),
            Span::styled(
                format!("{:>6}", fmt_thousands(app.processes.len())),
                Style::default().fg(Color::Green),
            ),
        ]),
        Line::from("  ─────────────────"),
        Line::from(vec![
            Span::raw("  Memory:     "),
            Span::styled(
                format!("{used_gb:.1}/{total_gb:.0}G"),
                Style::default().fg(mem_color(used_gb, total_gb)),
            ),
        ]),
    ];

    let widget = Paragraph::new(text).block(bordered(""));
    f.render_widget(widget, area);
}

// ── Process table ───────────────────────────────────────────

fn draw_process_table(f: &mut Frame, app: &mut App, area: Rect) {
    let header = Row::new(vec!["PID", "Process", "CPU %", "Memory"])
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .bottom_margin(1);

    let rows: Vec<Row> = app
        .processes
        .iter()
        .map(|p| {
            let cpu_style = if p.cpu_usage > 50.0 {
                Style::default().fg(Color::Red)
            } else if p.cpu_usage > 10.0 {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            };

            Row::new(vec![
                p.pid.to_string(),
                p.name.clone(),
                format!("{:.1}", p.cpu_usage),
                fmt_bytes(p.memory),
            ])
            .style(cpu_style)
        })
        .collect();

    let widths = [
        Constraint::Length(8),
        Constraint::Min(20),
        Constraint::Length(10),
        Constraint::Length(12),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            bordered(" Processes ")
                .title_bottom(Line::from(" q: quit  j/k ↑/↓: scroll ").right_aligned()),
        )
        .row_highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol("▶ ");

    f.render_stateful_widget(table, area, &mut app.table_state);
}

// ── Helpers ─────────────────────────────────────────────────

fn bordered(title: &str) -> Block<'_> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
}

fn fmt_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * 1024 * 1024;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

fn fmt_thousands(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        format!("{n}")
    }
}

fn mem_color(used: f64, total: f64) -> Color {
    if total <= 0.0 {
        return Color::White;
    }
    let pct = used / total * 100.0;
    match pct as u32 {
        0..=60 => Color::Green,
        61..=85 => Color::Yellow,
        _ => Color::Red,
    }
}
