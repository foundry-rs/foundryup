use crate::config::Config;
use eyre::{Result, bail};
use sysinfo::System;

pub(crate) fn check_bins_in_use(config: &Config) -> Result<()> {
    let bins = config.bins(None);
    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    for &bin in bins {
        for proc in sys.processes_by_name(bin.as_ref()) {
            let name = &*proc.name().to_string_lossy();
            if name == bin || name.ends_with(&*format!("/{bin}")) {
                bail!("'{name}' is currently running, please stop the process and try again");
            }
        }
    }

    Ok(())
}
