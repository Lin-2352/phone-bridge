//! The PC budgets after M5, checked against measured rows (scripts/pc-budget.ps1 writes them).
//! Pure: no Windows calls, so the check runs everywhere.

#![allow(dead_code)] // the run prints its table in the PowerShell script; the check is the tested part

/// Idle tray: working set at most 40 MB.
pub const IDLE_WORKING_SET_MB: f64 = 40.0;
/// Idle tray: private bytes at most 15 MB.
pub const IDLE_PRIVATE_MB: f64 = 15.0;
/// Main window open: private bytes at most 70 MB.
pub const WINDOW_PRIVATE_MB: f64 = 70.0;
/// Idle tray over 60 s: less than 0.05 s of CPU.
pub const IDLE_CPU_SECONDS: f64 = 0.05;

/// One measured row: the figures a budget run records.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BudgetRow {
    pub idle_working_set_mb: f64,
    pub idle_private_mb: f64,
    pub window_private_mb: f64,
    pub idle_cpu_seconds: f64,
}

/// A figure that went over its budget.
#[derive(Clone, Debug, PartialEq)]
pub struct Overrun {
    pub column: &'static str,
    pub measured: f64,
    pub limit: f64,
}

/// Every column over its limit (a figure exactly at its limit passes).
pub fn check_budget(row: &BudgetRow) -> Vec<Overrun> {
    let checks = [
        (
            "idle working set (MB)",
            row.idle_working_set_mb,
            IDLE_WORKING_SET_MB,
        ),
        ("idle private (MB)", row.idle_private_mb, IDLE_PRIVATE_MB),
        (
            "window private (MB)",
            row.window_private_mb,
            WINDOW_PRIVATE_MB,
        ),
        ("idle CPU (s)", row.idle_cpu_seconds, IDLE_CPU_SECONDS),
    ];
    checks
        .into_iter()
        .filter(|(_, measured, limit)| measured > limit)
        .map(|(column, measured, limit)| Overrun {
            column,
            measured,
            limit,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_table_flags_each_overrun() {
        let over = BudgetRow {
            idle_working_set_mb: 41.0,
            idle_private_mb: 16.0,
            window_private_mb: 71.0,
            idle_cpu_seconds: 0.06,
        };
        let got = check_budget(&over);
        assert_eq!(got.len(), 4, "one overrun per column: {got:?}");
        let columns: Vec<_> = got.iter().map(|o| o.column).collect();
        assert_eq!(
            columns,
            [
                "idle working set (MB)",
                "idle private (MB)",
                "window private (MB)",
                "idle CPU (s)"
            ]
        );
        let at_limit = BudgetRow {
            idle_working_set_mb: IDLE_WORKING_SET_MB,
            idle_private_mb: IDLE_PRIVATE_MB,
            window_private_mb: WINDOW_PRIVATE_MB,
            idle_cpu_seconds: 0.0,
        };
        assert!(check_budget(&at_limit).is_empty(), "at the limit passes");
    }
}
