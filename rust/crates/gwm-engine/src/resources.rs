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
    pub fn detect(max_cpus: u32, max_memory_gb: u32) -> Limits {
        let threads = std::thread::available_parallelism()
            .map(|n| n.get() as u32)
            .unwrap_or(1);
        let mut sys = sysinfo::System::new();
        sys.refresh_memory();
        let total = (sys.total_memory() / 1_073_741_824) as u32;
        Limits {
            cpus: if max_cpus > 0 { max_cpus } else { threads },
            memory_gb: if max_memory_gb > 0 {
                max_memory_gb
            } else {
                total.saturating_sub(4).max(2)
            },
            total_memory_gb: total,
        }
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
