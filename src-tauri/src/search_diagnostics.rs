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
}
struct Active {
    start: Instant,
    stages: BTreeMap<&'static str, StageTiming>,
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
        })
    });
}
pub fn finish() -> Option<SearchDiagnostics> {
    ACTIVE.with(|state| {
        state.borrow_mut().take().map(|active| SearchDiagnostics {
            elapsed_ms: active.start.elapsed().as_secs_f64() * 1000.0,
            stages: active.stages,
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
