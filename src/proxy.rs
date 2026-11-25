//! Proxy mode.
//!
//! Like rustup, the binaries in the bin dir are links to foundryup. When invoked as one of them,
//! foundryup runs that binary from the toolchain selected by a leading `+toolchain` argument, the
//! `FOUNDRYUP_TOOLCHAIN` environment variable, or the default set by `foundryup` and
//! `foundryup --use`.

use crate::{
    config::{Config, NetworkConfig},
    install::{bin_name, installed_toolchain_dir},
    say,
};
use eyre::{Result, WrapErr, bail};
use std::{
    convert::Infallible,
    path::{Path, PathBuf},
    process::Command,
};

const TOOLCHAIN_ENV: &str = "FOUNDRYUP_TOOLCHAIN";

/// Returns the Foundry binary that this process was invoked as, if any.
pub(crate) fn invoked_as() -> Option<&'static str> {
    let arg0 = PathBuf::from(std::env::args_os().next()?);
    let stem = arg0.file_stem()?;
    NetworkConfig::FOUNDRY
        .bins
        .iter()
        .map(|bin| bin.name)
        .find(|name| stem.eq_ignore_ascii_case(name))
}

pub(crate) fn main(bin: &str) -> ! {
    let Err(e) = run(bin);
    say!("error: {e:#}");
    std::process::exit(1)
}

fn run(bin: &str) -> Result<Infallible> {
    let mut args = std::env::args_os().skip(1).peekable();
    let plus = args
        .next_if(|arg| arg.to_str().is_some_and(|arg| arg.starts_with('+')))
        .map(|arg| arg.to_string_lossy()[1..].to_string());
    if plus.as_deref() == Some("") {
        bail!("missing toolchain name after '+'");
    }

    let config = installation()?;
    let name = plus.or_else(|| std::env::var(TOOLCHAIN_ENV).ok().filter(|name| !name.is_empty()));
    let dir = match &name {
        Some(name) => installed_toolchain_dir(&config, name)?,
        None => config.default_toolchain()?,
    };

    let path = dir.join(bin_name(bin));
    if !path.is_file() {
        bail!("{bin} is not installed in toolchain {}", dir.display());
    }

    let mut cmd = Command::new(&path);
    cmd.args(args);
    // Nested invocations, such as `cast` run by a `forge` script, use the same toolchain.
    if name.is_some() {
        cmd.env(TOOLCHAIN_ENV, std::path::absolute(&dir)?);
    }
    exec(&mut cmd).wrap_err_with(|| format!("failed to run {}", path.display()))
}

/// Returns the config of the installation that this proxy belongs to.
///
/// The proxy lives in `<foundry dir>/bin`, which may differ from the current `FOUNDRY_DIR` if
/// that was only set when installing.
fn installation() -> Result<Config> {
    let arg0 = std::env::args_os().next().unwrap_or_default();
    // Follow symlinks to the proxy, such as `/usr/local/bin/forge -> <foundry dir>/bin/forge`.
    let mut path = which::which(arg0).ok();
    while let Some(link) = path {
        if let Some(foundry_dir) = link.parent().and_then(Path::parent) {
            let config = Config::with_foundry_dir(foundry_dir.to_path_buf());
            if config.default_toolchain_file().is_file() {
                return Ok(config);
            }
        }
        path = std::fs::read_link(&link).ok().map(|target| link.with_file_name(target));
    }
    Config::new()
}

#[cfg(unix)]
fn exec(cmd: &mut Command) -> Result<Infallible> {
    use std::os::unix::process::CommandExt;
    Err(cmd.exec().into())
}

#[cfg(windows)]
fn exec(cmd: &mut Command) -> Result<Infallible> {
    use windows_sys::{
        Win32::{
            Foundation::{HANDLE, TRUE},
            System::{
                Console::SetConsoleCtrlHandler,
                JobObjects::{
                    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                    SetInformationJobObject,
                },
                Threading::GetCurrentProcess,
            },
        },
        core::BOOL,
    };

    // Leave Ctrl-C to the child, which shares this console, and keep waiting for its exit code.
    unsafe extern "system" fn ignore_ctrl_c(_: u32) -> BOOL {
        TRUE
    }

    let set_limits = |job: HANDLE, flags: u32| {
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = flags;
        // SAFETY: `info` is a valid `JOBOBJECT_EXTENDED_LIMIT_INFORMATION` of the given size.
        unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&raw const info).cast(),
                size_of_val(&info) as u32,
            ) != 0
        }
    };

    // SAFETY: `ignore_ctrl_c` is a valid handler for the life of the process, and the job handle
    // is only used while it is open.
    let job = unsafe {
        SetConsoleCtrlHandler(Some(ignore_ctrl_c), TRUE);

        // Like rustup, put this process in a job that kills the child when this process is
        // terminated, such as by a test harness. Failures only lose that guarantee.
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if !job.is_null()
            && set_limits(job, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE)
            && AssignProcessToJobObject(job, GetCurrentProcess()) != 0
        {
            Some(job)
        } else {
            None
        }
    };

    let status = cmd.status()?;
    // Leave processes that the child started in the background running.
    if let Some(job) = job {
        set_limits(job, 0);
    }
    std::process::exit(status.code().unwrap_or(1))
}
