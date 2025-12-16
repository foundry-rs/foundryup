use crate::cli::Network;
use eyre::Result;
use fs_err as fs;
use std::path::PathBuf;

pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

pub(crate) const LONG_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("VERGEN_GIT_SHA"),
    " ",
    env!("VERGEN_BUILD_TIMESTAMP"),
    ")"
);

pub(crate) const FOUNDRY_REPO: &str = "foundry-rs/foundry";
pub(crate) const FOUNDRYUP_REPO: &str = "foundry-rs/foundryup";
pub(crate) const TEMPO_REPO: &str = "tempoxyz/tempo-foundry";

pub(crate) const BINS: &[&str] = &["forge", "cast", "anvil", "chisel"];
pub(crate) const TEMPO_BINS: &[&str] = &["forge", "cast"];

#[derive(Debug)]
pub(crate) struct Config {
    pub foundry_dir: PathBuf,
    pub versions_dir: PathBuf,
    pub bin_dir: PathBuf,
    pub man_dir: PathBuf,
}

impl Config {
    pub(crate) fn new() -> Result<Self> {
        let base_dir =
            std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(home::home_dir);

        let base_dir = base_dir.ok_or_else(|| eyre::eyre!("could not determine home directory"))?;

        let foundry_dir = std::env::var_os("FOUNDRY_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| base_dir.join(".foundry"));

        let versions_dir = foundry_dir.join("versions");
        let bin_dir = foundry_dir.join("bin");
        let man_dir = foundry_dir.join("share/man/man1");

        Ok(Self { foundry_dir, versions_dir, bin_dir, man_dir })
    }

    pub(crate) fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(&self.versions_dir)?;
        fs::create_dir_all(&self.bin_dir)?;
        fs::create_dir_all(&self.man_dir)?;
        Ok(())
    }

    pub(crate) fn version_dir(&self, version: &str) -> PathBuf {
        self.versions_dir.join(version)
    }

    pub(crate) fn bin_path(&self, name: &str) -> PathBuf {
        let name = if cfg!(windows) && !name.ends_with(".exe") {
            format!("{name}.exe")
        } else {
            name.to_string()
        };
        self.bin_dir.join(name)
    }

    pub(crate) fn bins(&self, network: Option<Network>) -> &'static [&'static str] {
        match network {
            Some(Network::Tempo) => TEMPO_BINS,
            None => BINS,
        }
    }

    pub(crate) fn repo(&self, network: Option<Network>) -> &'static str {
        match network {
            Some(Network::Tempo) => TEMPO_REPO,
            None => FOUNDRY_REPO,
        }
    }

    pub(crate) fn repo_dir(&self, repo: &str) -> PathBuf {
        self.foundry_dir.join(repo)
    }
}
