//! Shared formatters and byte-size constants for the TUI / GUI backends.

pub const BYTES_PER_GIB: f64 = 1_073_741_824.0;

pub const KB: u64 = 1024;
pub const MB: u64 = 1024 * 1024;
pub const GB: u64 = 1024 * 1024 * 1024;

/// Thresholds for [`MemoryPalette`] selection.
const MEM_GREEN_MAX: u32 = 60;
const MEM_YELLOW_MAX: u32 = 85;

/// CPU-usage thresholds for [`CpuLevel`] selection.
const CPU_HOT: f32 = 50.0;
const CPU_WARM: f32 = 10.0;

/// Formats a byte count into a human-readable string (B / KB / MB / GB).
pub fn fmt_bytes(bytes: u64) -> String {
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

/// Formats a count with K / M suffixes for thousands / millions.
pub fn fmt_thousands(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        format!("{n}")
    }
}

/// Semantic memory-pressure bucket used by both backends to pick a colour.
pub enum MemoryPalette {
    Ok,
    Warn,
    Hot,
    Unknown,
}

/// Picks a memory bucket based on used / total bytes.
pub fn mem_palette(used: f64, total: f64) -> MemoryPalette {
    if total <= 0.0 {
        return MemoryPalette::Unknown;
    }
    let pct = (used / total * 100.0) as u32;
    match pct {
        0..=MEM_GREEN_MAX => MemoryPalette::Ok,
        x if x <= MEM_YELLOW_MAX => MemoryPalette::Warn,
        _ => MemoryPalette::Hot,
    }
}

/// Semantic CPU-intensity bucket used by both backends to pick a colour.
pub enum CpuLevel {
    Hot,
    Warm,
    Cool,
}

/// Picks a CPU bucket based on a usage percentage.
pub fn cpu_level(cpu: f32) -> CpuLevel {
    if cpu > CPU_HOT {
        CpuLevel::Hot
    } else if cpu > CPU_WARM {
        CpuLevel::Warm
    } else {
        CpuLevel::Cool
    }
}
