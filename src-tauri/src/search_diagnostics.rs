//! Opt-in per-worker timing; no source paths, queries or message content are collected.
use serde::Serialize;
use std::{cell::RefCell, collections::BTreeMap, time::Instant};

#[derive(Debug, Clone, Default, Serialize)]
pub struct StageTiming {
    pub count: u64,
    pub total_ms: f64,
}
#[derive(Debug, Clone, Serialize)]
pub struct SearchDiagnostics {
    pub elapsed_ms: f64,
    pub stages: BTreeMap<&'static str, StageTiming>,
    pub resources: Option<ResourceDiagnostics>,
}
/// Per-query resource deltas, sampled only at query boundaries on Windows.
/// Process I/O includes every process thread and is not physical disk I/O.
/// Wall time minus worker CPU time also includes blocking and instrumentation;
/// it must not be labelled exclusively as scheduler wait.
#[derive(Debug, Clone, Serialize)]
pub struct ResourceDiagnostics {
    pub worker_thread_id: u32,
    pub process_id: u32,
    pub worker_cpu_kernel_ms: Option<f64>,
    pub worker_cpu_user_ms: Option<f64>,
    /// Raw cycles: CPU frequency changes preclude converting these into time.
    pub worker_cycles: Option<u64>,
    pub process_io: Option<ProcessIo>,
    /// All CPUs (or the calling processor group on machines with >64 CPUs).
    pub system_cpu_busy_pct: Option<f64>,
}
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct ProcessIo {
    pub read_operations: u64,
    pub write_operations: u64,
    pub other_operations: u64,
    pub read_bytes: u64,
    pub write_bytes: u64,
    pub other_bytes: u64,
}
#[cfg(any(windows, test))]
impl ProcessIo {
    fn delta(self, before: Self) -> Option<Self> {
        Some(Self {
            read_operations: self.read_operations.checked_sub(before.read_operations)?,
            write_operations: self.write_operations.checked_sub(before.write_operations)?,
            other_operations: self.other_operations.checked_sub(before.other_operations)?,
            read_bytes: self.read_bytes.checked_sub(before.read_bytes)?,
            write_bytes: self.write_bytes.checked_sub(before.write_bytes)?,
            other_bytes: self.other_bytes.checked_sub(before.other_bytes)?,
        })
    }
}
#[cfg(any(windows, test))]
#[derive(Default)]
struct ResourceSnapshot {
    worker_thread_id: u32,
    process_id: u32,
    worker_times: Option<(u64, u64)>,
    worker_cycles: Option<u64>,
    process_io: Option<ProcessIo>,
    // Windows kernel time includes idle time.
    system_times: Option<(u64, u64, u64)>,
}
#[cfg(not(any(windows, test)))]
struct ResourceSnapshot;

#[cfg(any(windows, test))]
fn delta(before: Option<u64>, after: Option<u64>) -> Option<u64> {
    after?.checked_sub(before?)
}
#[cfg(any(windows, test))]
impl ResourceSnapshot {
    fn difference(&self, after: Self) -> ResourceDiagnostics {
        let same_worker = self.worker_thread_id == after.worker_thread_id;
        let system_cpu_busy_pct = self.system_times.zip(after.system_times).and_then(
            |((idle, kernel, user), (end_idle, end_kernel, end_user))| {
                let total = end_kernel
                    .checked_sub(kernel)?
                    .checked_add(end_user.checked_sub(user)?)?;
                let busy = total.checked_sub(end_idle.checked_sub(idle)?)?;
                (total > 0).then(|| 100.0 * busy as f64 / total as f64)
            },
        );
        ResourceDiagnostics {
            worker_thread_id: self.worker_thread_id,
            process_id: self.process_id,
            worker_cpu_kernel_ms: same_worker
                .then(|| {
                    delta(
                        self.worker_times.map(|t| t.0),
                        after.worker_times.map(|t| t.0),
                    )
                })
                .flatten()
                .map(|ticks| ticks as f64 / 10_000.0),
            worker_cpu_user_ms: same_worker
                .then(|| {
                    delta(
                        self.worker_times.map(|t| t.1),
                        after.worker_times.map(|t| t.1),
                    )
                })
                .flatten()
                .map(|ticks| ticks as f64 / 10_000.0),
            worker_cycles: same_worker
                .then(|| delta(self.worker_cycles, after.worker_cycles))
                .flatten(),
            process_io: self
                .process_io
                .zip(after.process_io)
                .filter(|_| self.process_id == after.process_id)
                .and_then(|(before, after)| after.delta(before)),
            system_cpu_busy_pct,
        }
    }
}
#[cfg(windows)]
fn resource_snapshot() -> Option<ResourceSnapshot> {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::Threading::{
            GetCurrentProcess, GetCurrentProcessId, GetCurrentThread, GetCurrentThreadId,
            GetProcessIoCounters, GetSystemTimes, GetThreadTimes, IO_COUNTERS,
        },
        System::WindowsProgramming::QueryThreadCycleTime,
    };
    fn ticks(value: FILETIME) -> u64 {
        (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
    }
    // SAFETY: output buffers are initialized and live for each call. The current
    // thread/process pseudo-handles need no close and are never stored or shared.
    unsafe {
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        let worker_times = (GetThreadTimes(
            GetCurrentThread(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        ) != 0)
            .then(|| (ticks(kernel), ticks(user)));
        let mut cycles = 0;
        let worker_cycles =
            (QueryThreadCycleTime(GetCurrentThread(), &mut cycles) != 0).then_some(cycles);
        let mut io = IO_COUNTERS::default();
        let process_io =
            (GetProcessIoCounters(GetCurrentProcess(), &mut io) != 0).then_some(ProcessIo {
                read_operations: io.ReadOperationCount,
                write_operations: io.WriteOperationCount,
                other_operations: io.OtherOperationCount,
                read_bytes: io.ReadTransferCount,
                write_bytes: io.WriteTransferCount,
                other_bytes: io.OtherTransferCount,
            });
        let mut idle = FILETIME::default();
        let system_times = (GetSystemTimes(&mut idle, &mut kernel, &mut user) != 0)
            .then(|| (ticks(idle), ticks(kernel), ticks(user)));
        Some(ResourceSnapshot {
            worker_thread_id: GetCurrentThreadId(),
            process_id: GetCurrentProcessId(),
            worker_times,
            worker_cycles,
            process_io,
            system_times,
        })
    }
}
#[cfg(not(windows))]
fn resource_snapshot() -> Option<ResourceSnapshot> {
    None
}
fn resource_difference(before: Option<ResourceSnapshot>) -> Option<ResourceDiagnostics> {
    #[cfg(windows)]
    {
        Some(before?.difference(resource_snapshot()?))
    }
    #[cfg(not(windows))]
    {
        let _ = before;
        None
    }
}
struct Active {
    start: Instant,
    stages: BTreeMap<&'static str, StageTiming>,
    resources: Option<ResourceSnapshot>,
}
thread_local! { static ACTIVE: RefCell<Option<Active>> = const { RefCell::new(None) }; }

pub fn begin() {
    begin_enabled(std::env::var_os("AGENTVAULT_SEARCH_DIAGNOSTICS").is_some_and(|v| v == "1"));
}
fn begin_enabled(enabled: bool) {
    ACTIVE.with(|state| {
        *state.borrow_mut() = enabled.then(|| Active {
            start: Instant::now(),
            stages: BTreeMap::new(),
            resources: resource_snapshot(),
        })
    });
}
pub fn finish() -> Option<SearchDiagnostics> {
    ACTIVE.with(|state| {
        state.borrow_mut().take().map(|active| SearchDiagnostics {
            elapsed_ms: active.start.elapsed().as_secs_f64() * 1000.0,
            stages: active.stages,
            resources: resource_difference(active.resources),
        })
    })
}
pub struct Guard {
    stage: &'static str,
    start: Option<Instant>,
}
pub fn stage(stage: &'static str) -> Guard {
    Guard {
        stage,
        start: ACTIVE.with(|state| state.borrow().is_some().then(Instant::now)),
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(start) = self.start {
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            ACTIVE.with(|state| {
                if let Some(active) = state.borrow_mut().as_mut() {
                    let timing = active.stages.entry(self.stage).or_default();
                    timing.count += 1;
                    timing.total_ms += elapsed;
                }
            });
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resource_deltas_preserve_unavailable_and_reject_counter_resets() {
        let before = ResourceSnapshot {
            worker_thread_id: 42,
            process_id: 7,
            worker_times: Some((10_000, 20_000)),
            worker_cycles: Some(100),
            process_io: Some(ProcessIo::default()),
            system_times: Some((100, 200, 100)),
        };
        let after = ResourceSnapshot {
            worker_thread_id: 42,
            process_id: 7,
            worker_times: Some((30_000, 50_000)),
            worker_cycles: Some(400),
            process_io: Some(ProcessIo {
                read_operations: 2,
                write_operations: 3,
                other_operations: 4,
                read_bytes: 200,
                write_bytes: 300,
                other_bytes: 400,
            }),
            system_times: Some((150, 300, 200)),
        };
        let result = before.difference(after);
        assert_eq!(result.worker_cpu_kernel_ms, Some(2.0));
        assert_eq!(result.worker_cpu_user_ms, Some(3.0));
        assert_eq!(result.worker_cycles, Some(300));
        assert_eq!(result.system_cpu_busy_pct, Some(75.0));
        let io = result.process_io.unwrap();
        assert_eq!(
            (io.read_operations, io.write_operations, io.other_operations),
            (2, 3, 4)
        );
        assert_eq!(
            (io.read_bytes, io.write_bytes, io.other_bytes),
            (200, 300, 400)
        );
        assert!(ProcessIo::default().delta(io).is_none());
        assert_eq!(delta(Some(5), Some(4)), None);
        assert_eq!(delta(None, Some(4)), None);
        assert_eq!(delta(Some(5), None), None);
        assert_eq!(delta(Some(5), Some(5)), Some(0));
        let unavailable = before.difference(ResourceSnapshot::default());
        assert!(unavailable.worker_cpu_kernel_ms.is_none());
        assert!(unavailable.worker_cpu_user_ms.is_none());
        assert!(unavailable.worker_cycles.is_none());
        assert!(unavailable.process_io.is_none());
        assert!(unavailable.system_cpu_busy_pct.is_none());
        let other_worker = before.difference(ResourceSnapshot {
            worker_thread_id: 43,
            process_id: 8,
            worker_times: Some((30_000, 50_000)),
            worker_cycles: Some(400),
            process_io: Some(ProcessIo::default()),
            system_times: None,
        });
        assert!(other_worker.worker_cpu_kernel_ms.is_none());
        assert!(other_worker.worker_cpu_user_ms.is_none());
        assert!(other_worker.worker_cycles.is_none());
        assert!(other_worker.process_io.is_none());
    }
    #[test]
    fn system_cpu_rejects_idle_over_total_and_zero_interval() {
        let before = ResourceSnapshot {
            system_times: Some((10, 20, 10)),
            ..Default::default()
        };
        for times in [(10, 20, 10), (100, 21, 11), (0, 21, 11)] {
            assert!(before
                .difference(ResourceSnapshot {
                    system_times: Some(times),
                    ..Default::default()
                })
                .system_cpu_busy_pct
                .is_none());
        }
    }
    #[test]
    fn disabled_collection_and_thread_isolation() {
        begin_enabled(false);
        {
            let _guard = stage("off");
        }
        assert!(finish().is_none());
        begin_enabled(true);
        {
            let _guard = stage("parent");
        }
        std::thread::spawn(|| {
            assert!(finish().is_none());
            begin_enabled(true);
            {
                let _guard = stage("child");
            }
            let result = finish().unwrap();
            assert_eq!(result.stages["child"].count, 1);
            assert!(!result.stages.contains_key("parent"));
        })
        .join()
        .unwrap();
        let result = finish().unwrap();
        assert_eq!(result.stages["parent"].count, 1);
        assert!(!result.stages.contains_key("child"));
        assert!(finish().is_none());
    }
    #[test]
    fn error_return_records_guard() {
        fn fail() -> Result<(), ()> {
            let _guard = stage("failure");
            Err(())
        }
        begin_enabled(true);
        assert!(fail().is_err());
        assert_eq!(finish().unwrap().stages["failure"].count, 1);
    }
}
