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

pub(crate) const FOUNDRYUP_REPO: &str = "foundry-rs/foundryup";

#[derive(Debug)]
pub(crate) struct Config {
    pub foundry_dir: PathBuf,
    pub versions_dir: PathBuf,
    pub bin_dir: PathBuf,
    pub man_dir: PathBuf,
    pub network: NetworkConfig,
}

impl Config {
    pub(crate) fn new(network: Option<Network>) -> Result<Self> {
        let base_dir =
            std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(home::home_dir);

        let base_dir = base_dir.ok_or_else(|| eyre::eyre!("could not determine home directory"))?;

        let foundry_dir = std::env::var_os("FOUNDRY_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| base_dir.join(".foundry"));

        let versions_dir = foundry_dir.join("versions");
        let bin_dir = foundry_dir.join("bin");
        let man_dir = foundry_dir.join("share/man/man1");
        let network = NetworkConfig::for_network(network);

        Ok(Self { foundry_dir, versions_dir, bin_dir, man_dir, network })
    }

    pub(crate) fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(&self.versions_dir)?;
        fs::create_dir_all(&self.bin_dir)?;
        fs::create_dir_all(&self.man_dir)?;
        Ok(())
    }

    pub(crate) fn version_dir(&self, repo: &str, version: &str) -> PathBuf {
        self.versions_dir.join(repo).join(version)
    }

    pub(crate) fn bin_path(&self, name: &str) -> PathBuf {
        let name = if cfg!(windows) && !name.ends_with(".exe") {
            format!("{name}.exe")
        } else {
            name.to_string()
        };
        self.bin_dir.join(name)
    }

    pub(crate) fn repo_dir(&self, repo: &str) -> PathBuf {
        self.foundry_dir.join(repo)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct NetworkConfig {
    pub repo: &'static str,
    pub bins: &'static [&'static str],
    pub archive_prefix: &'static str,
    pub default_version: &'static str,
    pub display_name: &'static str,
    pub has_attestation: bool,
}

impl NetworkConfig {
    const FOUNDRY: Self = Self {
        repo: "foundry-rs/foundry",
        bins: &["forge", "cast", "anvil", "chisel"],
        archive_prefix: "foundry",
        default_version: "stable",
        display_name: "foundry",
        has_attestation: true,
    };

    const TEMPO: Self = Self {
        repo: "tempoxyz/tempo-foundry",
        bins: &["forge", "cast"],
        archive_prefix: "foundry",
        default_version: "nightly",
        display_name: "tempo-foundry",
        has_attestation: false,
    };

    fn for_network(network: Option<Network>) -> Self {
        match network {
            Some(Network::Tempo) => Self::TEMPO,
            None => Self::FOUNDRY,
        }
    }
}
