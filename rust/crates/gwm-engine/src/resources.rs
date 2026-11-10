//! CPU and memory: the computer's budget for all running tasks together, and
//! what one task gets (same rules as v1's conf/base.config and compiler).

use gwm_core::compiler::PlanStep;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Limits {
    /// CPU threads all running tasks may use together (and the most one task gets)
    pub cpus: u32,
    /// GB all running tasks may use together (and the most one task gets)
    pub memory_gb: u32,
    /// the computer's RAM in whole GB
    pub total_memory_gb: u32,
}

impl Limits {
    /// Every CPU thread and total RAM minus 4 GB, so the desktop keeps room
    /// (otherwise the system can kill things mid-run). 0 = detect.
    ///
    /// The 4 GB are kept back once. On a Mac the containers run in a Linux
    /// machine (Colima) that the app gives the Mac's memory minus 4 GB: the
    /// runs get all of that machine, and never more than it really has (one
    /// made earlier with less).
    pub fn detect(max_cpus: u32, max_memory_gb: u32) -> Limits {
        let threads = std::thread::available_parallelism()
            .map(|n| n.get() as u32)
            .unwrap_or(1);
        let mut sys = sysinfo::System::new();
        sys.refresh_memory();
        let total = (sys.total_memory() / 1_073_741_824) as u32;
        let (memory_gb, total_memory_gb) = budget(total, crate::docker::memory_gb());
        Limits {
            cpus: if max_cpus > 0 { max_cpus } else { threads },
            memory_gb: if max_memory_gb > 0 { max_memory_gb } else { memory_gb },
            total_memory_gb,
        }
    }
}

/// (GB for all tasks together, GB the tasks' machine has) from this computer's
/// RAM and, when Docker answers, the memory of Docker's machine.
fn budget(total: u32, docker: Option<u32>) -> (u32, u32) {
    let reserve = total.saturating_sub(4).max(2);
    match docker {
        // Docker's machine is smaller than the computer (Mac): it is the budget
        Some(d) if d < total => (d.min(reserve), d),
        // Docker runs on the computer itself (Linux, WSL)
        _ => (reserve, total),
    }
}

/// GB of memory a task of this step asks for. A memory setting (e.g. Shovill's
/// "Memory (GB)"): a number = that many GB; 0/blank = Auto = the tool's amount
/// (or its low-memory amount), leaving 2 GB for the rest of the computer.
pub fn task_memory_gb(step: &PlanStep, total_memory_gb: u32) -> u32 {
    let r = &step.resources;
    let base = r.memory_gb.unwrap_or(2);
    let Some(setting) = &r.memory_setting else { return base };
    if let Some(gb) = step
        .settings
        .get(setting)
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|g| *g > 0)
    {
        return gb as u32;
    }
    let low = r
        .low_memory_setting
        .as_ref()
        .is_some_and(|s| step.settings.get(s).is_some_and(|v| v == "true"));
    let want = if low { r.low_memory_gb.unwrap_or(base) } else { base };
    want.min(total_memory_gb.saturating_sub(2).max(4))
}

#[cfg(test)]
mod tests {
    use super::budget;

    #[test]
    fn four_gb_are_kept_back_once() {
        // Linux and WSL: Docker has the whole computer
        assert_eq!(budget(32, Some(32)), (28, 32));
        assert_eq!(budget(32, None), (28, 32));
        // Mac: Colima has the Mac minus 4 GB, and the runs get all of it
        assert_eq!(budget(32, Some(28)), (28, 28));
        assert_eq!(budget(16, Some(12)), (12, 12));
        // a Colima machine made earlier with less: no more than it has
        assert_eq!(budget(16, Some(4)), (4, 4));
        // a small computer keeps at least 2 GB for runs
        assert_eq!(budget(4, None), (2, 4));
    }
}
