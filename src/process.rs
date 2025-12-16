use crate::config::Config;
use eyre::{Result, bail};
use sysinfo::System;

pub(crate) fn check_bins_in_use(config: &Config) -> Result<()> {
    let bins = config.bins(None);
    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    for bin in bins {
        if let Some(process) = sys.processes_by_name(bin.as_ref()).next() {
            let name = process.name().to_string_lossy();
            bail!("Error: '{name}' is currently running. Please stop the process and try again.");
        }
    }

    Ok(())
}
