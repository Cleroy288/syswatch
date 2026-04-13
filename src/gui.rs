//! Native-looking GPU UI backend for syswatch, built on `iced`.
//!
//! Mirrors the TUI layout (CPU stats | chart | counts + scrollable
//! process list) but rendered via `iced` and wgpu so the same binary
//! ships a lightweight GPU frontend without platform-specific shaders.

use std::time::Duration;

use iced::event::{self, Event};
use iced::mouse;
use iced::time;
use iced::widget::canvas::{self, Cache, Frame, Geometry, Path, Stroke};
use iced::widget::{Canvas, column, container, row, scrollable, text};
use iced::window;
use iced::{
    Background, Border, Color, Element, Fill, Length, Point, Rectangle, Renderer, Shadow,
    Subscription, Task, Theme,
};

use crate::app::{App as SysApp, ProcessInfo};
use crate::format::{
    BYTES_PER_GIB, CpuLevel, MemoryPalette, cpu_level, fmt_bytes, fmt_thousands, mem_palette,
};

const MAX_VISIBLE_ROWS: usize = 100;

// ── Sober palette (single accent, muted neutrals) ───────────

/// Panel background (slightly lifted above the window surface).
const PANEL_BG: Color = Color::from_rgb(0.11, 0.12, 0.14);
/// Panel border.
const BORDER: Color = Color::from_rgb(0.20, 0.22, 0.25);
/// Primary foreground.
const FG: Color = Color::from_rgb(0.88, 0.89, 0.91);
/// Muted secondary foreground (labels, axis, headers).
const MUTED: Color = Color::from_rgb(0.55, 0.57, 0.60);
/// Subtle tertiary foreground (inactive axis, faded text).
const FAINT: Color = Color::from_rgb(0.38, 0.40, 0.44);
/// Single accent tone (chart series, selection).
const ACCENT: Color = Color::from_rgb(0.42, 0.72, 0.92);
/// Warning tone (crossed mid threshold).
const WARN: Color = Color::from_rgb(0.92, 0.72, 0.33);
/// Danger tone (crossed upper threshold).
const DANGER: Color = Color::from_rgb(0.92, 0.40, 0.38);

const PANEL_RADIUS: f32 = 8.0;
const PANEL_PADDING: u16 = 14;

// ── Entry point ─────────────────────────────────────────────

pub fn run(interval: Duration) -> iced::Result {
    iced::application(move || init(interval), update, view)
        .title("syswatch")
        .theme(theme)
        .subscription(subscription)
        .run()
}

fn theme(_state: &State) -> Theme {
    Theme::Dark
}

fn init(interval: Duration) -> (State, Task<Message>) {
    let mut app = SysApp::new();
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    app.tick();
    (
        State {
            app,
            chart_cache: Cache::new(),
            focused: true,
            interval,
        },
        Task::none(),
    )
}

// ── State & messages ────────────────────────────────────────

struct State {
    app: SysApp,
    chart_cache: Cache,
    focused: bool,
    interval: Duration,
}

#[derive(Debug, Clone, Copy)]
enum Message {
    Tick,
    FocusChanged(bool),
}

fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::Tick => {
            state.app.tick();
            state.chart_cache.clear();
        }
        Message::FocusChanged(focused) => {
            state.focused = focused;
        }
    }
    Task::none()
}

fn subscription(state: &State) -> Subscription<Message> {
    let focus_events = event::listen_with(|event, _status, _id| match event {
        Event::Window(window::Event::Focused) => Some(Message::FocusChanged(true)),
        Event::Window(window::Event::Unfocused) => Some(Message::FocusChanged(false)),
        _ => None,
    });

    let ticks = if state.focused {
        time::every(state.interval).map(|_| Message::Tick)
    } else {
        Subscription::none()
    };

    Subscription::batch([ticks, focus_events])
}

// ── View ────────────────────────────────────────────────────

fn view(state: &State) -> Element<'_, Message> {
    let top = row![
        cpu_stats_panel(&state.app),
        cpu_chart_panel(state),
        counts_panel(&state.app),
    ]
    .spacing(12)
    .height(Length::Fixed(170.0));

    column![top, process_panel(&state.app)]
        .spacing(12)
        .padding(16)
        .into()
}

// ── Top panel ───────────────────────────────────────────────

fn cpu_stats_panel(app: &SysApp) -> Element<'_, Message> {
    panel(
        "CPU",
        column![
            stat_row("System", format!("{:>6.2}%", app.system_pct), FG),
            stat_row("User",   format!("{:>6.2}%", app.user_pct),   FG),
            stat_row("Idle",   format!("{:>6.2}%", app.idle_pct),   MUTED),
        ]
        .spacing(10)
        .into(),
    )
    .width(Length::Fixed(240.0))
    .into()
}

fn cpu_chart_panel(state: &State) -> Element<'_, Message> {
    let chart = Chart {
        cache: &state.chart_cache,
        system: state.app.system_slice(),
        user: state.app.user_slice(),
        bounds_x: state.app.history_bounds(),
    };

    panel(
        "Load",
        column![
            Canvas::new(chart).width(Fill).height(Fill),
            row![legend_dot("System", FAINT), legend_dot("User", ACCENT)]
                .spacing(16),
        ]
        .spacing(8)
        .into(),
    )
    .width(Length::Fill)
    .into()
}

fn counts_panel(app: &SysApp) -> Element<'_, Message> {
    let used_gb = app.used_memory as f64 / BYTES_PER_GIB;
    let total_gb = app.total_memory as f64 / BYTES_PER_GIB;

    panel(
        "System",
        column![
            stat_row("Threads",   fmt_thousands(app.thread_count),    FG),
            stat_row("Processes", fmt_thousands(app.processes.len()), FG),
            stat_row(
                "Memory",
                format!("{used_gb:.1} / {total_gb:.0} GB"),
                mem_color(used_gb, total_gb),
            ),
        ]
        .spacing(10)
        .into(),
    )
    .width(Length::Fixed(240.0))
    .into()
}

// ── Process panel ───────────────────────────────────────────

fn process_panel(app: &SysApp) -> Element<'_, Message> {
    let rows = app
        .processes
        .iter()
        .take(MAX_VISIBLE_ROWS)
        .map(process_row)
        .collect::<Vec<_>>();

    panel(
        "Processes",
        column![
            process_header(),
            scrollable(column(rows).spacing(2)).height(Length::Fill),
        ]
        .spacing(8)
        .into(),
    )
    .width(Fill)
    .height(Length::Fill)
    .into()
}

fn process_header() -> Element<'static, Message> {
    row![
        header_cell("PID", Length::Fixed(72.0)),
        header_cell("Process", Length::Fill),
        header_cell("CPU %", Length::Fixed(80.0)),
        header_cell("Memory", Length::Fixed(100.0)),
    ]
    .padding(iced::Padding::from([2, 6]))
    .into()
}

fn header_cell(label: &'static str, width: Length) -> Element<'static, Message> {
    text(label).size(11).color(MUTED).width(width).into()
}

fn process_row(p: &ProcessInfo) -> Element<'_, Message> {
    row![
        text(p.pid.to_string())
            .size(13)
            .color(MUTED)
            .width(Length::Fixed(72.0)),
        text(&p.name).size(13).color(FG).width(Length::Fill),
        text(format!("{:.1}", p.cpu_usage))
            .size(13)
            .color(cpu_color(p.cpu_usage))
            .width(Length::Fixed(80.0)),
        text(fmt_bytes(p.memory))
            .size(13)
            .color(MUTED)
            .width(Length::Fixed(100.0)),
    ]
    .padding(iced::Padding::from([3, 6]))
    .into()
}

// ── Reusable fragments ──────────────────────────────────────

/// A "label   value" row with a muted label on the left and a value on the right.
fn stat_row<'a>(
    label: &'static str,
    value: String,
    value_color: Color,
) -> Element<'a, Message> {
    row![
        text(label).size(12).color(MUTED).width(Length::Fill),
        text(value).size(14).color(value_color),
    ]
    .align_y(iced::Alignment::Center)
    .into()
}

/// Creates a sober rounded panel with a muted uppercase-style title overlay.
fn panel<'a>(title: &'static str, body: Element<'a, Message>) -> container::Container<'a, Message> {
    let content = column![
        text(title).size(11).color(MUTED),
        body,
    ]
    .spacing(10);

    container(content)
        .padding(PANEL_PADDING)
        .style(|_| container::Style {
            background: Some(Background::Color(PANEL_BG)),
            border: Border {
                color: BORDER,
                width: 1.0,
                radius: PANEL_RADIUS.into(),
            },
            shadow: Shadow::default(),
            ..Default::default()
        })
}

fn legend_dot<'a>(label: &'static str, color: Color) -> Element<'a, Message> {
    row![
        container(text(""))
            .width(Length::Fixed(6.0))
            .height(Length::Fixed(6.0))
            .style(move |_| container::Style {
                background: Some(Background::Color(color)),
                border: Border {
                    radius: 3.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
        text(label).size(11).color(MUTED),
    ]
    .spacing(6)
    .align_y(iced::Alignment::Center)
    .into()
}

// ── Canvas-based CPU chart ──────────────────────────────────

struct Chart<'a> {
    cache: &'a Cache,
    system: &'a [(f64, f64)],
    user: &'a [(f64, f64)],
    bounds_x: [f64; 2],
}

impl<'a> canvas::Program<Message> for Chart<'a> {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let geometry = self.cache.draw(renderer, bounds.size(), |frame| {
            draw_grid(frame, bounds);
            draw_series(frame, bounds, self.system, self.bounds_x, FAINT);
            draw_series(frame, bounds, self.user, self.bounds_x, ACCENT);
        });
        vec![geometry]
    }
}

fn draw_grid(frame: &mut Frame, bounds: Rectangle) {
    let grid = Color::from_rgba(1.0, 1.0, 1.0, 0.05);
    for ratio in [0.0, 0.5, 1.0] {
        let y = bounds.height * ratio;
        let path = Path::line(Point::new(0.0, y), Point::new(bounds.width, y));
        frame.stroke(&path, Stroke::default().with_width(1.0).with_color(grid));
    }
}

fn draw_series(
    frame: &mut Frame,
    bounds: Rectangle,
    samples: &[(f64, f64)],
    bounds_x: [f64; 2],
    color: Color,
) {
    if samples.len() < 2 {
        return;
    }

    let x_span = (bounds_x[1] - bounds_x[0]).max(1.0);

    let path = Path::new(|builder| {
        for (i, (tick, value)) in samples.iter().enumerate() {
            let x = ((*tick - bounds_x[0]) / x_span).clamp(0.0, 1.0) as f32
                * bounds.width;
            let y = (1.0 - (*value / 100.0).clamp(0.0, 1.0) as f32)
                * bounds.height;
            let point = Point::new(x, y);
            if i == 0 {
                builder.move_to(point);
            } else {
                builder.line_to(point);
            }
        }
    });

    frame.stroke(
        &path,
        Stroke::default().with_width(1.5).with_color(color),
    );
}

// ── Palette mapping ─────────────────────────────────────────

fn cpu_color(cpu: f32) -> Color {
    match cpu_level(cpu) {
        CpuLevel::Hot => DANGER,
        CpuLevel::Warm => WARN,
        CpuLevel::Cool => FG,
    }
}

fn mem_color(used: f64, total: f64) -> Color {
    match mem_palette(used, total) {
        MemoryPalette::Ok => FG,
        MemoryPalette::Warn => WARN,
        MemoryPalette::Hot => DANGER,
        MemoryPalette::Unknown => MUTED,
    }
}
