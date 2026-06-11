use crate::config::Config;
use eyre::{Result, bail};
use std::ffi::OsStr;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

pub(crate) fn check_bins_in_use(config: &Config) -> Result<()> {
    let mut sys = System::new();
    // Refresh process names only, excluding per-process threads/tasks.
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().without_tasks(),
    );

    let names = sys.processes().values().map(sysinfo::Process::name);

    if let Some(bin) = detect_in_use(config.network.bins, names) {
        bail!("'{bin}' is currently running. Please stop the process and try again.");
    }

    Ok(())
}

/// Returns the first binary in `bins` whose name exactly matches a running process.
fn detect_in_use<'a, 'n>(
    bins: &[&'a str],
    names: impl Iterator<Item = &'n OsStr>,
) -> Option<&'a str> {
    let names: Vec<&OsStr> = names.collect();
    bins.iter().copied().find(|bin| names.iter().any(|&name| matches_bin(name, bin)))
}

/// Exact name match, accepting the `.exe` extension on Windows.
fn matches_bin(name: &OsStr, bin: &str) -> bool {
    if name == OsStr::new(bin) {
        return true;
    }
    #[cfg(windows)]
    if name == OsStr::new(&format!("{bin}.exe")) {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const BINS: &[&str] = &["forge", "cast", "anvil", "chisel"];

    fn detect(names: &[&str]) -> Option<&'static str> {
        detect_in_use(BINS, names.iter().map(|n| OsStr::new(*n)))
    }

    #[test]
    fn detects_first_running_bin_in_bins_order() {
        assert_eq!(detect(&["bash", "cast", "anvil", "rustc"]), Some("cast"));
    }

    #[test]
    fn requires_exact_match() {
        assert_eq!(detect(&["forge-fmt", "castaway", "myanvil", "chiseling"]), None);
    }

    #[test]
    fn none_when_no_bins_running() {
        assert_eq!(detect(&["bash", "node", "foundryup"]), None);
    }

    #[cfg(windows)]
    #[test]
    fn matches_windows_exe_suffix() {
        assert_eq!(detect(&["bash", "anvil.exe"]), Some("anvil"));
    }

    #[cfg(not(windows))]
    #[test]
    fn ignores_exe_suffix_on_unix() {
        assert_eq!(detect(&["bash", "anvil.exe"]), None);
    }
}
