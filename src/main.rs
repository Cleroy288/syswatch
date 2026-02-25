mod app;
mod ui;

use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::DefaultTerminal;

use app::App;

const TICK_RATE: Duration = Duration::from_secs(1);

fn main() -> io::Result<()> {
    let mut terminal = ratatui::init();
    let result = run(&mut terminal);
    ratatui::restore();
    result
}

fn run(terminal: &mut DefaultTerminal) -> io::Result<()> {
    let mut app = App::new();
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    app.tick();

    let mut last_tick = Instant::now();

    while app.running {
        terminal.draw(|f| ui::draw(f, &mut app))?;

        let timeout = TICK_RATE.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    handle_key(&mut app, key.code);
                }
            }
        }

        if last_tick.elapsed() >= TICK_RATE {
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
