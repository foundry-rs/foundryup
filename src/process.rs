use crate::config::Config;
use eyre::{Result, bail};
use std::ffi::OsStr;
use sysinfo::{Process, ProcessRefreshKind, ProcessesToUpdate, System, Uid, UpdateKind};

pub(crate) fn check_bins_in_use(config: &Config) -> Result<()> {
    let mut sys = System::new();
    // Refresh only the process data needed for the in-use check, excluding per-process
    // threads/tasks.
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_user(UpdateKind::Always).without_tasks(),
    );

    let current_user_id = current_effective_user_id(&sys);
    let names = sys
        .processes()
        .values()
        .filter(|process| {
            current_user_id.as_ref().is_none_or(|uid| process.effective_user_id() == Some(uid))
        })
        .map(Process::name);

    let bins = config.network.bins.iter().map(|bin| bin.name).collect::<Vec<_>>();
    if let Some(bin) = detect_in_use(&bins, names) {
        bail!("'{bin}' is currently running. Please stop the process and try again.");
    }

    Ok(())
}

fn current_effective_user_id(sys: &System) -> Option<Uid> {
    let pid = sysinfo::get_current_pid().ok()?;
    sys.process(pid)?.effective_user_id().cloned()
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

    const BINS: &[&str] = &["forge", "cast", "anvil", "chisel", "solar"];

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
