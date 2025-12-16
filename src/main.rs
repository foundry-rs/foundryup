//! # foundryup
//!
//! Foundry toolchain manager.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(test)]
use snapbox as _;

use clap::Parser;
use eyre::Result;

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
    let config = Config::new()?;

    // Handle --completions first (no banner)
    if let Some(shell) = cli.completions {
        cli::print_completions(shell);
        return Ok(());
    }

    // Print banner for all other operations
    print_banner();
    check_update(&config).await;
    process::check_bins_in_use(&config)?;

    // Handle --update
    if cli.update {
        return self_update::run(&config).await;
    }

    // Handle --list
    if cli.list {
        return install::list(&config);
    }

    // Handle --use
    if let Some(ref version) = cli.use_version {
        return install::use_version(&config, version);
    }

    // Default: install
    install::run(&config, &cli).await
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

async fn check_update(config: &Config) {
    say("checking if foundryup is up to date...");
    match self_update::check_for_update(config).await {
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
        Ok(None) => say("foundryup is up to date."),
        Err(e) => warn(&format!("Could not check for updates: {e}")),
    }
}

pub fn say(msg: &str) {
    eprintln!("foundryup: {msg}");
}

pub fn warn(msg: &str) {
    eprintln!("foundryup: warning: {msg}");
}
