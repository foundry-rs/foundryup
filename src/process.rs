use crate::{config::Config, warn};
use eyre::Result;
use sysinfo::System;

pub(crate) fn check_bins_in_use(config: &Config) -> Result<()> {
    let bins = config.network.bins;
    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    for (pid, proc) in sys.processes() {
        if let Some(exe) = proc.exe().or_else(|| proc.cmd().first().map(AsRef::as_ref))
            && let Some(exe_fname) = exe.file_name()
            && let Some(exe_fname) = exe_fname.to_str()
            && let Some(bin) = bins.iter().find(|&&bin| exe_fname.starts_with(bin))
        {
            warn(&format!(
                "'{bin}' is currently running (PID: {pid}), please stop the process and try again"
            ));
        }
    }

    Ok(())
}
