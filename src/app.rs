use std::collections::VecDeque;
use std::mem;

use ratatui::widgets::TableState;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

const WINDOW: f64 = 180.0; // 3-minute sliding window (seconds)
const HISTORY_LEN: usize = 180;

// ── macOS mach FFI ──────────────────────────────────────────

const HOST_CPU_LOAD_INFO: i32 = 3;

#[repr(C)]
struct HostCpuLoadInfo {
    cpu_ticks: [u32; 4],
}

unsafe extern "C" {
    fn mach_host_self() -> u32;
    unsafe fn host_statistics(host: u32, flavor: i32, info: *mut i32, count: *mut u32) -> i32;
}

fn cached_host_port() -> u32 {
    use std::sync::OnceLock;
    static PORT: OnceLock<u32> = OnceLock::new();
    *PORT.get_or_init(|| unsafe { mach_host_self() })
}

fn get_cpu_ticks() -> Option<[u64; 4]> {
    unsafe {
        let mut info: HostCpuLoadInfo = mem::zeroed();
        let mut count = (mem::size_of::<HostCpuLoadInfo>() / mem::size_of::<u32>()) as u32;
        let ret = host_statistics(
            cached_host_port(),
            HOST_CPU_LOAD_INFO,
            &mut info as *mut HostCpuLoadInfo as *mut i32,
            &mut count,
        );
        if ret == 0 {
            Some(info.cpu_ticks.map(|t| t as u64))
        } else {
            None
        }
    }
}

// ── macOS libproc FFI (per-process thread count) ────────────

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

fn total_thread_count() -> usize {
    unsafe {
        let num_pids = proc_listallpids(std::ptr::null_mut(), 0);
        if num_pids <= 0 {
            return 0;
        }

        let mut pids = vec![0i32; num_pids as usize * 2];
        let bufsize = (pids.len() * mem::size_of::<i32>()) as i32;
        let actual = proc_listallpids(pids.as_mut_ptr() as *mut libc::c_void, bufsize);
        if actual <= 0 {
            return 0;
        }

        let expected = mem::size_of::<ProcTaskInfo>() as i32;
        pids[..actual as usize]
            .iter()
            .map(|&pid| {
                let mut info: ProcTaskInfo = mem::zeroed();
                let ret = proc_pidinfo(
                    pid,
                    PROC_PIDTASKINFO,
                    0,
                    &mut info as *mut ProcTaskInfo as *mut libc::c_void,
                    expected,
                );
                if ret == expected { info.pti_threadnum.max(0) as usize } else { 0 }
            })
            .sum()
    }
}

// ── Data ────────────────────────────────────────────────────

pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_usage: f32,
    pub memory: u64,
}

pub struct App {
    sys: System,
    prev_ticks: Option<[u64; 4]>,
    tick_count: u64,

    pub system_pct: f64,
    pub user_pct: f64,
    pub idle_pct: f64,

    pub system_history: VecDeque<(f64, f64)>,
    pub user_history: VecDeque<(f64, f64)>,

    pub thread_count: usize,
    pub total_memory: u64,
    pub used_memory: u64,

    pub processes: Vec<ProcessInfo>,
    pub table_state: TableState,
    selected_pid: Option<u32>,
    pub running: bool,
}

impl App {
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
            table_state,
            selected_pid: None,
            running: true,
        }
    }

    pub fn tick(&mut self) {
        self.update_cpu_split();
        self.update_processes();
        self.thread_count = total_thread_count();
        self.tick_count += 1;
    }

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

    pub fn history_bounds(&self) -> [f64; 2] {
        let end = (self.tick_count as f64).max(WINDOW);
        let start = end - WINDOW;
        [start, end]
    }

    fn update_cpu_split(&mut self) {
        let Some(now) = get_cpu_ticks() else {
            push_bounded(&mut self.system_history, (self.tick_count as f64, self.system_pct), HISTORY_LEN);
            push_bounded(&mut self.user_history, (self.tick_count as f64, self.user_pct), HISTORY_LEN);
            return;
        };

        if let Some(prev) = self.prev_ticks {
            let d_user = now[0].saturating_sub(prev[0]);
            let d_system = now[1].saturating_sub(prev[1]);
            let d_idle = now[2].saturating_sub(prev[2]);
            let d_nice = now[3].saturating_sub(prev[3]);
            let total = d_user + d_system + d_idle + d_nice;

            if total > 0 {
                self.user_pct = (d_user + d_nice) as f64 / total as f64 * 100.0;
                self.system_pct = d_system as f64 / total as f64 * 100.0;
                self.idle_pct = d_idle as f64 / total as f64 * 100.0;
            }
        }

        self.prev_ticks = Some(now);
        push_bounded(&mut self.system_history, (self.tick_count as f64, self.system_pct), HISTORY_LEN);
        push_bounded(&mut self.user_history, (self.tick_count as f64, self.user_pct), HISTORY_LEN);
    }

    fn update_processes(&mut self) {
        self.sys.refresh_memory();
        self.sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );

        self.total_memory = self.sys.total_memory();
        self.used_memory = self.sys.used_memory();

        let mut procs: Vec<ProcessInfo> = self
            .sys
            .processes()
            .values()
            .map(|p| ProcessInfo {
                pid: p.pid().as_u32(),
                name: p.name().to_string_lossy().into_owned(),
                cpu_usage: p.cpu_usage(),
                memory: p.memory(),
            })
            .collect();

        procs.sort_by(|a, b| {
            b.cpu_usage
                .partial_cmp(&a.cpu_usage)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        self.processes = procs;
        self.restore_selection();
    }

    fn restore_selection(&mut self) {
        let pid = match self.selected_pid {
            Some(pid) => pid,
            None => return,
        };

        if let Some(i) = self.processes.iter().position(|p| p.pid == pid) {
            self.table_state.select(Some(i));
        }
    }
}

fn push_bounded<T>(buf: &mut VecDeque<T>, value: T, max: usize) {
    if buf.len() >= max {
        buf.pop_front();
    }
    buf.push_back(value);
}
