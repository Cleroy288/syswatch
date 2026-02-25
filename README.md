# syswatch

A lightweight terminal system monitor for macOS, built in Rust with [ratatui](https://github.com/ratatui/ratatui).

![syswatch](screenshot.png)

## Features

- **CPU Load** — real-time chart with system (red) and user (cyan) split, 3-minute sliding window
- **System stats** — system/user/idle CPU percentages, thread count, process count, memory usage
- **Process table** — all processes sorted by CPU usage, scrollable with keyboard
- **Lightweight** — ~10 MB RSS vs ~90 MB for Activity Monitor

## Install

```sh
git clone git@github.com:Cleroy288/syswatch.git
cd syswatch
cargo build --release
```

Optionally add an alias to your shell:

```sh
echo 'alias sysmonitor="/path/to/syswatch/target/release/syswatch"' >> ~/.zshrc
```

## Usage

```sh
cargo run --release
# or, if aliased:
sysmonitor
```

| Key | Action |
|-----|--------|
| `q` / `Esc` | Quit |
| `j` / `Down` | Scroll down |
| `k` / `Up` | Scroll up |

## Tech

- **Rust** — fast, safe, no garbage collector
- **ratatui** — renders the UI in the terminal (charts, tables, gauges)
- **crossterm** — captures keyboard input and controls the terminal
- **sysinfo** — reads process list, CPU usage, and memory stats
- **macOS mach API** — gets the system vs user CPU split directly from the kernel
- **macOS libproc API** — counts threads per process (same source as Activity Monitor)

## Requirements

- macOS (uses Darwin-specific APIs)
- Rust 1.85+

## License

MIT
