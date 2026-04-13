//! Terminal UI backend for syswatch — owns the event loop and ratatui drawing.

use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::DefaultTerminal;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Axis, Block, BorderType, Borders, Chart, Dataset, GraphType, Paragraph, Row, Table,
};

use crate::app::App;
use crate::format::{
    BYTES_PER_GIB, CpuLevel, MemoryPalette, cpu_level, fmt_bytes, fmt_thousands, mem_palette,
};

// ── Sober palette ───────────────────────────────────────────

/// Primary foreground colour for values and active content.
const FG: Color = Color::Gray;
/// Muted colour for labels, borders, axis marks.
const MUTED: Color = Color::DarkGray;
/// Single accent used for the live series and highlights.
const ACCENT: Color = Color::Cyan;
/// Warning tone used only when a threshold is crossed.
const WARN: Color = Color::Yellow;
/// Danger tone used only when a threshold is crossed.
const DANGER: Color = Color::Red;

/// Entry point for the TUI backend.
pub fn run(tick_rate: Duration) -> io::Result<()> {
    let mut terminal = ratatui::init();
    let result = run_loop(&mut terminal, tick_rate);
    ratatui::restore();
    result
}

fn run_loop(terminal: &mut DefaultTerminal, tick_rate: Duration) -> io::Result<()> {
    let mut app = App::new();
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    app.tick();

    let mut last_tick = Instant::now();

    while app.running {
        terminal.draw(|f| draw(f, &mut app))?;

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            handle_key(&mut app, key.code);
        }

        if last_tick.elapsed() >= tick_rate {
            app.tick();
            last_tick = Instant::now();
        }
    }

    Ok(())
}

fn handle_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => app.running = false,
        KeyCode::Down | KeyCode::Char('j') => app.select_process(1),
        KeyCode::Up | KeyCode::Char('k') => app.select_process(-1),
        _ => {}
    }
}

// ── Main layout ─────────────────────────────────────────────

/// Draws the complete UI: top metrics panel and process table.
fn draw(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(10),
            Constraint::Min(8),
        ])
        .split(f.area());

    draw_top_panel(f, app, chunks[0]);
    draw_process_table(f, app, chunks[1]);
}

// ── Top panel: stats | chart | counts ───────────────────────

/// Renders the three-column header: CPU stats, CPU chart, system counts.
fn draw_top_panel(f: &mut Frame, app: &App, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(26),
            Constraint::Min(30),
            Constraint::Length(26),
        ])
        .spacing(1)
        .split(area);

    draw_cpu_stats(f, app, cols[0]);
    draw_cpu_chart(f, app, cols[1]);
    draw_system_counts(f, app, cols[2]);
}

/// Renders the System / User / Idle percentage column.
fn draw_cpu_stats(f: &mut Frame, app: &App, area: Rect) {
    let lines = vec![
        Line::from(""),
        stat_line("System", format!("{:>6.2}%", app.system_pct), FG),
        Line::from(""),
        stat_line("User",   format!("{:>6.2}%", app.user_pct),   FG),
        Line::from(""),
        stat_line("Idle",   format!("{:>6.2}%", app.idle_pct),   MUTED),
    ];

    let widget = Paragraph::new(lines).block(panel("CPU"));
    f.render_widget(widget, area);
}

/// Builds a "label    value" line with a muted label and a coloured value.
fn stat_line(label: &'static str, value: String, value_color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {label:<9}"), Style::default().fg(MUTED)),
        Span::styled(value, Style::default().fg(value_color)),
    ])
}

/// Renders the live CPU-load chart with system and user datasets.
fn draw_cpu_chart(f: &mut Frame, app: &App, area: Rect) {
    let datasets = vec![
        Dataset::default()
            .name("System")
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(MUTED))
            .data(app.system_slice()),
        Dataset::default()
            .name("User")
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(ACCENT))
            .data(app.user_slice()),
    ];

    let axis_style = Style::default().fg(MUTED);

    let chart = Chart::new(datasets)
        .block(panel("Load").title_alignment(Alignment::Center))
        .x_axis(Axis::default().style(axis_style).bounds(app.history_bounds()))
        .y_axis(
            Axis::default()
                .style(axis_style)
                .bounds([0.0, 100.0])
                .labels(["0", "50", "100"]),
        );

    f.render_widget(chart, area);
}

/// Renders the Threads / Processes / Memory column.
fn draw_system_counts(f: &mut Frame, app: &App, area: Rect) {
    let used_gb = app.used_memory as f64 / BYTES_PER_GIB;
    let total_gb = app.total_memory as f64 / BYTES_PER_GIB;

    let lines = vec![
        Line::from(""),
        stat_line("Threads",   format!("{:>8}", fmt_thousands(app.thread_count)),    FG),
        Line::from(""),
        stat_line("Processes", format!("{:>8}", fmt_thousands(app.processes.len())), FG),
        Line::from(""),
        stat_line("Memory",    format!("{used_gb:>4.1} / {total_gb:.0} GB"),         mem_color(used_gb, total_gb)),
    ];

    let widget = Paragraph::new(lines).block(panel("System"));
    f.render_widget(widget, area);
}

// ── Process table ───────────────────────────────────────────

/// Renders the scrollable, sortable process table.
fn draw_process_table(f: &mut Frame, app: &mut App, area: Rect) {
    let header = Row::new(["PID", "Process", "CPU %", "Memory"])
        .style(Style::default().fg(MUTED))
        .bottom_margin(1);

    let rows: Vec<Row> = app
        .processes
        .iter()
        .map(|p| {
            Row::new([
                p.pid.to_string(),
                p.name.clone(),
                format!("{:.1}", p.cpu_usage),
                fmt_bytes(p.memory),
            ])
            .style(cpu_row_style(p.cpu_usage))
        })
        .collect();

    let widths = [
        Constraint::Length(8),
        Constraint::Min(20),
        Constraint::Length(10),
        Constraint::Length(12),
    ];

    let footer = Line::from(Span::styled(
        " q quit · j/k scroll ",
        Style::default().fg(MUTED),
    ))
    .right_aligned();

    let table = Table::new(rows, widths)
        .header(header)
        .block(panel("Processes").title_bottom(footer))
        .row_highlight_style(
            Style::default()
                .fg(ACCENT)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("› ");

    f.render_stateful_widget(table, area, &mut app.table_state);
}

// ── Helpers ─────────────────────────────────────────────────

/// Creates a sober bordered block with a muted title.
fn panel(title: &'static str) -> Block<'static> {
    Block::default()
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(MUTED),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(MUTED))
}

/// Picks a colour for the memory reading based on usage percentage.
fn mem_color(used: f64, total: f64) -> Color {
    match mem_palette(used, total) {
        MemoryPalette::Ok => FG,
        MemoryPalette::Warn => WARN,
        MemoryPalette::Hot => DANGER,
        MemoryPalette::Unknown => MUTED,
    }
}

/// Picks a row style for a process based on its CPU usage.
fn cpu_row_style(cpu: f32) -> Style {
    match cpu_level(cpu) {
        CpuLevel::Hot => Style::default().fg(DANGER),
        CpuLevel::Warm => Style::default().fg(WARN),
        CpuLevel::Cool => Style::default().fg(FG),
    }
}
