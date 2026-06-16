//! # foundryup
//!
//! Foundry toolchain manager.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(test)]
use snapbox as _;

use clap::Parser;
use eyre::Result;
use std::sync::Arc;

mod cli;
mod config;
mod download;
mod install;
mod platform;
mod process;
mod self_update;

use cli::Cli;
use config::Config;

/// Removes empty `FOUNDRYUP_*` variables before clap parses them.
///
/// clap's `env` support captures `Some("")` and parses it as a real value (e.g.
/// an empty `FOUNDRYUP_VERSION` treated as a real version), so an empty variable
/// is cleared here and treated as unset.
fn clear_empty_foundryup_env() {
    // `vars_os` is a snapshot, so removing during iteration is safe.
    for (key, value) in std::env::vars_os() {
        if value.is_empty() && key.to_str().is_some_and(|key| key.starts_with("FOUNDRYUP_")) {
            // SAFETY: runs at startup before any threads are spawned.
            unsafe { std::env::remove_var(&key) };
        }
    }
}

fn main() -> Result<()> {
    clear_empty_foundryup_env();

    color_eyre::install()?;
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive(tracing::level_filters::LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .without_time()
        .with_target(false)
        .init();

    let mut cli = Cli::parse();
    cli.clear_empty_values();

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;

    rt.block_on(run(cli))
}

async fn run(cli: Cli) -> Result<()> {
    // Handle --completions first (no banner, no config needed)
    if let Some(shell) = cli.completions {
        cli::print_completions(shell);
        return Ok(());
    }

    let config = Arc::new(Config::new()?);

    if cli.update {
        return self_update::run(&config).await;
    }

    config.migrate_legacy_versions()?;

    // `--list`/`--use` run offline: no update check or banner.
    if cli.list {
        return install::list(&config);
    }

    if let Some(ref version) = cli.use_version {
        // An empty `--use` value is rejected at parse time by `parse_use_version`, so `version` is
        // always non-empty here.
        let repo = cli.repo.as_deref().unwrap_or(config.network.repo);
        return install::use_version_resolved(&config, repo, version, cli.repo.is_some()).await;
    }

    if cli.network.is_some() {
        warn!(
            "the --network flag is deprecated and will be removed in a future release. Tempo is now included in the default Foundry installation."
        );
    }

    print_banner();

    // Run the update check in the background so it doesn't block the install.
    let update_handle = tokio::spawn({
        let config = config.clone();
        async move { self_update::check_for_update(&config).await }
    });

    let install_result = async {
        process::check_bins_in_use(&config)?;
        install::run(&config, &cli).await
    }
    .await;

    // Report the update status regardless of whether the install succeeded; a
    // failure of the background check itself must not mask an install error.
    match update_handle.await {
        Ok(update) => print_update(update),
        Err(e) => warn!("Could not check for updates: {e}"),
    }

    install_result
}

fn print_banner() {
    eprintln!(
        r#"
.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx

 ╔═╗ ╔═╗ ╦ ╦ ╔╗╔ ╔╦╗ ╦═╗ ╦ ╦         Portable and modular toolkit
 ╠╣  ║ ║ ║ ║ ║║║  ║║ ╠╦╝ ╚╦╝    for Ethereum Application Development
 ╚   ╚═╝ ╚═╝ ╝╚╝ ═╩╝ ╩╚═  ╩                 written in Rust.

.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx

Repo       : https://github.com/foundry-rs/foundry
Book       : https://book.getfoundry.sh/
Chat       : https://t.me/foundry_rs/
Support    : https://t.me/foundry_support/
Contribute : https://github.com/foundry-rs/foundry/blob/HEAD/CONTRIBUTING.md

.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx.xOx
"#
    );
}

fn print_update(res: Result<Option<String>>) {
    match res {
        Ok(Some(new_version)) => {
            eprintln!(
                r#"
Your installation of foundryup is out of date.

Installed: {} → Latest: {new_version}

To update, run:

  foundryup --update

Updating is highly recommended as it gives you access to the latest features and bug fixes.
"#,
                config::VERSION
            );
        }
        Ok(None) => say!("foundryup is up to date."),
        Err(e) => warn!("Could not check for updates: {e}"),
    }
}

#[macro_export]
macro_rules! say {
    ($($arg:tt)*) => {
        eprintln!("foundryup: {}", format_args!($($arg)*))
    };
}

/// Like [`say`], but writes to stdout instead of stderr.
#[macro_export]
macro_rules! tell {
    ($($arg:tt)*) => {
        println!("foundryup: {}", format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {
        eprintln!("foundryup: warning: {}", format_args!($($arg)*))
    };
}
