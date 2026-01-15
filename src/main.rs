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

fn main() -> Result<()> {
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

    let cli = Cli::parse();

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;

    rt.block_on(run(cli))
}

async fn run(cli: Cli) -> Result<()> {
    // Handle --completions first (no banner, no config needed)
    if let Some(shell) = cli.completions {
        cli::print_completions(shell);
        return Ok(());
    }

    let config = Arc::new(Config::new(cli.network)?);
    config.migrate_legacy_versions()?;

    if cli.update {
        return self_update::run(&config).await;
    }

    let update_handle = tokio::spawn({
        let config = config.clone();
        async move { self_update::check_for_update(&config).await }
    });

    if cli.list {
        install::list(&config)?;
    } else if let Some(ref version) = cli.use_version {
        install::use_version(&config, config.network.repo, version)?;
    } else {
        print_banner();
        process::check_bins_in_use(&config)?;
        install::run(&config, &cli).await?;
    }

    print_update(update_handle.await?);

    Ok(())
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

#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {
        eprintln!("foundryup: warning: {}", format_args!($($arg)*))
    };
}
