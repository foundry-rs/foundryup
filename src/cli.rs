use clap::{CommandFactory, Parser};

/// The installer for Foundry.
///
/// Update or revert to a specific Foundry version with ease.
///
/// By default, the latest stable version is installed from built binaries.
#[derive(Debug, Parser)]
#[command(name = "foundryup", version = crate::config::LONG_VERSION, about, disable_version_flag = true)]
pub(crate) struct Cli {
    /// Update foundryup to the latest version
    #[arg(short = 'U', long = "update")]
    pub update: bool,

    /// Build and install from a remote GitHub repo (uses default branch if no other options)
    #[arg(short = 'r', long, env = "FOUNDRYUP_REPO", hide_env = true)]
    pub repo: Option<String>,

    /// Build and install a specific branch
    #[arg(short = 'b', long, conflicts_with = "pr", env = "FOUNDRYUP_BRANCH", hide_env = true)]
    pub branch: Option<String>,

    /// Install a specific version from built binaries (e.g., stable, nightly, 0.3.0)
    #[arg(
        id = "ver",
        short = 'i',
        long = "install",
        value_name = "VERSION",
        env = "FOUNDRYUP_VERSION",
        hide_env = true
    )]
    pub version: Option<String>,

    /// List installed versions
    #[arg(short = 'l', long = "list")]
    pub list: bool,

    /// Use a specific installed version
    #[arg(short = 'u', long = "use", value_name = "VERSION")]
    pub use_version: Option<String>,

    /// Build and install a local repository
    #[arg(short = 'p', long, env = "FOUNDRYUP_LOCAL_REPO", hide_env = true)]
    pub path: Option<std::path::PathBuf>,

    /// Build and install a specific Pull Request
    #[arg(short = 'P', long, conflicts_with = "branch", env = "FOUNDRYUP_PR", hide_env = true)]
    pub pr: Option<u64>,

    /// Build and install a specific commit
    #[arg(short = 'C', long, env = "FOUNDRYUP_COMMIT", hide_env = true)]
    pub commit: Option<String>,

    /// Number of CPUs to use for building (default: all)
    #[arg(short = 'j', long, env = "FOUNDRYUP_JOBS", hide_env = true)]
    pub jobs: Option<u32>,

    /// Cargo profile to use for building
    #[arg(long, default_value = "release")]
    pub cargo_profile: String,

    /// Cargo features to enable for building
    #[arg(long)]
    pub cargo_features: Option<String>,

    /// [deprecated] Install binaries for a specific network
    #[arg(short = 'n', long, hide = true, env = "FOUNDRYUP_NETWORK", hide_env = true)]
    pub network: Option<String>,

    /// Skip SHA verification (INSECURE)
    #[arg(short = 'f', long)]
    pub force: bool,

    /// Install a specific architecture (amd64, arm64)
    #[arg(long, env = "FOUNDRYUP_ARCH", hide_env = true)]
    pub arch: Option<String>,

    /// Install a specific platform (win32, linux, darwin, alpine)
    #[arg(long, env = "FOUNDRYUP_PLATFORM", hide_env = true)]
    pub platform: Option<String>,

    /// Generate shell completions
    #[arg(long, value_name = "SHELL")]
    pub completions: Option<clap_complete::Shell>,

    /// Print version
    #[arg(short = 'V', short_alias = 'v', long = "version", action = clap::ArgAction::Version)]
    pub version_flag: Option<bool>,
}

pub(crate) fn print_completions(shell: clap_complete::Shell) {
    clap_complete::generate(shell, &mut Cli::command(), "foundryup", &mut std::io::stdout());
}
