//! Syswatch — a macOS system monitor with two frontends:
//!   * `--tui`  : ratatui terminal UI
//!   * `--gpui` : native GPU window rendered by `iced`
//!
//! Exactly one flag is required; otherwise usage is printed.

mod app;
mod format;
mod gui;
mod tui;

use std::process::ExitCode;
use std::time::Duration;

use clap::{ArgGroup, Parser};

/// Default refresh interval in seconds (2 s balances responsiveness and CPU cost).
const DEFAULT_INTERVAL_SECS: u64 = 2;
const MIN_INTERVAL_SECS: u64 = 1;

#[derive(Parser)]
#[command(name = "syswatch", about = "macOS system monitor", version)]
#[command(group(ArgGroup::new("mode").required(true).args(["tui", "gpui"])))]
struct Cli {
    /// Render the dashboard in the terminal (ratatui).
    #[arg(long)]
    tui: bool,
    /// Render the dashboard in a native GPU-accelerated window (iced).
    #[arg(long)]
    gpui: bool,
    /// Refresh interval in seconds (min 1, default 2).
    #[arg(long, default_value_t = DEFAULT_INTERVAL_SECS)]
    interval: u64,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let interval = Duration::from_secs(cli.interval.max(MIN_INTERVAL_SECS));

    let result = if cli.gpui {
        gui::run(interval).map_err(|e| e.to_string())
    } else {
        tui::run(interval).map_err(|e| e.to_string())
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("syswatch: {err}");
            ExitCode::FAILURE
        }
    }
}
