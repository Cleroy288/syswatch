//! Application state and system-data collection.
//!
//! [`App`] owns the `sysinfo::System` handle, CPU tick history,
//! process list, and all derived metrics displayed by the UI.

use std::collections::{HashMap, VecDeque};
use std::mem;

use ratatui::widgets::TableState;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

/// Type alias for a macOS process identifier.
type Pid = u32;

/// Sliding-window width in seconds (3 minutes).
const WINDOW: f64 = 180.0;

/// Maximum number of data-points kept per history deque.
const HISTORY_LEN: usize = 180;

/// Sample thread count once every N ticks (cheap values re-use the cached total).
const THREAD_SAMPLE_EVERY: u64 = 5;

/// Retain only the top-N processes after sorting (enough for any UI viewport).
const DISPLAYED_PROCS: usize = 200;

// ── macOS mach FFI ──────────────────────────────────────────

/// Mach host_statistics flavor for CPU load info.
const HOST_CPU_LOAD_INFO: i32 = 3;

#[repr(C)]
struct HostCpuLoadInfo {
    cpu_ticks: [u32; 4],
}

unsafe extern "C" {
    fn mach_host_self() -> u32;
    unsafe fn host_statistics(host: u32, flavor: i32, info: *mut i32, count: *mut u32) -> i32;
}

/// Returns the cached Mach host port (evaluated once).
fn cached_host_port() -> u32 {
    use std::sync::OnceLock;
    static PORT: OnceLock<u32> = OnceLock::new();
    *PORT.get_or_init(|| unsafe { mach_host_self() })
}

/// Reads aggregate CPU ticks from the Mach kernel.
///
/// Returns `[user, system, idle, nice]` as `u64`, or `None` on failure.
fn get_cpu_ticks() -> Option<[u64; 4]> {
    unsafe {
        let mut info: HostCpuLoadInfo = mem::zeroed();
        let mut count = (mem::size_of::<HostCpuLoadInfo>() / mem::size_of::<u32>()) as u32;
        let ret = host_statistics(
            cached_host_port(),
            HOST_CPU_LOAD_INFO,
            (&raw mut info).cast::<i32>(),
            &mut count,
        );
        if ret == 0 {
            Some(info.cpu_ticks.map(u64::from))
        } else {
            None
        }
    }
}

// ── macOS libproc FFI (per-process thread count) ────────────

/// `proc_pidinfo` flavor for task-level info.
const PROC_PIDTASKINFO: i32 = 4;

#[repr(C)]
struct ProcTaskInfo {
    pti_virtual_size: u64,
    pti_resident_size: u64,
    pti_total_user: u64,
    pti_total_system: u64,
    pti_threads_user: u64,
    pti_threads_system: u64,
    pti_policy: i32,
    pti_faults: i32,
    pti_pageins: i32,
    pti_cow_faults: i32,
    pti_messages_sent: i32,
    pti_messages_received: i32,
    pti_syscalls_mach: i32,
    pti_syscalls_unix: i32,
    pti_csw: i32,
    pti_threadnum: i32,
    pti_numrunning: i32,
    pti_priority: i32,
}

unsafe extern "C" {
    unsafe fn proc_pidinfo(
        pid: i32,
        flavor: i32,
        arg: u64,
        buffer: *mut libc::c_void,
        buffersize: i32,
    ) -> i32;
    unsafe fn proc_listallpids(buffer: *mut libc::c_void, buffersize: i32) -> i32;
}

/// Sums thread counts across all processes, reusing `pid_buf` to avoid
/// per-tick allocations.
fn total_thread_count(pid_buf: &mut Vec<i32>) -> usize {
    unsafe {
        let num_pids = proc_listallpids(std::ptr::null_mut(), 0);
        if num_pids <= 0 {
            return 0;
        }

        let needed = num_pids as usize * 2;
        if pid_buf.len() < needed {
            pid_buf.resize(needed, 0);
        }

        let bufsize = (pid_buf.len() * mem::size_of::<i32>()) as i32;
        let actual = proc_listallpids(pid_buf.as_mut_ptr().cast::<libc::c_void>(), bufsize);
        if actual <= 0 {
            return 0;
        }

        let expected = mem::size_of::<ProcTaskInfo>() as i32;
        pid_buf[..actual as usize]
            .iter()
            .map(|&pid| thread_count_for(pid, expected))
            .sum()
    }
}

/// Returns the thread count for a single pid, or 0 on failure.
unsafe fn thread_count_for(pid: i32, expected: i32) -> usize {
    unsafe {
        let mut info: ProcTaskInfo = mem::zeroed();
        let ret = proc_pidinfo(
            pid,
            PROC_PIDTASKINFO,
            0,
            (&raw mut info).cast::<libc::c_void>(),
            expected,
        );
        if ret == expected {
            info.pti_threadnum.max(0) as usize
        } else {
            0
        }
    }
}

// ── Data ────────────────────────────────────────────────────

/// Snapshot of a single process shown in the table.
#[derive(Debug, Clone)]
pub struct ProcessInfo {
    /// macOS process identifier.
    pub pid: Pid,
    /// Display name of the process (allocation reused across ticks).
    pub name: String,
    /// Instantaneous CPU usage percentage.
    pub cpu_usage: f32,
    /// Resident memory in bytes.
    pub memory: u64,
}

/// Central application state — owns system handles, metrics, and UI state.
#[derive(Debug)]
pub struct App {
    sys: System,
    prev_ticks: Option<[u64; 4]>,
    tick_count: u64,

    /// System (kernel) CPU percentage.
    pub system_pct: f64,
    /// User-space CPU percentage.
    pub user_pct: f64,
    /// Idle CPU percentage.
    pub idle_pct: f64,

    /// Time-series of `(tick, system_pct)` for the chart.
    pub system_history: VecDeque<(f64, f64)>,
    /// Time-series of `(tick, user_pct)` for the chart.
    pub user_history: VecDeque<(f64, f64)>,

    /// Total thread count across all processes.
    pub thread_count: usize,
    /// Total physical memory in bytes.
    pub total_memory: u64,
    /// Used physical memory in bytes.
    pub used_memory: u64,

    /// Process list sorted by descending CPU usage (storage reused).
    pub processes: Vec<ProcessInfo>,
    /// Reused `pid -> processes[index]` map (storage retained between ticks).
    pid_index: HashMap<Pid, usize>,
    /// Reused pid buffer for [`total_thread_count`].
    pid_buf: Vec<i32>,

    /// Ratatui table selection state.
    pub table_state: TableState,
    selected_pid: Option<Pid>,
    /// Whether the event loop should keep running.
    pub running: bool,
}

impl App {
    /// Creates a new `App`, performing an initial full system refresh.
    pub fn new() -> Self {
        let mut sys = System::new_all();
        sys.refresh_all();

        let mut table_state = TableState::default();
        table_state.select(Some(0));

        Self {
            sys,
            prev_ticks: get_cpu_ticks(),
            tick_count: 0,
            system_pct: 0.0,
            user_pct: 0.0,
            idle_pct: 0.0,
            system_history: VecDeque::with_capacity(HISTORY_LEN),
            user_history: VecDeque::with_capacity(HISTORY_LEN),
            thread_count: 0,
            total_memory: 0,
            used_memory: 0,
            processes: Vec::new(),
            pid_index: HashMap::new(),
            pid_buf: Vec::new(),
            table_state,
            selected_pid: None,
            running: true,
        }
    }

    /// Advances state by one tick: refreshes CPU, memory, processes, threads.
    pub fn tick(&mut self) {
        self.update_cpu_split();
        self.update_processes();
        if self.tick_count.is_multiple_of(THREAD_SAMPLE_EVERY) {
            self.thread_count = total_thread_count(&mut self.pid_buf);
        }
        self.tick_count += 1;
    }

    /// Moves the process-table selection by `offset` rows (clamped).
    pub fn select_process(&mut self, offset: i32) {
        let len = self.processes.len();
        if len == 0 {
            return;
        }

        let current = self.table_state.selected().unwrap_or(0) as i32;
        let next = (current + offset).clamp(0, len as i32 - 1) as usize;

        self.table_state.select(Some(next));
        self.selected_pid = Some(self.processes[next].pid);
    }

    /// Returns `[start, end]` x-axis bounds for the CPU chart.
    pub fn history_bounds(&self) -> [f64; 2] {
        let end = (self.tick_count as f64).max(WINDOW);
        let start = end - WINDOW;
        [start, end]
    }

    /// Computes user / system / idle CPU percentages from Mach tick deltas.
    fn update_cpu_split(&mut self) {
        if let Some(now) = get_cpu_ticks() {
            self.apply_tick_delta(now);
            self.prev_ticks = Some(now);
        }
        self.record_history_sample();
    }

    /// Updates the CPU split from a delta against `prev_ticks`.
    fn apply_tick_delta(&mut self, now: [u64; 4]) {
        let Some(prev) = self.prev_ticks else {
            return;
        };

        let d_user = now[0].saturating_sub(prev[0]);
        let d_system = now[1].saturating_sub(prev[1]);
        let d_idle = now[2].saturating_sub(prev[2]);
        let d_nice = now[3].saturating_sub(prev[3]);
        let total = d_user + d_system + d_idle + d_nice;

        if total == 0 {
            return;
        }

        self.user_pct = (d_user + d_nice) as f64 / total as f64 * 100.0;
        self.system_pct = d_system as f64 / total as f64 * 100.0;
        self.idle_pct = d_idle as f64 / total as f64 * 100.0;
    }

    /// Pushes the current sample into both histories, keeping them contiguous.
    fn record_history_sample(&mut self) {
        let tick = self.tick_count as f64;
        push_bounded(&mut self.system_history, (tick, self.system_pct), HISTORY_LEN);
        push_bounded(&mut self.user_history, (tick, self.user_pct), HISTORY_LEN);
        self.system_history.make_contiguous();
        self.user_history.make_contiguous();
    }

    /// Returns the system CPU history as a contiguous slice.
    pub fn system_slice(&self) -> &[(f64, f64)] {
        self.system_history.as_slices().0
    }

    /// Returns the user CPU history as a contiguous slice.
    pub fn user_slice(&self) -> &[(f64, f64)] {
        self.user_history.as_slices().0
    }

    /// Refreshes the process list and memory counters from `sysinfo`.
    ///
    /// Reuses [`Self::processes`] storage and per-entry `String` allocations
    /// across ticks to keep steady-state memory churn close to zero.
    fn update_processes(&mut self) {
        self.sys.refresh_memory();
        self.sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );

        self.total_memory = self.sys.total_memory();
        self.used_memory = self.sys.used_memory();

        self.reconcile_process_list();
        self.keep_top_processes();
        self.rebuild_pid_index();
        self.restore_selection();
    }

    /// Keeps only the top [`DISPLAYED_PROCS`] processes by CPU, using
    /// partial selection (O(n)) before a final O(k log k) sort.
    fn keep_top_processes(&mut self) {
        if self.processes.len() > DISPLAYED_PROCS {
            self.processes
                .select_nth_unstable_by(DISPLAYED_PROCS, cmp_desc_cpu);
            self.processes.truncate(DISPLAYED_PROCS);
        }
        self.processes.sort_by(cmp_desc_cpu);
    }

    /// Updates `processes` in place from `sys.processes()`, reusing Strings.
    fn reconcile_process_list(&mut self) {
        for p in &mut self.processes {
            p.cpu_usage = f32::NAN;
        }

        for sp in self.sys.processes().values() {
            let pid = sp.pid().as_u32();
            let os_name = sp.name().to_string_lossy();
            let cpu = sp.cpu_usage();
            let memory = sp.memory();

            match self.pid_index.get(&pid).copied() {
                Some(i) if i < self.processes.len() && self.processes[i].pid == pid => {
                    let entry = &mut self.processes[i];
                    if entry.name != os_name.as_ref() {
                        entry.name.clear();
                        entry.name.push_str(&os_name);
                    }
                    entry.cpu_usage = cpu;
                    entry.memory = memory;
                }
                _ => self.processes.push(ProcessInfo {
                    pid,
                    name: os_name.into_owned(),
                    cpu_usage: cpu,
                    memory,
                }),
            }
        }

        self.processes.retain(|p| !p.cpu_usage.is_nan());
    }

    /// Rebuilds [`Self::pid_index`] after reordering `processes`.
    fn rebuild_pid_index(&mut self) {
        self.pid_index.clear();
        for (i, p) in self.processes.iter().enumerate() {
            self.pid_index.insert(p.pid, i);
        }
    }

    /// Re-selects the previously highlighted PID after a sort shuffle.
    fn restore_selection(&mut self) {
        let Some(pid) = self.selected_pid else {
            return;
        };

        if let Some(&i) = self.pid_index.get(&pid) {
            self.table_state.select(Some(i));
        }
    }
}

/// Compares two [`ProcessInfo`] entries by descending CPU usage.
fn cmp_desc_cpu(a: &ProcessInfo, b: &ProcessInfo) -> std::cmp::Ordering {
    b.cpu_usage
        .partial_cmp(&a.cpu_usage)
        .unwrap_or(std::cmp::Ordering::Equal)
}

/// Pushes `value` into `buf`, evicting the oldest entry when full.
fn push_bounded<T>(buf: &mut VecDeque<T>, value: T, max: usize) {
    if buf.len() >= max {
        buf.pop_front();
    }
    buf.push_back(value);
}
